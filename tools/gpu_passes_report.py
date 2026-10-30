#!/usr/bin/env python3
"""Summarise the per-stage GPU times a game log holds when it ran with HORIZON_GPU_PASSES=1.

  HORIZON_GPU_PASSES=1 horizon-client ...            (logs a line per view every 120 frames)
  python3 tools/gpu_passes_report.py game.log [--skip 3] [--json] [--by-look]

Each report line carries the median of one view's last 120 frames, and in VR the head's pose
(`head YAW PITCH`) they were taken at: a change of pose starts the window again. The summary
takes, for every stage of every view (`desktop`, `eye1`, `eye2`, `vr panel`, `vr mirror`), the
median and the spread (min and max) of those medians over the reports after the first --skip
ones (the world is still loading and the caches still filling in those), and the frame's
median and 95th percentile the same way. With --by-look the last report of each pose is
listed on its own (the poses the run turned through), then the median over the poses.
"""

import argparse
import json
import re
import statistics
import sys
from collections import defaultdict

LINE = re.compile(r"GPU stages \[(?P<view>[^\]]+)\] (?P<note>.*?) ?\(median ms of the last \d+ frames\): "
                  r"(?P<stages>.*) \| frame median (?P<median>[\d.]+), p95 (?P<p95>[\d.]+)")


def parse(path: str):
    """Every report, in order: (view, note, stage order, stage -> ms, frame median, frame p95)."""
    reports = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = LINE.search(line)
            if not m:
                continue
            order, stages = [], {}
            for part in m["stages"].split("; "):
                name, _, ms = part.rpartition(" ")
                stages[name] = float(ms)
                order.append(name)
            reports.append((m["view"], m["note"], order, stages, float(m["median"]), float(m["p95"])))
    return reports


def summarise(reports, skip: int, by_look: bool) -> dict:
    views = defaultdict(list)
    for r in reports:
        views[r[0]].append(r)
    summary = {}
    for view, rs in views.items():
        if by_look:
            # The last report of each look: the window there holds only frames of that look.
            last = {}
            for r in rs:
                last[r[1]] = r
            kept = list(last.values())
        else:
            kept = rs[skip:-1] if len(rs) > skip + 1 else rs[skip:]
        if not kept:
            continue
        order = max(kept, key=lambda r: len(r[2]))[2]
        stages = {}
        for name in order:
            values = [r[3][name] for r in kept if name in r[3]]
            stages[name] = (statistics.median(values), min(values), max(values))
        summary[view] = {
            "reports": len(kept),
            "looks": {r[1]: {"frame_median": r[4], "frame_p95": r[5], "stages": r[3]} for r in kept},
            "stages": stages,
            "frame_median": statistics.median(r[4] for r in kept),
            "frame_p95": statistics.median(r[5] for r in kept),
        }
    return summary


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("log")
    ap.add_argument("--skip", type=int, default=3, help="reports to ignore at the start")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--by-look", action="store_true", help="one report a head pose, not every report")
    args = ap.parse_args()
    summary = summarise(parse(args.log), args.skip, args.by_look)
    if not summary:
        print("no GPU stage lines: run the game with HORIZON_GPU_PASSES=1", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(summary, indent=2))
        return 0
    for view, s in summary.items():
        print(f"[{view}] {s['reports']} reports: frame median {s['frame_median']:.3f} ms, "
              f"p95 {s['frame_p95']:.3f} ms")
        if args.by_look:
            for note, look in s["looks"].items():
                print(f"  look {note or '-':>12}: {look['frame_median']:.3f} ms (p95 {look['frame_p95']:.3f})")
        for name, (median, low, high) in sorted(s["stages"].items(), key=lambda kv: -kv[1][0]):
            print(f"  {median:8.3f}  ({low:.3f}..{high:.3f})  {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
