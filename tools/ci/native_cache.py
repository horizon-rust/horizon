#!/usr/bin/env python3
"""Builds the Nightly's native-mod cache: the compiled crates of the launcher's own native build, packed.

The launcher compiles a native-mod build with its own pinned Rust (launcher/src/rust_toolchain.rs), from a clean
source folder, with `cargo build --locked --profile fast` (launcher/src/native_mods.rs, `compile` and `cargo`).
Starting from the crates this script compiles the same way, a player's launcher compiles only what the mods change.
This script repeats that build for one platform, with no mods, and writes
`native-cache-<triple>-fast.tar.zst` (layout: docs/native-mods.md, "Nightly cache"):

    fast/{deps,build,.fingerprint}/...   (no incremental/, no final programs)
    native-cache.json

Run it from a checkout of the commit; the Nightly signs the archive with the game files (nightly-build.yml).
    python3 tools/ci/native_cache.py --commit <sha> --triple x86_64-unknown-linux-gnu --out dist

The build never sees HORIZON_VERSION or HORIZON_NIGHTLY (cargo tracks option_env! values, so a build with them
would not match a player's, which has neither), the runner's cargo settings, or its rustup.
"""
import argparse
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TRIPLES = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "x86_64-pc-windows-gnu")
FEATURES = "horizon-client/mods"  # launcher/src/native_mods.rs FEATURES
OFFICIAL = "horizon-official"     # launcher/src/native_mods/api.rs OFFICIAL
KEEP = ("deps", "build", ".fingerprint")
MAX_BYTES = 1900 * 1024 * 1024    # a release asset is at most 2 GiB
# launcher/src/rust_toolchain.rs SCRUBBED and scrubbed(): the settings that would make another compiler build.
SCRUBBED = {"RUSTUP_TOOLCHAIN", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
            "RUSTDOCFLAGS", "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_INCREMENTAL"}
SCRUBBED_PREFIXES = ("CARGO_PROFILE_", "CARGO_BUILD_", "CARGO_TARGET_")
# What the game never ships in the cache: the game's own program (the player's build links its own).
GAME_PROGRAM = re.compile(r"^(horizon_official|horizon_client)-[0-9a-f]{16}(\.exe|\.pdb)?$")


def scrubbed(name: str) -> bool:
    name = name.upper()
    if name == "CARGO_BUILD_JOBS":
        return False
    return name in SCRUBBED or name.startswith(SCRUBBED_PREFIXES)


def run(cmd, **kwargs) -> str:
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run(cmd, check=True, text=True, **kwargs).stdout


def launcher_const(pattern: str) -> str:
    text = (ROOT / "launcher/src/rust_toolchain.rs").read_text(encoding="utf-8")
    match = re.search(pattern, text, re.S)
    if not match:
        sys.exit(f"launcher/src/rust_toolchain.rs: no match for {pattern!r}")
    return match.group(1)


def has_mods_feature(stage: Path) -> bool:
    """launcher/src/native_mods.rs has_mods_feature: horizon-client's Cargo.toml has `mods` under [features]."""
    text = (stage / "crates/horizon-client/Cargo.toml").read_text(encoding="utf-8")
    in_features = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_features = stripped == "[features]"
        elif in_features and re.match(r"mods\s*=", stripped):
            return True
    return False


