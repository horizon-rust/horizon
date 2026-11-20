#!/usr/bin/env python3
"""Every nested Cargo workspace that reaches a crate outside its own folder still matches its Cargo.lock.

The root workspace's lock is checked by verify's `--locked` builds. A nested workspace (crates/horizon-telemetry-server,
crates/horizon-mod-registry, the wasm test fixtures, launcher/...) builds only in its own workflow or test, so when a
crate it reaches gains or changes a dependency, its lock goes stale and its `--locked` build fails after the merge.
`cargo update --workspace --locked` resolves from the registry index alone (it downloads and compiles no crate) and
fails when the lock would have to change.

    python3 tools/ci/nested_locks.py [--base REV]   # with --base: nothing to do unless a Cargo.toml or Cargo.lock changed
"""

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KINDS = ("dependencies", "dev-dependencies", "build-dependencies")


def path_dependencies(manifest):
    """The folders `manifest` names through `path`, in every dependency table."""
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    tables = [data.get(kind, {}) for kind in KINDS]
    tables += [target.get(kind, {}) for target in data.get("target", {}).values() for kind in KINDS]
    return [(manifest.parent / spec["path"]).resolve() for table in tables for spec in table.values()
            if isinstance(spec, dict) and "path" in spec]


def workspaces(root=ROOT):
    """The nested workspaces (folders with their own tracked Cargo.lock) whose manifest reaches outside the folder."""
    locks = subprocess.run(["git", "ls-files", "*Cargo.lock"], cwd=root, capture_output=True, text=True,
                           check=True).stdout.split()
    found = []
    for lock in locks:
        folder = (root / lock).parent.resolve()
        if folder == root.resolve() or not (folder / "Cargo.toml").exists():
            continue
        if any(not dependency.is_relative_to(folder) for dependency in path_dependencies(folder / "Cargo.toml")):
            found.append(folder.relative_to(root.resolve()).as_posix())
    return sorted(found)


def resolution_changed(paths):
    """Whether a change to `paths` can change what a lock resolves to."""
    return any(Path(path).name in ("Cargo.toml", "Cargo.lock") for path in paths)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="skip unless a Cargo.toml or Cargo.lock changed between this revision and HEAD")
    args = parser.parse_args(argv)
    if args.base:
        changed = subprocess.run(["git", "diff", "--name-only", args.base, "HEAD"], cwd=ROOT, capture_output=True,
                                 text=True, check=True).stdout.split()
        if not resolution_changed(changed):
            print("nested locks: no Cargo.toml or Cargo.lock changed")
            return 0
    stale = []
    for folder in workspaces():
        manifest = f"{folder}/Cargo.toml"
        result = subprocess.run(["cargo", "update", "--workspace", "--locked", "--quiet", "--manifest-path", manifest],
                                cwd=ROOT, capture_output=True, text=True)
        if result.returncode == 0:
            print(f"nested locks: {folder}/Cargo.lock is current")
            continue
        stale.append(folder)
        print(result.stderr.strip(), file=sys.stderr)
        print(f"::error file={folder}/Cargo.lock::{folder}/Cargo.lock no longer matches the crates it reaches. "
              f"Update it with `cargo update --workspace --manifest-path {manifest}` and commit the lock.")
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main())
