#!/usr/bin/env python3
"""Run the game in VR on the fake OpenXR runtime (crates/horizon-xr-fake): no headset, no
SteamVR, nothing installed. The game runs from a fresh temporary folder that links the
prepared resources read-only, plays a new world for a number of frames with a scripted head
and controllers, then exits; the fake runtime's report (frame times, layers, actions) and
captures of each eye and the HUD panel land in --out.

  python3 tools/vr_fake_run.py --resources PATH/WITH/.local --out vr-run
  python3 tools/vr_fake_run.py --resources ... --script tools/vr_fake_scripts/look_around.txt --hz 1000
  python3 tools/vr_fake_run.py --resources ... --title --world-frames 2200 \
      --script tools/vr_fake_scripts/title_laser.txt --capture-seconds 6,11,15,18

--title stays on the title screen (no world) for --world-frames frames in all.

--hz 1000 lets the game run as fast as it can (the fake headset never waits), to measure it.
Build first: cargo build --release -p horizon-client -p horizon-xr -p horizon-xr-fake
"""

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import shlex
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WINDOWS = platform.system() == "Windows"
EXE = ".exe" if WINDOWS else ""
FAKE_LIB = "horizon_xr_fake.dll" if WINDOWS else (
    "libhorizon_xr_fake.dylib" if platform.system() == "Darwin" else "libhorizon_xr_fake.so"
)
LOADER = "openxr_loader.dll" if WINDOWS else None


def link_dir(src: Path, dst: Path) -> None:
    """Links a folder without copying it (a junction on Windows needs no privileges)."""
    if WINDOWS:
        subprocess.run(["cmd", "/c", "mklink", "/J", str(dst), str(src)], check=True,
                       stdout=subprocess.DEVNULL)
    else:
        dst.symlink_to(src, target_is_directory=True)


