#!/usr/bin/env python3
"""Split a parity selection over several runners, and export the dependency lists that let a runner without parity
state of its own select what a change reaches.

  python3 tools/parity_plan.py --plan plan.json -- --changed origin/main   # the shards this selection needs
  python3 tools/parity_plan.py --shard 2/5 -- --all                        # the names in shard 2 of 5, one per line
  python3 tools/parity_plan.py --export-dependencies DIR                   # this checkout's dependency lists

The selection is parity.py's own: the arguments after `--` go to `parity.py --list`, so nothing is built or run.
Each harness joins the group of the probe it builds (game, client, world, other; from its record here or the shared
dependency list in PARITY_SHARED), and a shard holds one group, so a runner pays only that group's cold build.
Runners then run `parity.py <names>`. This file is never imported by parity.py or a harness: a change to it reruns
no harness.
"""
import argparse
import json
import math
import re
import subprocess
import sys
from pathlib import Path

try:
    from . import parity
except ImportError:
    import parity

# Harnesses per shard for about 20 minutes on a cold 16-vCPU hosted runner with no GPU (measured 2026-11-19 on
# blacksmith-16vcpu-ubuntu-2404, 8 harnesses at a time): ~60 s of setup and data, then the group's cold build (world
# ~40 s; client ~530 s for client_probe's fast and dev profiles; game ~480 s for its two builds), then 3-10 s of
# runner time per harness, nearly all of it in the vanilla oracle. other: probes nothing recorded names.
GROUPS = ("game", "client", "world", "other")
SHARD_SIZES = {"game": 50, "client": 100, "world": 150, "other": 50}
GAME_PACKAGES = {"horizon-game", "horizon-client", "horizon-client-lib"}
GAME_BINARIES = {"horizon-client", "horizon-ctl"}


def selection(arguments):
    """The harnesses `parity.py ARGUMENTS --list` would run, in its order. Exits as parity.py does on a bad argument."""
    result = subprocess.run([sys.executable, str(Path(parity.__file__)), *arguments, "--list"], cwd=parity.ROOT,
                            capture_output=True, text=True)
    sys.stderr.write(result.stderr)
    if result.returncode:
        sys.exit(result.returncode)
    found = parity.harnesses()
    return [line for line in result.stdout.splitlines() if line in found]


def artifacts(name, records):
    """The probes a harness built: its record here, else the shared dependency list (as parity.py prebuilds them)."""
    record = records.get(name)
    if not record or "artifacts" not in record:
        template = parity.SHARED / "dependencies" / f"{name}.json"
        record = json.loads(template.read_text(encoding="utf-8")) if template.exists() else {}
    return parity.recorded_artifacts(record)


def probe_group(built):
    """game when it builds the game's executables, else client or world for that crate's probe, else other."""
    def packages(artifact):
        chosen = artifact[3] if len(artifact) > 3 else []
        return {value for flag, value in zip(chosen, chosen[1:]) if flag == "-p"}
    if any(artifact[0] in ("bin", "bins") and (artifact[1] in GAME_BINARIES or packages(artifact) & GAME_PACKAGES)
           for artifact in built):
        return "game"
    names = {artifact[1] for artifact in built}
    every = set().union(*map(packages, built)) if built else set()
    if "client_probe" in names or every & {"horizon-client", "horizon-client-lib"}:
        return "client"
    if "world_probe" in names or "horizon-world" in every:
        return "world"
    return "other"


def shards_needed(names, groups):
    return sum(math.ceil(sum(groups[name] == group for name in names) / SHARD_SIZES[group]) for group in GROUPS)


