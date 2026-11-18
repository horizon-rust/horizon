#!/usr/bin/env python3
"""Check that a change leaves the mod point catalog and the generated mod API as they were (docs/layers.md#mod-points-stay-put).

Every eligible `&self` or `&mut self` method of `Game` is an injection point: its name, module path and signature are
part of the mod API. Moving code out of `impl Game` must keep each such method where it is and only delegate from its
body. This tool builds and runs the `hook_snapshot` example of `horizon-mod-api-gen` on the base and on the working
tree, then compares what it wrote (points.json, the generated API, catalog.json, hook-notes.md). It fails on:
- a point that was added, removed or changed (name, target, signature hash or any other field of points.json);
- a change to any other generated file.
`--allow-added` accepts a change whose points were only added (a new hook on purpose); the generated files then differ
too, and are accepted with them.

    python3 tools/check_mod_points_unchanged.py                      # the working tree against the merge base with origin/main
    python3 tools/check_mod_points_unchanged.py --base HEAD^1        # against another ref
    python3 tools/check_mod_points_unchanged.py --allow-added        # new points are fine, removed or changed ones are not
    python3 tools/check_mod_points_unchanged.py --target-dir DIR     # the Cargo build folder both runs share (default: target)
    python3 tools/check_mod_points_unchanged.py --compare OLD NEW    # compare two snapshot folders, build nothing

The base is built in a temporary `git worktree`, removed afterwards. The first run builds the generator's crates twice
(two source paths); a shared build folder keeps their dependencies.
"""

import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
DOC = "docs/layers.md#mod-points-stay-put"
POINTS = "points.json"
MAX_DIFF_LINES = 120
MAX_LISTED = 40
ROOT_MARK = "<repository>"


def load_points(path):
    """points.json as a dict from point name to its record. A name listed twice is an error: the diff would lose one."""
    with open(path, encoding="utf-8") as f:
        points = json.load(f)
    by_name = {}
    for point in points:
        name = point["name"]
        if name in by_name:
            raise ValueError(f"{path}: point '{name}' is listed twice")
        by_name[name] = point
    return by_name


def diff_points(base, head):
    """(added, removed, changed) between two name-to-record dicts. `changed` holds (name, {field: (old, new)})."""
    added = sorted(set(head) - set(base))
    removed = sorted(set(base) - set(head))
    changed = []
    for name in sorted(set(base) & set(head)):
        fields = {key: (base[name].get(key), head[name].get(key))
                  for key in sorted(set(base[name]) | set(head[name]))
                  if base[name].get(key) != head[name].get(key)}
        if fields:
            changed.append((name, fields))
    return added, removed, changed


def read_text(path, root_marks):
    """A file's text with each source folder's path replaced, so two worktrees' outputs compare."""
    text = path.read_bytes().decode("utf-8", errors="replace")
    for mark in root_marks:
        text = text.replace(str(mark), ROOT_MARK)
    return text


def snapshot_files(folder):
    return {path.relative_to(folder).as_posix() for path in folder.rglob("*") if path.is_file()} - {POINTS}


def diff_files(base_dir, head_dir, base_root=None, head_root=None):
    """(added, removed, diffs) for the generated files other than points.json: file names that exist on one side, and
    for each file on both sides that differs, its unified diff as a list of lines."""
    base_dir, head_dir = Path(base_dir), Path(head_dir)
    base_files, head_files = snapshot_files(base_dir), snapshot_files(head_dir)
    diffs = []
    for name in sorted(base_files & head_files):
        old = read_text(base_dir / name, [base_root] if base_root else [])
        new = read_text(head_dir / name, [head_root] if head_root else [])
        if old != new:
            diffs.append((name, list(difflib.unified_diff(
                old.splitlines(), new.splitlines(), f"base/{name}", f"head/{name}", lineterm="", n=1))))
    return sorted(head_files - base_files), sorted(base_files - head_files), diffs


def short(value):
    text = json.dumps(value, ensure_ascii=False) if not isinstance(value, str) else value
    return text if len(text) <= 80 else text[:77] + "..."


def summary(added, removed, changed, files_added, files_removed, diffs, base_points, head_points,
            max_lines=MAX_DIFF_LINES, max_listed=MAX_LISTED):
    """The report as a list of lines."""
    out = []

    def listing(title, names, show):
        if not names:
            return
        out.append(f"{title} ({len(names)}):")
        for name in names[:max_listed]:
            out.append(f"  {show(name)}")
        if len(names) > max_listed:
            out.append(f"  ... and {len(names) - max_listed} more")

    listing("Points added", added,
            lambda n: f"{n}  target {head_points[n]['target']}  hash {head_points[n]['signature_hash']}")
    listing("Points removed", removed,
            lambda n: f"{n}  target {base_points[n]['target']}  hash {base_points[n]['signature_hash']}")
    if changed:
        out.append(f"Points changed ({len(changed)}):")
        for name, fields in changed[:max_listed]:
            out.append(f"  {name}")
            for key, (old, new) in fields.items():
                out.append(f"    {key}: {short(old)} -> {short(new)}")
        if len(changed) > max_listed:
            out.append(f"  ... and {len(changed) - max_listed} more")
    listing("Generated files added", files_added, str)
    listing("Generated files removed", files_removed, str)
    if diffs:
        out.append(f"Generated files changed ({len(diffs)}): " + ", ".join(name for name, _ in diffs[:max_listed]))
        left = max_lines
        for name, lines in diffs:
            if left <= 0:
                out.append("... more diff lines left out")
                break
            out.extend(lines[:left])
            if len(lines) > left:
                out.append(f"... {len(lines) - left} more lines of {name} left out")
            left -= len(lines)
    return out


