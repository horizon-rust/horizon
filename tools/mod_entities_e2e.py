#!/usr/bin/env python3
"""Build and drive the Stage E host-record fixture in disposable native games."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import select
import shutil
import subprocess
import tempfile
import time
import zipfile


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def private_environment(scratch):
    env = {k: v for k, v in os.environ.items() if not k.startswith("HORIZON_")}
    for key in ["WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"]:
        env.pop(key, None)
    env["CARGO_HOME"] = os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    env["RUSTUP_HOME"] = os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))
    for key, part in [("HOME", "home"), ("HORIZON_HOME", "h"), ("HORIZON_LAUNCHER_HOME", "l"), ("XDG_DATA_HOME", "d"), ("XDG_CONFIG_HOME", "c"), ("XDG_STATE_HOME", "s"), ("XDG_CACHE_HOME", "k"), ("XDG_RUNTIME_DIR", "r"), ("TMPDIR", "t"), ("HORIZON_LAUNCHER_TRASH", "trash"), ("HORIZON_LAUNCHER_DESKTOP", "desktop"), ("XDG_DESKTOP_DIR", "desktop")]:
        path = scratch / part
        path.mkdir(exist_ok=True, mode=0o700)
        env[key] = str(path)
    (scratch / "java").mkdir()
    (scratch / "c/user-dirs.dirs").write_text(f'XDG_DESKTOP_DIR="{scratch}/desktop"\n')
    env.update(HORIZON_NO_AUDIO="1", HORIZON_NO_DISCORD="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1", HORIZON_LAUNCHER_CLIPBOARD=str(scratch / "clipboard"), JAVA_TOOL_OPTIONS=f"-Duser.home={scratch}/java -Djava.io.tmpdir={scratch}/t", WINIT_UNIX_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1")
    for path in ["/usr/share/vulkan/icd.d/lvp_icd.x86_64.json", "/usr/share/vulkan/icd.d/lvp_icd.json"]:
        if Path(path).is_file():
            env.update(VK_ICD_FILENAMES=path, VK_DRIVER_FILES=path)
            break
    return env


def fixture(root, component, index):
    owner = f"entity_host_{index}"
    command = f"entity{index}"
    package = root / owner
    package.mkdir()
    manifest = f'id="{owner}"\nname="Entity {index}"\nversion="0.1.0"\napi="0.3"\nside="server"\ncapabilities=["entities","resources","players"]\nexperimental=["entities","resources"]\nnamespaces=["entity_lab"]\n'
    nonce = secrets.token_bytes(24)
    update = secrets.token_bytes(32)
    assert nonce not in component and update not in component
    files = {"horizon-mod.toml": manifest.encode(), "mod.wasm": component, "data/entity_lab/config.txt": f"{owner}|{command}|".encode(), "data/entity_lab/nonce.bin": nonce, "data/entity_lab/update.bin": update}
    for name, data in files.items():
        path = package / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    path = package
    if index == 1:
        path = root / f"{owner}.hmod"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            for name, data in files.items():
                archive.writestr(name, data)
    return {"path": str(path), "owner": owner, "command": command, "nonce": list(nonce), "update": list(update)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--in-frame", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    assert not output.is_relative_to(root), "evidence belongs outside the source tree"
    output.mkdir(parents=True, exist_ok=True)
    target = args.target_dir.resolve()
    assert not target.is_relative_to(root), "use a separate build target"
    scratch = Path(tempfile.mkdtemp(prefix="he-", dir="/tmp"))
    env = private_environment(scratch)
    env.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS="4")
    sources = ["crates/horizon-client/src/game.rs", "crates/horizon-client/src/game/mod_entities.rs", "crates/horizon-client/src/game/mods.rs", "crates/horizon-client/src/game/server_mods.rs", "crates/horizon-client/src/game/levels.rs", "crates/horizon-client/src/game/persist.rs", "crates/horizon-mods/src/entities.rs", "crates/horizon-mods/src/runtime.rs", "crates/horizon-mods/src/manifest.rs", "crates/horizon-mods/src/api_level.rs", "crates/horizon-mod-sdk/wit/horizon.wit"]
    record = {"status": "building", "scratch": str(scratch), "server_thread": not args.in_frame, "source_sha256": {p: sha(root / p) for p in sources}, "calls": []}
    def save():
        (output / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    command = ["cargo", "build", "--locked", "--profile", "fast", "-p", "horizon-client", "--features", "mods", "--no-default-features", "--bin", "horizon-client", "--bin", "horizon-ctl"]
    started = time.monotonic()
    with (output / "build.log").open("w") as log:
        result = subprocess.run(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
    record["build"] = {"command": command, "exit": result.returncode, "seconds": time.monotonic() - started}
    save()
    assert result.returncode == 0, "native build failed"
    assert all(sha(root / p) == h for p, h in record["source_sha256"].items()), "source changed during build"
    game_binary = target / "fast/horizon-client"
    ctl = target / "fast/horizon-ctl"
    component = args.component.read_bytes()
    record.update(game_sha256=sha(game_binary), ctl_sha256=sha(ctl), component_sha256=hashlib.sha256(component).hexdigest())
    fixtures = [fixture(scratch, component, i) for i in range(3)]
    record["fixtures"] = fixtures
    socket = scratch / "ctl.sock"
    trace = output / "host-records.jsonl"
    assert not trace.exists(), "use fresh output for each run"
    env["HORIZON_MOD_ENTITIES_TRACE"] = str(trace)
    cwd = scratch / "world"
    cwd.mkdir()
    xvfb = game = None
    def call(words, timeout=15):
        response = subprocess.run([str(ctl), "--socket", str(socket), *words], cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
        lines = response.stdout.strip().splitlines()
        value = json.loads(lines[-1]) if lines else {"error": response.stderr.strip()}
        record["calls"].append({"args": words, "exit": response.returncode, "response": value})
        save()
        return response.returncode, value
    def player_command(line, expected):
        trace_since = len(trace.read_text().splitlines()) if trace.exists() else 0
        code, before = call(["events"])
        assert code == 0 and type(before.get("last")) is int, before
        since = before["last"]
        code, sent = call(["chat", "/" + line])
        assert code == 0 and sent.get("ok") is True, sent
        if expected is None:
            code, events = call(["events", "--since", str(since)])
            assert code == 0, events
            return {"since": since, "trace_since": trace_since, "events": events}
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and game.poll() is None:
            code, events = call(["events", "--since", str(since)])
            matched = [e for e in events.get("events", []) if e.get("seq", -1) > since and expected in json.dumps(e)]
            if code == 0 and matched:
                return {"since": since, "trace_since": trace_since, "events": matched}
            time.sleep(0.2)
        raise AssertionError(("no fresh player feedback", line, expected))
    def applied(predicate, since):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and game.poll() is None:
            for line in reversed(trace.read_text().splitlines()[since:] if trace.exists() else []):
                try:
                    state = json.loads(line)
                except json.JSONDecodeError:
                    continue
                records = [r for level in state["levels"] for r in level["records"]]
                if state["world"] is not None and predicate(records):
                    return state
            time.sleep(0.1)
        raise AssertionError("no matching applied host records")
    with (output / "xvfb.log").open("wb") as xlog, (output / "game.log").open("wb") as glog:
        try:
            read_fd, write_fd = os.pipe()
            xvfb = subprocess.Popen([shutil.which("Xvfb") or "Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1280x720x24", "-nolisten", "tcp"], cwd=cwd, env=env, pass_fds=(write_fd,), stdout=xlog, stderr=subprocess.STDOUT)
            os.close(write_fd)
            with os.fdopen(read_fd) as reader:
                assert select.select([reader], [], [], 10)[0], "owned Xvfb did not become ready"
                display = reader.readline().strip()
            assert display.isdigit(), display
            env["DISPLAY"] = ":" + display
            command = [str(game_binary), "--resources", str(args.resources.resolve()), "--world", "EntityLab", "--test-terrain", "--creative", "--offscreen", "--render-distance", "2", "--name", "EntityFixture", "--no-pause", "--in-frame" if args.in_frame else "--server-thread", "--control-socket", str(socket), "--timeout-seconds", "180"]
            for f in fixtures:
                command.extend(["--dev-mod", f["path"]])
            game = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=glog, stderr=subprocess.STDOUT)
            record.update(status="running", command=command, game_pid=game.pid, xvfb_pid=xvfb.pid, environment={k: v for k, v in env.items() if k.startswith(("HORIZON_", "XDG_")) or k in ["HOME", "DISPLAY", "JAVA_TOOL_OPTIONS", "TMPDIR"]})
            save()
            print(json.dumps({"status": "running", "output": str(output), "game_pid": game.pid}), flush=True)
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline and game.poll() is None:
                if socket.exists():
                    code, state = call(["state"])
                    if code == 0 and state.get("screen") == "playing" and state.get("world") == "EntityLab" and state.get("player") is not None and state.get("ticks", 0) > 0:
                        record["first_state"] = state
                        break
                time.sleep(0.3)
            else:
                raise AssertionError("native world did not become ready")
            record["proof"] = []
            for i, f in enumerate(fixtures):
                feedback = player_command(f"{f['command']} spawn", "e:")
                state = applied(lambda rs, i=i, f=f: len(rs) == i + 1 and any(r["kind"] == f["owner"] + ":probe" and r["public_state"] == f["nonce"] for r in rs), feedback["trace_since"])
                entity = next(r for level in state["levels"] for r in level["records"] if r["kind"] == f["owner"] + ":probe")
                assert entity["position"][0] % 16 == f["nonce"][0] % 8 + 0.5
                assert entity["position"][1] == 64 + f["nonce"][0] % 16
                assert entity["position"][2] % 16 == f["nonce"][-1] % 8 + 0.5
                record["proof"].append({"operation": f"spawn{i}", "applied": state, "feedback": feedback})
            first = fixtures[0]
            feedback = player_command(first["command"] + " update", "updated")
            state = applied(lambda rs: len(rs) == 3 and any(r["kind"] == first["owner"] + ":probe" and r["public_state"] == first["update"][::-1] and r["revision"] == 2 for r in rs), feedback["trace_since"])
            record["proof"].append({"operation": "update", "applied": state})
            feedback = player_command(fixtures[2]["command"] + " trap", None)
            state = applied(lambda rs: len(rs) == 2 and all(r["kind"] != fixtures[2]["owner"] + ":probe" for r in rs), feedback["trace_since"])
            record["proof"].append({"operation": "trap", "applied": state, "feedback": feedback})
            for f in fixtures[:2]:
                player_command(f["command"] + " update", "updated")
                feedback = player_command(f["command"] + " despawn", "despawned")
            state = applied(lambda rs: not rs, feedback["trace_since"])
            assert all(not level["allocated"] for level in state["levels"]), state
            record["proof"].append({"operation": "despawn", "applied": state})
            record["status"] = "passed"
        except BaseException as error:
            record.update(status="failed", error=repr(error))
        finally:
            if game is not None:
                if game.poll() is None:
                    try:
                        call(["quit"], timeout=10)
                        game.wait(timeout=30)
                    except BaseException as error:
                        record.update(status="failed", shutdown_error=repr(error))
                        game.terminate()
                        try:
                            game.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            game.kill()
                            game.wait(timeout=10)
                record["game_exit"] = game.returncode
                if game.returncode != 0:
                    record["status"] = "failed"
            if xvfb is not None:
                xvfb.terminate()
                try:
                    xvfb.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    xvfb.kill()
                    xvfb.wait(timeout=10)
                record["xvfb_exit"] = xvfb.returncode
            save()
    print(json.dumps({"status": record["status"], "output": str(output), "error": record.get("error")}), flush=True)
    return 0 if record["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
