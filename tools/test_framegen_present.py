#!/usr/bin/env python3
"""Frame generation must not freeze the macOS present thread.

The present thread used to call winit's pre_present_notify. On macOS that is a
dispatch_sync onto the main queue, and the main thread is waiting on this
thread, so the window stops and the pointer stays grabbed.

The default runs the library test. It does not open a window or take the pointer.
`--show` opens a visible game for a person to watch, and it does take the pointer.
Agents do not pass `--show` unless the person asked to look.
"""

import argparse
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import lab

ROOT = Path(__file__).resolve().parents[1]
GRAPHICS = """\
hdr:false
taa:true
fxaa:false
upscaling:performance
upscalingSharpness:85
frameGeneration:fsr3.1
lodDistance:0
dynamicResolution:false
resolutionScale:100
"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--show",
        action="store_true",
        help="open a visible window and grab the pointer, for a person watching",
    )
    parser.add_argument(
        "--bin",
        type=Path,
        default=ROOT / "target/fast/horizon-client",
        help="horizon-client to launch with --show",
    )
    parser.add_argument("--world-frames", type=int, default=30)
    parser.add_argument("--seconds", type=float, default=45, help="watchdog, then kill")
    args = parser.parse_args()
    if not args.show:
        return _library_test()
    if sys.platform != "darwin":
        print("skip: the visible hang check is macOS-only")
        return 0
    print("show mode: a Horizon window will open and grab the pointer", flush=True)
    binary = args.bin if args.bin.is_absolute() else ROOT / args.bin
    if not binary.is_file():
        print(f"missing binary: {binary}", file=sys.stderr)
        return 2
    resources = ROOT / ".local/horizon-client/26.3"
    if not resources.is_dir():
        print(f"missing prepared resources: {resources}", file=sys.stderr)
        return 2

    home = ROOT / ".local/framegen-present"
    if home.exists():
        subprocess.run(["rm", "-rf", str(home)], check=True)
    for folder in (home, home / "launcher", home / "xdg-data", home / "xdg-config"):
        folder.mkdir(parents=True)
    (home / "horizon-graphics.txt").write_text(GRAPHICS)
    log_path = home / "client.log"
    # game_env keeps this home and turns audio off. A missing home would fall back to the player's folder.
    env = lab.game_env(
        os.environ,
        HORIZON_HOME=home,
        HORIZON_LAUNCHER_HOME=home / "launcher",
        XDG_DATA_HOME=home / "xdg-data",
        XDG_CONFIG_HOME=home / "xdg-config",
    )
    command = [
        str(binary),
        "--resources",
        str(resources),
        "--game-dir",
        str(home),
        "--home",
        str(home),
        "--world",
        "Hang",
        "--creative",
        "--seed",
        "1",
        "--test-terrain",
        "--render-distance",
        "2",
        "--no-pause",
        "--width",
        "1280",
        "--height",
        "720",
        "--world-frames",
        str(args.world_frames),
    ]
    print(" ".join(command), flush=True)
    with log_path.open("w") as log:
        proc = subprocess.Popen(
            command,
            cwd=ROOT,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    deadline = time.monotonic() + args.seconds
    code = None
    while time.monotonic() < deadline:
        code = proc.poll()
        if code is not None:
            break
        time.sleep(0.25)
    if code is None:
        print(f"hang: still running after {args.seconds:.0f}s", flush=True)
        _sample(proc.pid, home / "sample.txt")
        _kill(proc)
        _tail(log_path)
        return 1
    print(f"exited {code}", flush=True)
    if code != 0:
        _tail(log_path)
        return 1
    text = log_path.read_text(errors="replace")
    if "Horizon: first frame" not in text:
        print("exited before the first frame", flush=True)
        _tail(log_path)
        return 1
    print("drew world frames and exited")
    return 0


def _library_test() -> int:
    print(
        "library test only: no window, no pointer grab",
        flush=True,
    )
    env = os.environ.copy()
    env["CARGO_BUILD_JOBS"] = env.get("CARGO_BUILD_JOBS", "2")
    result = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "horizon-client-render",
            "--lib",
            "the_present_thread_does_not_wait_on_the_main_queue",
        ],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    if result.returncode != 0 or "1 passed" not in result.stdout:
        print("the frame-generation test did not pass", file=sys.stderr)
        return result.returncode or 1
    return 0


def _sample(pid: int, path: Path) -> None:
    print(f"sampling pid {pid} -> {path}", flush=True)
    try:
        subprocess.run(
            ["/usr/bin/sample", str(pid), "2", "-file", str(path), "-mayDie"],
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"sample failed: {error}", flush=True)


def _kill(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except OSError:
        proc.kill()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


def _tail(path: Path) -> None:
    if not path.is_file():
        return
    lines = path.read_text(errors="replace").splitlines()
    print("--- client log ---")
    print("\n".join(lines[-40:]))


if __name__ == "__main__":
    sys.exit(main())