def mingw(into: Path) -> Path:
    """The launcher's MinGW-w64 (WinLibs, pinned and checksummed): the C compiler its Windows builds use."""
    url = launcher_const(r"pub const MINGW: CToolchain = CToolchain \{.*?url: \"([^\"]+)\"")
    digest = launcher_const(r"pub const MINGW: CToolchain = CToolchain \{.*?sha256: \"([0-9a-f]{64})\"")
    print(f"MinGW-w64 {url}", flush=True)
    data = urllib.request.urlopen(url, timeout=600).read()
    if hashlib.sha256(data).hexdigest() != digest:
        sys.exit("the MinGW-w64 archive is not the one the launcher pins")
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        z.extractall(into)
    return into / "mingw64" / "bin"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--commit", required=True, help="the Nightly's commit (40 hex)")
    ap.add_argument("--triple", required=True, choices=TRIPLES)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.commit):
        sys.exit("--commit must be the full 40-character commit")
    started = time.time()
    work = Path(tempfile.mkdtemp(prefix="native-cache-"))
    version = launcher_const(r'pub const VERSION: &str = "([^"]+)"')

    # The source as the launcher has it: the commit's own files, no .git (GitHub's archive of the commit).
    stage = work / "stage"
    stage.mkdir()
    source_tar = work / "source.tar"
    run(["git", "archive", "--format=tar", "-o", str(source_tar), args.commit], cwd=ROOT)
    with tarfile.open(source_tar) as t:
        t.extractall(stage, filter="data")
    source_tar.unlink()

    # The launcher's Rust: the pinned release for this computer's triple, used by its own paths, not rustup's proxies.
    toolchain = f"{version}-{args.triple}"
    run(["rustup", "toolchain", "install", toolchain, "--profile", "minimal", "--no-self-update"])
    rustc = Path(run(["rustup", "which", "rustc", "--toolchain", toolchain], stdout=subprocess.PIPE).strip())
    bin_dir = rustc.parent
    cargo = bin_dir / rustc.name.replace("rustc", "cargo")

    env = {k: v for k, v in os.environ.items() if not scrubbed(k)}
    for name in ("HORIZON_VERSION", "HORIZON_NIGHTLY"):
        env.pop(name, None)
    extra = [str(bin_dir)]
    if args.triple == "x86_64-pc-windows-gnu":
        extra.append(str(mingw(work / "mingw")))
    env["PATH"] = os.pathsep.join(extra + [env.get("PATH", "")])
    env["RUSTC"] = str(rustc)
    env["CARGO_HOME"] = str(work / "cargo-home")
    target_dir = work / "target"
    env["CARGO_TARGET_DIR"] = str(target_dir)

    rustc_v = run([str(rustc), "-vV"], env=env, stdout=subprocess.PIPE)
    cargo_v = run([str(cargo), "-V"], env=env, stdout=subprocess.PIPE).strip()
    host = re.search(r"^host: (.+)$", rustc_v, re.M).group(1).strip()
    if host != args.triple:
        sys.exit(f"the toolchain's host is {host}, not {args.triple}")

    package = OFFICIAL if (stage / "crates/horizon-official/Cargo.toml").is_file() else "horizon-client"
    features = FEATURES if has_mods_feature(stage) else ""
    cmd = [str(cargo), "build"]
    config = stage / ".cargo/config.toml"
    if config.is_file():
        cmd += ["--config", str(config)]
    cmd += ["--manifest-path", str(stage / "Cargo.toml"), "--locked", "--profile", "fast"]
    if features:
        cmd += ["--features", features]
    cmd += ["-p", package, "--bin", package if package == OFFICIAL else "horizon-client"]
    # From the filesystem root, as the launcher does (rust_toolchain::outside_configs): no `.cargo/config.toml` above
    # the build is read, only the workspace's own.
    run(cmd, env=env, cwd=Path(work.anchor))

    fast = target_dir / "fast"
    lock = hashlib.sha256((stage / "Cargo.lock").read_bytes()).hexdigest()
    info = {"format": 1, "commit": args.commit, "triple": args.triple, "profile": "fast", "rustc": rustc_v,
            "cargo": cargo_v, "package": package, "features": features, "cargo_lock_sha256": lock}

    args.out.mkdir(parents=True, exist_ok=True)
    name = f"native-cache-{args.triple}-fast.tar.zst"
    tar_path = work / "cache.tar"
    with tarfile.open(tar_path, "w", format=tarfile.PAX_FORMAT) as t:
        raw = (json.dumps(info, indent=2) + "\n").encode()
        member = tarfile.TarInfo("native-cache.json")
        member.size, member.mtime, member.mode = len(raw), int(time.time()), 0o644
        t.addfile(member, io.BytesIO(raw))
        for folder in KEEP:
            top = fast / folder
            if not top.is_dir():
                sys.exit(f"{top} is missing: nothing was compiled")
            for path in sorted(top.rglob("*")):
                relative = path.relative_to(fast)
                if folder == "deps" and GAME_PROGRAM.match(path.name):
                    continue
                t.add(path, arcname=f"fast/{relative.as_posix()}", recursive=False)
    run(["zstd", "-T0", "-12", "-q", "-f", "-o", str(args.out / name), str(tar_path)])
    size = (args.out / name).stat().st_size
    shutil.rmtree(work, ignore_errors=True)
    minutes = (time.time() - started) / 60
    print(f"{name}: {size / 1e6:.0f} MB, {minutes:.0f} min", flush=True)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(f"- `{name}`: {size / 1e6:.0f} MB, {minutes:.0f} min, rustc `{cargo_v}`\n")
    if size > MAX_BYTES:
        (args.out / name).unlink()
        sys.exit(f"{name} is {size} bytes: too big for a release asset, not published")


if __name__ == "__main__":
    main()
