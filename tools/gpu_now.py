#!/usr/bin/env python3
"""Where a running game's GPU frame goes, now, without relaunching it.

  python3 tools/gpu_now.py --socket PATH [--seconds 3] [--keep] [--json] [--split] [--ctl horizon-ctl]

Turns the game's stage timing on through its control socket (`horizon-ctl gpu-passes on`, the
same stages as HORIZON_GPU_PASSES=1), waits for a window of frames, reads each stage's median and
95th percentile back and prints, per view (desktop, eye1, eye2...), the stages from the costliest
down with their share of the frame, then the same summed by family (the name before the colon:
rays, world, froxels...). The timing goes off again unless --keep; it costs a few timestamps a
frame while on. Run it while the player looks at what is slow.

RT lighting's rays are one kernel (`rays: trace`): --split times it again with its parts left out
(`horizon-ctl rt-skip`: the sun's ray, ReSTIR GI, ReSTIR DI, the PSR), each alone and each
missing, and gives what each adds to the bare kernel and what its absence saves from the full one
(parts share the kernel's registers, so these differ), then traces everything again. The picture
is wrong for the half a minute it takes.
"""

import argparse
import json
import shutil
import subprocess
import sys
import time
from collections import defaultdict


def ctl(binary: str, socket: str, *args: str) -> dict:
    out = subprocess.run([binary, "--socket", socket, *args], capture_output=True, text=True,
                         timeout=30)
    try:
        reply = json.loads(out.stdout)
    except json.JSONDecodeError:
        sys.exit(f"horizon-ctl said: {out.stdout.strip() or out.stderr.strip()}")
    if "error" in reply:
        sys.exit(f"horizon-ctl: {reply['error']}")
    return reply


def family(stage: str) -> str:
    return stage.split(":", 1)[0].strip() if ":" in stage else stage


def table(view: dict) -> str:
    frame = view["frame_median_ms"] or 1e-9
    lines = [f"[{view['view']}] frame median {view['frame_median_ms']:.2f} ms, "
             f"p95 {view['frame_p95_ms']:.2f} ms ({view['frames']} frames)"]
    stages = sorted(view["stages"], key=lambda s: -s["median_ms"])
    for s in stages:
        if s["median_ms"] < 0.005:
            continue
        lines.append(f"  {s['median_ms']:7.2f} ms {100 * s['median_ms'] / frame:5.1f} %"
                     f"   p95 {s['p95_ms']:6.2f}   {s['stage']}")
    sums = defaultdict(float)
    for s in view["stages"]:
        sums[family(s["stage"])] += s["median_ms"]
    lines.append("  by family:")
    for name, ms in sorted(sums.items(), key=lambda kv: -kv[1]):
        if ms >= 0.005:
            lines.append(f"  {ms:7.2f} ms {100 * ms / frame:5.1f} %   {name}")
    return "\n".join(lines)


TRACE = "rays: trace (sun, DI, GI, PSR)"
PARTS = [(1, "sun's ray (shadows)"), (2, "ReSTIR GI (sky, bounces)"), (4, "ReSTIR DI (light sources)"),
         (8, "PSR (water, glass, mirrors)")]


def timed(a, skip: int) -> dict:
    """The views' times with `skip` left out of the kernel, over a fresh window."""
    ctl(a.ctl, a.socket, "gpu-passes", "off")
    ctl(a.ctl, a.socket, "rt-skip", str(skip))
    ctl(a.ctl, a.socket, "gpu-passes", "on")
    time.sleep(a.seconds)
    return ctl(a.ctl, a.socket, "gpu-passes")["gpu"]


def trace_ms(gpu: dict, view: str) -> float:
    for v in gpu["views"]:
        if v["view"] == view:
            return next((s["median_ms"] for s in v["stages"] if s["stage"] == TRACE), 0.0)
    return 0.0


def split(a) -> None:
    try:
        full = timed(a, 0)
        bare = timed(a, 15)
        without = [(name, timed(a, mask)) for mask, name in PARTS]
        alone = [timed(a, 15 & ~mask) for mask, _ in PARTS]
    finally:
        ctl(a.ctl, a.socket, "rt-skip", "0")
        if not a.keep:
            ctl(a.ctl, a.socket, "gpu-passes", "off")
    print("\n\n".join(table(v) for v in full["views"]))
    for v in full["views"]:
        view = v["view"]
        total = trace_ms(full, view)
        base = trace_ms(bare, view)
        print(f"\n[{view}] {TRACE}: {total:.2f} ms; with every part out {base:.2f} ms (the depth,"
              f" the ambient rays, the radiance cache).")
        print("  Parts share the kernel's registers: what one costs lies between what it adds alone"
              " (to the bare kernel) and what its absence saves (from the full one).")
        print("      alone   saved   part")
        for (name, gpu), only in zip(without, alone):
            saved = total - trace_ms(gpu, view)
            added = trace_ms(only, view) - base
            print(f"  {added:7.2f} {saved:7.2f}   {name}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--socket", required=True, help="the game's control socket")
    parser.add_argument("--seconds", type=float, default=3.0, help="how long to time")
    parser.add_argument("--keep", action="store_true", help="leave the timing on")
    parser.add_argument("--json", action="store_true", help="print the raw reply")
    parser.add_argument("--split", action="store_true", help="the rays' kernel by part")
    parser.add_argument("--ctl", default=shutil.which("horizon-ctl") or "horizon-ctl")
    a = parser.parse_args()
    if a.split:
        split(a)
        return
    ctl(a.ctl, a.socket, "gpu-passes", "on")
    time.sleep(a.seconds)
    reply = ctl(a.ctl, a.socket, "gpu-passes")
    if not a.keep:
        ctl(a.ctl, a.socket, "gpu-passes", "off")
    gpu = reply["gpu"]
    if a.json:
        print(json.dumps(gpu, indent=2))
        return
    if not gpu["views"]:
        sys.exit("no frame timed yet: wait longer (--seconds)")
    print("\n\n".join(table(v) for v in gpu["views"]))


if __name__ == "__main__":
    main()
