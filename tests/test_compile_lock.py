"""tools/compile_lock.py: jobs on the self-hosted Mac take turns at compiling (#19151). Real processes and flock
where the system has them (Linux CI), a temporary folder."""

import ast
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from unittest import mock

from tools import compile_lock
from tools.compile_lock import Turn, WAIT

ROOT = Path(__file__).resolve().parents[1]
try:
    import fcntl  # noqa: F401
    HAS_FLOCK = True
except ImportError:
    HAS_FLOCK = False
needs_flock = unittest.skipUnless(HAS_FLOCK and os.name == "posix", "flock and process groups (POSIX)")
MAC = {"RUNNER_ENVIRONMENT": "self-hosted", "GITHUB_RUN_ID": "77", "GITHUB_WORKFLOW": "parity",
       "GITHUB_JOB": "parity", "RUNNER_NAME": "mac-runner"}


class Clock:
    """Fake monotonic time: a waiter's sleep moves it."""

    def __init__(self):
        self.now, self.slept = 100.0, 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        self.slept += seconds


HOLDER = textwrap.dedent("""
    import sys, time
    sys.path.insert(0, sys.argv[1])
    from tools.compile_lock import Turn
    turn = Turn(sys.argv[2], folder=sys.argv[3], say=lambda line: None)
    turn.__enter__()
    print("held" if turn.held else "not held", flush=True)
    time.sleep(120)
""")


class CompileLockTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = os.path.join(self.temp.name, "lock")
        self.clock = Clock()
        self.lines = []

    def turn(self, role, job=None, sleep=None, folder=None, environ=None, platform="darwin"):
        environ = dict(MAC if environ is None else environ, **({"GITHUB_JOB": job} if job else {}))
        return Turn(role, folder=folder or self.folder, environ=environ, platform=platform, clock=self.clock,
                    sleep=sleep or self.clock.sleep, say=self.lines.append)

    def holder_process(self, role="pr", environ=None):
        """A job in another process holding (or trying for) the turn; ended at cleanup."""
        process = subprocess.Popen([sys.executable, "-c", HOLDER, str(ROOT), role, self.folder],
                                   stdout=subprocess.PIPE, text=True, env=dict(os.environ, **(environ or MAC)))
        self.addCleanup(lambda: (process.poll() is None and process.kill(), process.wait(),
                                 process.stdout.close()))
        self.assertEqual(process.stdout.readline().strip(), "held")
        return process

    # The turn

    @needs_flock
    def test_taken_and_released(self):
        turn = self.turn("pr")
        self.assertTrue(turn.acquire())
        self.assertIn("parity / parity - role pr run 77 runner mac-runner pid ", Path(self.folder, "holder").read_text())
        self.clock.now += 90
        turn.release()
        self.assertEqual(self.lines, ["  compile lock: taken, no wait", "  compile lock: released after 1.5 min"])
        self.assertEqual(Path(self.folder, "holder").read_text(), "")
        self.assertEqual(sorted(os.listdir(self.folder)), ["holder", "priority", "turn"])
        self.assertTrue(self.turn("pr").acquire(), "free again")

    @needs_flock
    def test_a_killed_holder_frees_the_turn_at_once(self):
        process = self.holder_process("main")
        self.assertFalse(self.turn("build", sleep=lambda seconds: self.clock.sleep(WAIT["build"])).acquire())
        process.send_signal(signal.SIGKILL)
        process.wait()
        self.clock.slept = 0
        self.assertTrue(self.turn("build").acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_a_bounded_wait_goes_ahead_and_names_the_holder(self):
        self.holder_process("main", dict(MAC, GITHUB_WORKFLOW="parity-main", GITHUB_JOB="parity-main"))
        waiter = self.turn("build", job="build macos")
        self.assertFalse(waiter.acquire())
        self.assertEqual(self.clock.slept, WAIT["build"])
        self.assertTrue(self.lines[0].startswith("  compile lock: waiting: parity-main / parity-main - role main run "
                                                 "77 runner mac-runner pid "))
        self.assertTrue(self.lines[-1].startswith("  compile lock: waited 60 min, this job's limit; compiling "
                                                  "without the turn while parity-main / parity-main - role main"))
        self.assertEqual(len(self.lines), 2)

    @needs_flock
    def test_the_lock_never_reaches_a_child_process(self):
        # A child that inherits every descriptor it may (close_fds=False) and outlives the release, as sccache's
        # server or a lingering cargo would: the turn is free all the same.
        turn = self.turn("main")
        self.assertTrue(turn.acquire())
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], close_fds=False)
        self.addCleanup(lambda: (child.kill(), child.wait()))
        turn.release()
        self.assertIsNone(child.poll())
        self.assertTrue(self.turn("pr").acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_the_wrappers_command_never_holds_the_turn(self):
        command = ("import subprocess, sys; subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], "
                   "start_new_session=True, close_fds=False, stdout=subprocess.DEVNULL)")
        wrapper = subprocess.run([sys.executable, str(ROOT / "tools/compile_lock.py"), "build", "--", sys.executable,
                                  "-c", command], env=dict(os.environ, HORIZON_COMPILE_LOCK_DIR=self.folder),
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, timeout=60)
        self.assertEqual(wrapper.returncode, 0)
        self.assertIn("compile lock: taken, no wait", wrapper.stdout)
        self.assertTrue(self.turn("pr").acquire(), "the grandchild still runs, and the turn is free")
        self.assertEqual(self.clock.slept, 0)

    # Priority

    @needs_flock
    def test_a_pull_request_waits_while_a_priority_job_is_here_even_with_the_turn_free(self):
        main = self.turn("main", job="parity-main")
        main.open()
        self.assertTrue(main.arrive())  # arrived; the turn itself is free
        pr = self.turn("pr")
        self.assertFalse(pr.acquire())
        self.assertEqual(self.clock.slept, WAIT["pr"])
        self.assertIn("jobs ahead of pull requests (parity-main, a Mac build) are here", self.lines[0])
        self.assertTrue(main.try_turn(), "the free turn was left to parity-main")
        main.release()
        self.clock.slept = 0
        self.assertTrue(self.turn("pr").acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_a_priority_job_arriving_between_a_pull_requests_checks(self):
        # Hold `priority` exclusively (the pull request's step 1); the priority job's arrival is refused, so it is
        # not here yet: the pull request takes the turn, and the priority job then waits for the turn.
        pr, main = self.turn("pr"), self.turn("main", job="parity-main")
        pr.open()
        main.open()
        arrivals = []
        real = pr.lock

        def lock(descriptor, mode):
            taken = real(descriptor, mode)
            if descriptor == pr.priority and mode == pr.fcntl.LOCK_EX and taken:
                arrivals.append(main.arrive())  # between step 1 and step 2
            return taken
        pr.lock = lock
        self.assertTrue(pr.try_turn())
        self.assertEqual(arrivals, [False])
        self.assertTrue(main.arrive(), "step 3 let `priority` go")
        self.assertFalse(main.try_turn(), "it waits for the turn like anybody")
        # And the other order: arrived first, the pull request's step 1 is refused.
        pr.release()
        other = self.turn("pr")
        other.open()
        self.assertFalse(other.try_turn())
        self.assertTrue(other.ahead)
        self.assertTrue(main.try_turn())

    @needs_flock
    def test_priority_jobs_wait_for_no_pull_request_that_only_waits(self):
        waiting = self.turn("pr")
        waiting.open()  # a pull request in its wait loop holds no lock between attempts
        main, build = self.turn("main", job="parity-main"), self.turn("build", job="build macos")
        self.assertTrue(main.acquire())
        main.release()
        self.assertTrue(build.acquire())
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_a_priority_job_that_goes_ahead_keeps_pull_requests_waiting(self):
        self.holder_process("pr")
        main = self.turn("main", job="parity-main")
        self.assertFalse(main.acquire())
        self.assertEqual(self.clock.slept, WAIT["main"])
        self.clock.slept = 0
        late = self.turn("pr", job="late")
        self.assertFalse(late.acquire())
        self.assertIn("jobs ahead of pull requests", self.lines[-1])
        main.release()

    # Fail open, and what the folder may hold

    @needs_flock
    def test_fails_open_where_the_folder_cannot_be_made(self):
        blocker = Path(self.temp.name, "a-file")
        blocker.write_text("")
        with self.turn("main", folder=str(blocker / "lock")) as turn:
            self.assertFalse(turn.held)
        self.assertEqual(len(self.lines), 1)
        self.assertTrue(self.lines[0].endswith("; compiling without it"))
        self.assertEqual(self.clock.slept, 0)

    @needs_flock
    def test_hostile_folder_contents_never_block_or_reach_outside(self):
        outside = Path(self.temp.name, "outside")
        outside.mkdir()
        target = outside / "precious"
        target.write_text("keep")
        cases = {
            "symlink root": lambda: os.symlink(outside, self.folder),
            "symlink turn": lambda: (os.mkdir(self.folder), os.symlink(target, os.path.join(self.folder, "turn"))),
            "fifo turn": lambda: (os.mkdir(self.folder), os.mkfifo(os.path.join(self.folder, "turn"))),
            "fifo priority": lambda: (os.mkdir(self.folder), os.mkfifo(os.path.join(self.folder, "priority"))),
            "directory turn": lambda: (os.mkdir(self.folder), os.mkdir(os.path.join(self.folder, "turn"))),
            "hard link turn": lambda: (os.mkdir(self.folder), os.link(target, os.path.join(self.folder, "turn"))),
        }
        for name, make in cases.items():
            with self.subTest(name):
                self.reset_folder()
                make()
                self.lines.clear()
                started = time.monotonic()
                with self.turn("pr") as turn:
                    self.assertFalse(turn.held)
                self.assertLess(time.monotonic() - started, 5)
                self.assertEqual(len(self.lines), 1, self.lines)
                self.assertTrue(self.lines[0].endswith("compiling without it"))
                self.assertEqual(target.read_text(), "keep")
                self.assertEqual(sorted(os.listdir(outside)), ["precious"])
        for name, make in {
            "symlink holder": lambda: os.symlink(target, os.path.join(self.folder, "holder")),
            "hard link holder": lambda: os.link(target, os.path.join(self.folder, "holder")),
            "fifo holder": lambda: os.mkfifo(os.path.join(self.folder, "holder")),
            "directory holder": lambda: os.mkdir(os.path.join(self.folder, "holder")),
        }.items():
            with self.subTest(name):
                self.reset_folder()
                os.mkdir(self.folder)
                make()
                started = time.monotonic()
                with self.turn("pr") as turn:
                    self.assertTrue(turn.held, "the display file never decides")
                    self.assertEqual(turn.holder(), "")
                self.assertLess(time.monotonic() - started, 5)
                self.assertEqual(target.read_text(), "keep")

    def reset_folder(self):
        if os.path.islink(self.folder) or os.path.isfile(self.folder):
            os.unlink(self.folder)
        elif os.path.isdir(self.folder):
            for name in os.listdir(self.folder):
                path = os.path.join(self.folder, name)
                os.rmdir(path) if os.path.isdir(path) and not os.path.islink(path) else os.unlink(path)
            os.rmdir(self.folder)

    @needs_flock
    def test_the_holders_text_reaches_a_log_only_through_the_allow_list(self):
        process = self.holder_process("main")
        hostile = b"##[error]boom ::error::x \x1b[31mred\x1b[0m\n::warning::y \xff\xfe" + b"z" * 5000
        Path(self.folder, "holder").write_bytes(hostile)
        self.turn("build").acquire()
        process.kill()
        for line in self.lines:
            self.assertTrue(line.startswith("  compile lock: "))  # so `::` can't start a line
            for marker in ("##[", "\x1b", "\n", "\r"):
                self.assertNotIn(marker, line)
            self.assertLess(len(line), 600)
        self.assertIn("##?error?boom ::error::x ??31mred??0m?::warning::y ??zzz", self.lines[0])

    def test_clean_keeps_only_the_allow_list(self):
        self.assertEqual(compile_lock.clean("a-Z_0.9:/#@ ok"), "a-Z_0.9:/#@ ok")
        self.assertEqual(compile_lock.clean("##[error]x\n::y\x1b[0m"), "##?error?x?::y??0m")

    def test_a_no_op_off_the_mac(self):
        cases = [(MAC, "linux"), (MAC, "win32"), (dict(MAC, RUNNER_ENVIRONMENT="github-hosted"), "darwin"),
                 ({}, "darwin")]
        for environ, platform in cases:
            turn = Turn("main", environ=environ, platform=platform, say=self.lines.append)
            with turn:
                self.assertFalse(turn.held)
        with self.turn("off") as turn:
            self.assertFalse(turn.held)
        self.assertFalse(os.path.exists(self.folder))
        self.assertEqual(self.lines, [])

    def test_imports_and_fails_open_without_fcntl(self):
        # Windows has no fcntl: importing the module (and tools/parity.py) works, a turn fails open.
        script = textwrap.dedent("""
            import sys
            sys.modules["fcntl"] = None
            sys.path.insert(0, sys.argv[1])
            from tools import parity
            from tools.compile_lock import Turn
            lines = []
            with Turn("main", folder=sys.argv[2], say=lines.append) as turn:
                assert not turn.held
            assert len(lines) == 1 and lines[0].endswith("compiling without it"), lines
        """)
        result = subprocess.run([sys.executable, "-c", script, str(ROOT), self.folder], capture_output=True,
                                text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stderr)

    # The wrapper

    def test_the_wrapper_returns_its_commands_status(self):
        with mock.patch.object(compile_lock.signal, "signal"):
            self.assertEqual(compile_lock.main(["off", "--", sys.executable, "-c", "raise SystemExit(3)"]), 3)
            self.assertEqual(compile_lock.main(["build"]), 2)
            self.assertEqual(compile_lock.main(["--probe"]), 0)  # what tools/ci/compile-lock.sh checks first

    @needs_flock
    def test_a_cancelled_wrapper_ends_its_commands_group_and_frees_the_turn(self):
        # The command's child ignores SIGTERM: the group still ends after the grace, and the turn is free.
        command = ("import signal, subprocess, sys, time; "
                   "subprocess.Popen([sys.executable, '-c', 'import signal, time; "
                   "signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(120)']); time.sleep(120)")
        script = textwrap.dedent("""
            import sys
            sys.path.insert(0, sys.argv[1])
            from tools import compile_lock
            compile_lock.GRACE = 1
            sys.exit(compile_lock.main(sys.argv[2:]))
        """)
        wrapper = subprocess.Popen([sys.executable, "-c", script, str(ROOT), "build", "--", sys.executable, "-c",
                                    command], env=dict(os.environ, HORIZON_COMPILE_LOCK_DIR=self.folder),
                                   stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (wrapper.poll() is None and wrapper.kill(), wrapper.stdout.close()))
        self.assertIn("taken", wrapper.stdout.readline())
        time.sleep(1)
        started = time.monotonic()
        wrapper.send_signal(signal.SIGTERM)
        self.assertEqual(wrapper.wait(timeout=30), 128 + signal.SIGTERM)
        self.assertLess(time.monotonic() - started, 15)
        self.assertTrue(self.turn("pr").acquire())

    def test_the_module_runs_on_the_macs_system_python(self):
        ast.parse(Path(compile_lock.__file__).read_text(encoding="utf-8"), feature_version=(3, 9))

    def test_the_mac_compiles_take_their_turn(self):
        with mock.patch("sys.path", [str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"), *sys.path]):
            import yaml
        game = yaml.safe_load((ROOT / ".github/workflows/game-build.yml").read_text(encoding="utf-8"))["jobs"]
        wrapped = 'lock="bash tools/ci/compile-lock.sh build --"'
        exporters = next(step["run"] for step in game["exporters"]["steps"] if "cargo build" in step.get("run", ""))
        self.assertIn(wrapped, exporters)
        self.assertIn("$lock cargo build --release --locked -p horizon-prepare", exporters)
        build = next(step["run"] for step in game["build"]["steps"] if "cargo build" in step.get("run", ""))
        self.assertIn(f'[ "${{{{ matrix.mac }}}}" != true ] || [ ! -f tools/ci/compile-lock.sh ] || {wrapped}', build)
        self.assertIn("$lock cargo build --release --locked", build)
        # Room for the 60 min wait: 120 min on the Mac legs.
        self.assertEqual(game["exporters"]["timeout-minutes"], 120)
        self.assertTrue(game["build"]["timeout-minutes"].startswith("${{ matrix.mac && 120 ||"))
        main = yaml.safe_load((ROOT / ".github/workflows/parity-main.yml").read_text(encoding="utf-8"))
        steps = main["jobs"]["parity-main"]["steps"]
        harnesses = next(step for step in steps if step.get("id") == "harnesses")
        self.assertEqual(harnesses["env"]["HORIZON_COMPILE_LOCK_ROLE"], "main")
        verdict = next(step for step in steps if "verdict" in step.get("run", ""))
        self.assertEqual(verdict["env"]["HORIZON_COMPILE_LOCK_ROLE"], "off")

if __name__ == "__main__":
    unittest.main()