def plan(names, groups, count):
    """count shards of names: [(group, [names])], each group's shards together, in GROUPS order.

    Every group with harnesses gets a shard of its own while there are enough; the rest go one at a time to the group
    whose shards are fullest for their size. A group's harnesses are dealt in turn, so the entries of a sharded
    harness (world_spawn#1...) spread over its shards. With fewer shards than groups, whole groups share shards,
    the largest first into the emptiest."""
    if count < 1:
        raise ValueError("a plan needs at least one shard")
    members = {group: [name for name in names if groups[name] == group] for group in GROUPS}
    present = [group for group in GROUPS if members[group]]
    if not present:
        return [("", [])] * count
    if count < len(present):
        shards = [([], []) for _ in range(count)]
        for group in sorted(present, key=lambda group: (-len(members[group]), GROUPS.index(group))):
            labels, harnesses = min(shards, key=lambda shard: len(shard[1]))
            labels.append(group)
            harnesses += members[group]
        return [("+".join(labels), [name for name in names if name in set(harnesses)]) for labels, harnesses in shards]
    counts = {group: 1 for group in present}
    for _ in range(count - len(present)):
        fullest = max(present, key=lambda group: (len(members[group]) / (counts[group] * SHARD_SIZES[group]),
                                                  -GROUPS.index(group)))
        counts[fullest] += 1
    return [(group, members[group][index::counts[group]]) for group in present for index in range(counts[group])]


def export_dependencies(records, found, directory):
    """Write NAME.json ({"dependencies", "artifacts"}, as parity.py's shared lists) for each harness this checkout
    itself last passed: never a pass taken from the shared folder (another checkout's, possibly a pull request's),
    a failed run (its trace may stop early) or a record with no dependencies (a runner would then never select it).
    Returns how many."""
    directory.mkdir(parents=True, exist_ok=True)
    count = 0
    for name in sorted(found):
        record = records.get(name) or {}
        if record.get("status") != "pass" or record.get("shared") or not record.get("dependencies") \
                or "artifacts" not in record:
            continue
        data = {"dependencies": record["dependencies"], "artifacts": record["artifacts"]}
        (directory / f"{name}.json").write_text(json.dumps(data), encoding="utf-8")
        count += 1
    return count


def shard_spec(text):
    match = re.fullmatch(r"(\d+)/(\d+)", text)
    if not match or not 1 <= int(match[1]) <= int(match[2]):
        raise argparse.ArgumentTypeError(f"--shard takes I/N with 1 <= I <= N, not {text!r}")
    return int(match[1]), int(match[2])


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    ours, rest = (argv[:argv.index("--")], argv[argv.index("--") + 1:]) if "--" in argv else (argv, [])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--plan", metavar="FILE", help="write the shards the selection needs as JSON, and list them")
    action.add_argument("--shard", type=shard_spec, metavar="I/N", help="print the names in shard I of N")
    action.add_argument("--export-dependencies", metavar="DIR", type=Path,
                        help="write this checkout's dependency lists into DIR (no selection)")
    args = parser.parse_args(ours)
    records = parity.load_state()["harnesses"]
    if args.export_dependencies:
        count = export_dependencies(records, parity.harnesses(), args.export_dependencies)
        print(f"{count} dependency lists written to {args.export_dependencies}")
        return 0
    names = selection(rest)
    groups = {name: probe_group(artifacts(name, records)) for name in names}
    if args.shard:
        index, count = args.shard
        for name in plan(names, groups, count)[index - 1][1]:
            print(name)
        return 0
    shards = plan(names, groups, shards_needed(names, groups)) if names else []
    Path(args.plan).write_text(json.dumps({"shards": [{"group": group, "harnesses": harnesses}
                                                      for group, harnesses in shards]}, indent=1) + "\n",
                               encoding="utf-8")
    for index, (group, harnesses) in enumerate(shards, 1):
        print(f"shard {index}/{len(shards)} {group}: {len(harnesses)}: {' '.join(harnesses)}")
    print(f"{len(names)} harnesses in {len(shards)} shards: "
          + ", ".join(f"{sum(groups[name] == group for name in names)} {group}" for group in GROUPS))
    return 0


if __name__ == "__main__":
    sys.exit(main())