def verdict(added, removed, changed, files_added, files_removed, diffs, allow_added=False):
    """0 when the catalog and the API are as they were, or, with allow_added, when points were only added."""
    different = bool(added or removed or changed or files_added or files_removed or diffs)
    if not different:
        return 0
    if allow_added and added and not removed and not changed:
        return 0
    return 1


def compare(base_dir, head_dir, allow_added=False, base_root=None, head_root=None):
    """Compare two snapshot folders, print the report and return the exit status."""
    base_points, head_points = load_points(Path(base_dir) / POINTS), load_points(Path(head_dir) / POINTS)
    added, removed, changed = diff_points(base_points, head_points)
    files_added, files_removed, diffs = diff_files(base_dir, head_dir, base_root, head_root)
    status = verdict(added, removed, changed, files_added, files_removed, diffs, allow_added)
    lines = summary(added, removed, changed, files_added, files_removed, diffs, base_points, head_points)
    if not lines:
        print(f"Mod points unchanged: {len(head_points)} points, the generated API is the same.")
        return 0
    print("\n".join(lines))
    if status == 0:
        print(f"\nMod points: {len(added)} added, none removed or changed (--allow-added).")
    else:
        print(f"\nMod point check failed: the mod API changed ({DOC}).")
        print("A `Game` method keeps its name, module path and signature when its body moves; only the body delegates.")
        print("If the change is on purpose, a new hook only adds points: run again with --allow-added.")
    return status


def git(*args, cwd=ROOT):
    result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {result.stderr.strip() or result.stdout.strip()}")
    return result.stdout.strip()


def default_base():
    try:
        return git("merge-base", "HEAD", "origin/main")
    except RuntimeError as error:
        raise RuntimeError(f"{error}\nNo origin/main to compare with: pass --base REF.")


LOCK_OUT_OF_DATE = "cannot update the lock file"
TAIL_LINES = 30


def snapshot(root, out, target_dir, side):
    """Run the hook_snapshot example of the tree at `root`, writing into `out`. Cargo's and the example's output is
    captured: it prints nothing on success, and the end of stderr on failure."""
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir))
    command = ["cargo", "run", "--locked", "--quiet", "-p", "horizon-mod-api-gen", "--example", "hook_snapshot",
               "--", str(out), str(root)]
    print(f"Snapshot of the {side} ({root})", flush=True)
    result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True, errors="replace")
    if result.returncode == 0:
        return
    stderr = result.stderr.strip()
    if LOCK_OUT_OF_DATE in stderr:
        raise RuntimeError(f"the {side}'s Cargo.lock is out of date: run cargo once without --locked, then rerun")
    tail = "\n".join(stderr.splitlines()[-TAIL_LINES:])
    raise RuntimeError(f"hook_snapshot failed on the {side} (exit {result.returncode}): {' '.join(command)}\n{tail}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="the git ref to compare with (default: the merge base of HEAD and origin/main)")
    parser.add_argument("--allow-added", action="store_true", help="accept a change that only adds points")
    parser.add_argument("--target-dir", type=Path,
                        help="Cargo build folder for both runs (default: $CARGO_TARGET_DIR, else the repository's target)")
    parser.add_argument("--compare", nargs=2, metavar=("OLD", "NEW"), type=Path,
                        help="compare two snapshot folders instead of building")
    args = parser.parse_args(argv)
    if args.compare:
        try:
            return compare(args.compare[0], args.compare[1], args.allow_added)
        except (OSError, ValueError, KeyError) as error:
            print(f"Mod point check failed: {error}")
            return 2
    target = args.target_dir or Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    scratch = Path(tempfile.mkdtemp(prefix="mod-points-"))
    worktree = scratch / "base"
    added_worktree = False
    try:
        base = git("rev-parse", args.base) if args.base else default_base()
        print(f"Base {base[:12]}, build folder {target}")
        git("worktree", "add", "--detach", "--quiet", str(worktree), base)
        added_worktree = True
        snapshot(worktree, scratch / "base-out", target, "base")
        snapshot(ROOT, scratch / "head-out", target, "working tree")
        return compare(scratch / "base-out", scratch / "head-out", args.allow_added, worktree, ROOT)
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        print(f"Mod point check failed: {error}")
        return 2
    finally:
        if added_worktree:
            subprocess.run(["git", "worktree", "remove", "--force", str(worktree)], cwd=ROOT, capture_output=True)
        subprocess.run(["git", "worktree", "prune"], cwd=ROOT, capture_output=True)
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
