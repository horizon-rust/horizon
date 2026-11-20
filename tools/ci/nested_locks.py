#!/usr/bin/env python3
"""Every nested Cargo workspace still matches its Cargo.lock.

The root workspace's lock is checked by verify's `--locked` builds. A nested workspace (crates/horizon-telemetry-server,
crates/horizon-mod-registry, the wasm test fixtures, launcher/...) builds only in its own workflow or test, so when a
crate it reaches gains or changes a dependency, its lock goes stale and its `--locked` build fails after the merge.
`cargo update --workspace --locked`, run inside each workspace, resolves from the registry index alone (it downloads
and compiles no crate) and fails with cargo's own message when the lock would have to change.

Only that message is a finding. Any other failure (the registry unreachable, a yanked version, a git source) is retried
twice, then reported as a warning and passes: the check could not resolve, which says nothing about the lock, and the
nested workspace's own `--locked` build after the merge still catches a stale lock, as it did before this check.

    python3 tools/ci/nested_locks.py [--base REV]   # with --base: nothing to do unless a resolution input changed
"""

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Locks this job cannot resolve, with the reason. Every other tracked nested Cargo.lock with a Cargo.toml beside it is
# checked: one that cannot go stale costs about a second.
EXCLUDED = {
    # Its horizon-mod-api dependency points into .horizon/sdk/, which tools/ci/fn_replace_check.sh generates first.
    "tools/ci/fn-replace-check/registry/Cargo.lock": "needs the SDK fn_replace_check.sh generates",
}

# cargo's message when --locked stops a lock change (src/cargo/ops/lockfile.rs). Nothing else is a stale lock.
STALE = re.compile(r"cannot update the lock file .+Cargo\.lock because --locked was passed to prevent this")
ATTEMPTS = 3
PAUSE_SECONDS = 10


def git_paths(command, *arguments, root=ROOT):
    """The paths `git <command> -z <arguments>` prints, NUL-delimited so any file name survives."""
    output = subprocess.run(["git", command, "-z", *arguments], cwd=root, capture_output=True, check=True).stdout
    return [path for path in output.decode("utf-8").split("\0") if path]


def resolution_input(path):
    """Whether a change to `path` can change what a Cargo.lock resolves to."""
    parts = path.split("/")
    name = parts[-1]
    return (name in ("Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml")
            or (len(parts) >= 2 and parts[-2] == ".cargo" and name in ("config", "config.toml"))
            or path == "tools/ci/rust-version")


def needs_check(base, root=ROOT):
    """Whether a resolution input changed between `base` and HEAD; True when that can't be told."""
    if not base or re.fullmatch(r"0+", base):
        return True
    try:
        changed = git_paths("diff", "--no-renames", "--name-only", base, "HEAD", root=root)
    except subprocess.CalledProcessError:
        return True
    return any(resolution_input(path) for path in changed)


def workspaces(root=ROOT):
    """The folders of every tracked nested Cargo.lock with a Cargo.toml beside it, except EXCLUDED."""
    found = []
    for lock in git_paths("ls-files", "--", "*Cargo.lock", root=root):
        folder = Path(lock).parent
        if folder == Path(".") or lock in EXCLUDED or not (root / folder / "Cargo.toml").exists():
            continue
        found.append(folder.as_posix())
    return sorted(found)


def check(folder, root=ROOT, run=subprocess.run, sleep=time.sleep):
    """'current', 'stale' or 'unresolved' for the workspace in `folder`, printing what CI should show."""
    lock = f"{folder}/Cargo.lock"
    for attempt in range(1, ATTEMPTS + 1):
        # From inside the workspace, so a .cargo/config.toml there applies as it does to its own builds.
        result = run(["cargo", "update", "--workspace", "--locked", "--quiet"], cwd=root / folder,
                     capture_output=True, text=True)
        if result.returncode == 0:
            print(f"nested locks: {lock} is current")
            return "current"
        if STALE.search(result.stderr):
            print(result.stderr.strip(), file=sys.stderr)
            print(f"::error file={lock}::{lock} no longer matches the crates it reaches. Update it with "
                  f"`cd {folder} && cargo update --workspace` and commit the lock.")
            return "stale"
        if attempt < ATTEMPTS:
            sleep(PAUSE_SECONDS)
    print(f"nested locks: could not resolve {lock}; this is NOT a staleness finding. cargo said:")
    print(result.stderr.strip())
    print(f"::warning file={lock}::could not resolve {lock} (not a stale lock); its own --locked build still checks it")
    return "unresolved"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="skip unless a resolution input changed between this revision and HEAD")
    args = parser.parse_args(argv)
    if not needs_check(args.base):
        print("nested locks: no Cargo.toml, Cargo.lock, .cargo config or toolchain file changed")
        return 0
    results = [check(folder) for folder in workspaces()]
    return 1 if "stale" in results else 0


if __name__ == "__main__":
    sys.exit(main())