def drive(build: Path, socket: Path, script: Path, out: Path, proc: subprocess.Popen) -> None:
    """Sends `script`'s horizon-ctl commands to the game in turn, once its control socket is up."""
    ctl = build / f"horizon-ctl{EXE}"
    with open(out / "ctl.log", "w", encoding="utf-8") as log:
        while not socket.exists() and proc.poll() is None:
            time.sleep(0.5)
        for line in script.read_text().splitlines():
            line = line.split("#", 1)[0].strip()
            if not line or proc.poll() is not None:
                continue
            if line.startswith("sleep "):
                time.sleep(float(line.split()[1]))
                continue
            reply = subprocess.run([str(ctl), "--socket", str(socket), *shlex.split(line)],
                                   capture_output=True, text=True)
            log.write(f"[{time.strftime('%H:%M:%S')}] {line}\n{reply.stdout}{reply.stderr}\n")
            log.flush()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--build", default=str(ROOT / "target" / "release"), help="folder with the built binaries")
    ap.add_argument("--resources", required=True, help="a game folder holding the prepared .local")
    ap.add_argument("--loader", help="openxr_loader.dll (Windows); default: .local/openxr-loader or beside the build")
    ap.add_argument("--out", default="vr-fake-run", help="report and captures")
    ap.add_argument("--world-frames", type=int, default=900, help="frames in the world before the game exits")
    ap.add_argument("--size", default="1600x1600", help="each eye's image")
    ap.add_argument("--hz", type=float, default=90.0, help="the fake headset's rate (1000: unpaced)")
    ap.add_argument("--script", help="head and controller script (crates/horizon-xr-fake docs)")
    ap.add_argument("--capture-every", type=int, default=180, help="capture the eyes and panel every N frames")
    ap.add_argument("--capture-seconds", help="also capture at these script times, e.g. 6,9.5 (t6-*.png)")
    ap.add_argument("--title", action="store_true", help="stay on the title screen: no world")
    ap.add_argument("--seed", default="1")
    ap.add_argument("--settings-from", help="a game folder whose options.txt and horizon-*.txt to play with")
    ap.add_argument("--shaderpacks", help="a shaderpacks folder to link beside the game (pickedShaderPack picks one)")
    ap.add_argument("--timeout", type=float, default=600.0)
    ap.add_argument("--keep", action="store_true", help="keep the temporary game folder")
    ap.add_argument("--ctl", help="a file of horizon-ctl commands sent to the game in turn (one a line, "
                    "'sleep SECONDS' waits; replies go to ctl.log), e.g. to teleport the player")
    ap.add_argument("--no-clouds", action="store_true", help="play without clouds (frames compare cleanly)")
    ap.add_argument("--home", help="a folder to keep HORIZON_HOME and the saves in: the next run with it plays the same world")
    args = ap.parse_args()

    build = Path(args.build).resolve()
    resources = Path(args.resources).resolve() / ".local"
    if not resources.is_dir():
        print(f"no prepared resources at {resources}", file=sys.stderr)
        return 2
    out = Path(args.out).resolve()
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)

    game = Path(tempfile.mkdtemp(prefix="horizon-vr-fake-"))
    try:
        shutil.copy2(build / f"horizon-client{EXE}", game / f"horizon-client{EXE}")
        shutil.copy2(build / FAKE_LIB, game / FAKE_LIB)
        if LOADER:
            candidates = [Path(args.loader)] if args.loader else [
                ROOT / ".local" / "openxr-loader" / LOADER, build / LOADER]
            loader = next((c for c in candidates if c.is_file()), None)
            if loader is None:
                print("no openxr_loader.dll: pass --loader", file=sys.stderr)
                return 2
            shutil.copy2(loader, game / LOADER)
        (game / "horizon-xr-fake.json").write_text(json.dumps({
            "file_format_version": "1.0.0",
            "runtime": {"name": "Horizon fake XR", "library_path": f"./{FAKE_LIB}"},
        }, indent=2))
        link_dir(resources, game / ".local")
        if args.settings_from:
            for name in ("options.txt", "horizon-graphics.txt", "horizon-extras.txt"):
                src = Path(args.settings_from) / name
                if src.is_file():
                    shutil.copy2(src, game / name)
        if args.shaderpacks:
            link_dir(Path(args.shaderpacks).resolve(), game / "shaderpacks")
        if args.no_clouds:
            options = game / "options.txt"
            lines = options.read_text().splitlines() if options.is_file() else []
            lines = [l for l in lines if not l.startswith("renderClouds:")] + ['renderClouds:"false"']
            options.write_text("\n".join(lines) + "\n")
        home = Path(args.home).resolve() if args.home else game / "home"
        home.mkdir(parents=True, exist_ok=True)
        if args.home:
            # The game saves beside itself: the saves folder is linked into the kept one.
            (home / "saves").mkdir(exist_ok=True)
            link_dir(home / "saves", game / "saves")

        env = dict(os.environ)
        env.update({
            "HORIZON_XR": "1",
            "XR_RUNTIME_JSON": str(game / "horizon-xr-fake.json"),
            "HORIZON_FAKE_XR_OUT": str(out),
            "HORIZON_FAKE_XR_SIZE": args.size,
            "HORIZON_FAKE_XR_HZ": str(args.hz),
            "HORIZON_FAKE_XR_CAPTURE_EVERY": str(args.capture_every),
            "HORIZON_HOME": str(home),
            "HORIZON_NO_AUDIO": "1",
            "HORIZON_LAUNCHER_NO_SELF_UPDATE": "1",
            "XDG_DATA_HOME": str(game / "xdg-data"),
            "XDG_CONFIG_HOME": str(game / "xdg-config"),
        })
        if args.script:
            env["HORIZON_FAKE_XR_SCRIPT"] = str(Path(args.script).resolve())
        if args.capture_seconds:
            env["HORIZON_FAKE_XR_CAPTURE_SECONDS"] = args.capture_seconds
        if args.title:
            cmd = [str(game / f"horizon-client{EXE}"), "--frames", str(args.world_frames)]
        else:
            cmd = [str(game / f"horizon-client{EXE}"), "--world", "VR fake test", "--seed", args.seed,
                   "--world-frames", str(args.world_frames)]
        # The game gives up a frame-limited run after its own timeout (120 s by default).
        cmd += ["--timeout-seconds", str(int(args.timeout))]
        socket = game / "control.json"
        if args.ctl:
            cmd += ["--control-socket", str(socket)]
        started = time.time()
        with open(out / "game.log", "w", encoding="utf-8", errors="replace") as log:
            proc = subprocess.Popen(cmd, cwd=game, env=env, stdout=log, stderr=subprocess.STDOUT)
            if args.ctl:
                threading.Thread(target=drive, args=(build, socket, Path(args.ctl), out, proc),
                                 daemon=True).start()
            try:
                # The game stops itself at --timeout; the margin covers its saving.
                proc.wait(timeout=args.timeout + 60)
            except subprocess.TimeoutExpired:
                if WINDOWS:
                    subprocess.run(["taskkill", "/F", "/T", "/PID", str(proc.pid)], check=False,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                else:
                    proc.kill()
                proc.wait()
                raise
        elapsed = time.time() - started
        report_path = out / "report.json"
        report = json.loads(report_path.read_text()) if report_path.is_file() else None
        vr_lines = [line for line in (out / "game.log").read_text(errors="replace").splitlines()
                    if "Horizon VR" in line or "panicked" in line]
        print(f"game exited with {proc.returncode} after {elapsed:.0f} s")
        for line in vr_lines[-5:]:
            print(f"  {line}")
        if report is None:
            print("no report: the game never opened an OpenXR session", file=sys.stderr)
            return 1
        print(json.dumps({k: report[k] for k in ("frames", "fps", "last_300", "layers", "errors")}, indent=2))
        print(f"captures in {out}")
        return 0 if proc.returncode == 0 and not report["errors"] else 1
    finally:
        if args.keep:
            print(f"game folder kept: {game}")
        else:
            for linked in (".local", "saves", "shaderpacks"):
                if not args.home and linked == "saves" or not args.shaderpacks and linked == "shaderpacks":
                    continue
                if WINDOWS and (game / linked).exists():
                    # Remove the junction itself, never what it points to.
                    subprocess.run(["cmd", "/c", "rmdir", str(game / linked)], check=False)
                elif (game / linked).is_symlink():
                    (game / linked).unlink()
            shutil.rmtree(game, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
