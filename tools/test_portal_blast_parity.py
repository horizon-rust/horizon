#!/usr/bin/env python3
"""Check vanilla's portal calculator and Horizon's headless TNT/cube blast regressions.

The Java side calls the actual pinned calculator without creating a server. Rust
runs the production teleport, persistence, mob-event and explosion paths. This is
scoped contract coverage, not a full cross-dimension vanilla server transcript.
"""
import os
import json
from pathlib import Path
import subprocess
import tempfile
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

RUST_SOURCES = ["crates/horizon-client/src/game.rs", "crates/horizon-client/src/game/tnt.rs", "crates/horizon-client/src/game/explosion.rs",
                     "crates/horizon-client/src/game/nether_portal.rs", "crates/horizon-client/src/game/persist.rs",
                     "crates/horizon-client/src/game/creatures.rs", "crates/horizon-client/src/game/commands.rs",
                     "crates/horizon-client/src/mobs/sulfur_cube.rs",
                     "crates/horizon-client/src/game/remote_projectiles.rs", "crates/horizon-client/src/game/wind.rs"]


def child_env(scratch, resources=None):
    """Fresh data roots; the test process's cwd is its isolated game directory."""
    env = dict(os.environ)
    env.pop("HORIZON_TEST_RESOURCES", None)
    for key, name in [("HORIZON_HOME", "home"), ("HORIZON_LAUNCHER_HOME", "launcher"),
                      ("XDG_DATA_HOME", "xdg-data"), ("XDG_CONFIG_HOME", "xdg-config"),
                      ("HORIZON_LAUNCHER_TRASH", "trash"),
                      ("HORIZON_LAUNCHER_CLIPBOARD", "clipboard"),
                      ("HORIZON_LAUNCHER_DESKTOP", "desktop")]:
        env[key] = str(scratch / name)
    env = lab.game_env(env)
    if resources is not None:
        env["HORIZON_TEST_RESOURCES"] = str(resources)
    return env


def main():
    lock, jars, libraries = base.verify_artifacts()
    resources = Path(os.environ.get("HORIZON_TEST_RESOURCES", lab.RESOURCES)).resolve()
    if not (resources / "movement-properties.nbt").exists():
        raise RuntimeError("Set HORIZON_TEST_RESOURCES to the prepared pinned resources")
    build = lab.LOCAL / "portal-blast-parity"
    build.mkdir(parents=True, exist_ok=True)
    report = build / "report.json"
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPortalBlastOracle.java"
    sources = [Path(__file__), oracle, lab.ROOT / "tools/lab.py", lab.ROOT / "tools/test_rust_parity.py",
               lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock", lab.ROOT / "crates/horizon-client/Cargo.toml",
               lab.ROOT / "crates/horizon-client/src/lib.rs",
               *[lab.ROOT / path for path in RUST_SOURCES]]
    hashes = base.source_hashes(sources)
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars["server"], *libraries]))
    subprocess.run(["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none",
                    "-cp", dependencies, "-d", str(classes), str(oracle)], cwd=lab.ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="horizon-portal-oracle-") as directory:
        scratch = Path(directory)
        with (build / "vanilla.log").open("w", encoding="utf-8") as log:
            subprocess.run(["java", "-Xmx512M", "-Djava.awt.headless=true", "-cp",
                            str(classes) + os.pathsep + dependencies,
                            "dev.horizonlab.parity.VanillaPortalBlastOracle"],
                           cwd=scratch, env=child_env(scratch), check=True,
                           stdout=log, stderr=subprocess.STDOUT)
    if (build / "vanilla.log").read_text(encoding="utf-8").count("Block{minecraft:") != 6:
        raise AssertionError("Expected six actual vanilla calculator queries")
    with tempfile.TemporaryDirectory(prefix="horizon-portal-compile-") as directory:
        scratch = Path(directory)
        with (build / "compile.jsonl").open("w", encoding="utf-8") as output, \
                (build / "compile.log").open("w", encoding="utf-8") as errors:
            subprocess.run(["cargo", "test", "--locked", "--manifest-path", str(lab.ROOT / "Cargo.toml"),
                            "-p", "horizon-client", "--lib", "--no-run", "--message-format=json"],
                           cwd=lab.ROOT, env=child_env(scratch), check=True,
                           stdout=output, stderr=errors)
    artifacts = []
    for line in (build / "compile.jsonl").read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get("reason") == "compiler-artifact" and row.get("executable") \
                and row.get("target", {}).get("name") == "horizon_client" \
                and row.get("profile", {}).get("test"):
            artifacts.append(Path(row["executable"]))
    if len(artifacts) != 1:
        raise AssertionError("Expected one actual gameplay library test executable")
    binary = artifacts[0]
    binary_hash = lab.digest(binary, "sha256")
    with tempfile.TemporaryDirectory(prefix="horizon-portal-game-dir-") as directory:
        scratch = Path(directory)
        with (build / "rust.log").open("w", encoding="utf-8") as log:
            subprocess.run([str(binary), "portal_blast_", "--include-ignored", "--color", "never"],
                           cwd=scratch, env=child_env(scratch, resources),
                           check=True, stdout=log, stderr=subprocess.STDOUT)
    result = (build / "rust.log").read_text(encoding="utf-8")
    if "running 4 tests" not in result or "4 passed; 0 failed" not in result:
        raise AssertionError("Expected all four portal-blast regressions to execute")
    if binary_hash != lab.digest(binary, "sha256"):
        raise RuntimeError("Test executable changed during validation")
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, {"minecraft": lab.VERSION, "sources_sha256": hashes,
                           "server_jar_sha256": lab.digest(jars["server"], "sha256"),
                           "oracle_class_sha256": lab.digest(classes / "dev/horizonlab/parity/VanillaPortalBlastOracle.class", "sha256"),
                           "rust_binary_sha256": binary_hash,
                           "vanilla_log_sha256": lab.digest(build / "vanilla.log", "sha256"),
                           "rust_log_sha256": lab.digest(build / "rust.log", "sha256"),
                           "vanilla_calculator_calls": 6, "rust_tests": 4,
                           "scope": __doc__})
    print("Portal blast: 6 vanilla calculator queries and 4 production regressions passed")


if __name__ == "__main__":
    main()
