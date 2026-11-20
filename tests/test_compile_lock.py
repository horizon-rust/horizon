"""tools/compile_lock.py: the Mac mini's runners take turns at compiling (#19151). Fake clocks, a temporary folder."""

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

from tools import compile_lock
from tools.compile_lock import STALE, Turn, WAIT

MAC = {"RUNNER_ENVIRONMENT": "self-hosted", "GITHUB_RUN_ID": "77", "GITHUB_JOB": "parity",
       "RUNNER_NAME": "ludovics-mac-mini"}


class Clock:
    """Shared fake time: a waiter's sleep moves it, and with it every other job's view."""

    def __init__(self):
        self.now = 1_000_000.0
        self.slept = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        self.slept += seconds


class CompileLockTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name) / "compile-lock"
        self.clock = Clock()
        self.lines = []

    def turn(self, role, job=None, environ=None, platform="darwin", folder=None, sleep=None):
        environ = dict(MAC if environ is None else environ, **({"GITHUB_JOB": job} if job else {}))
        return Turn(role, folder=folder or self.folder, environ=environ, platform=platform, clock=self.clock,
                    sleep=sleep or self.clock.sleep, say=self.lines.append, beat=None)

    def test_taken_and_released(self):
        turn = self.turn("pr")
        self.assertTrue(turn.acquire())
        self.assertEqual(json.loads((self.folder / "held" / "owner.json").read_text())["token"], turn.token)
        self.assertEqual(list(self.folder.glob("wants-*")), [], "the marker goes once the lock is taken")
        self.clock.now += 90
        turn.release()
        self.assertFalse((self.folder / "held").exists())
        self.assertEqual(sorted(path.name for path in self.folder.iterdir()), [])
        self.assertEqual(self.lines, ["  compile lock: taken, no wait", "  compile lock: released after 1.5 min"])
        self.assertTrue(self.turn("pr").acquire(), "free again")

    def test_never_stale_while_the_heartbeat_is_fresh_even_after_hours(self):
        holder = self.turn("main", job="parity-main")
        self.assertTrue(holder.acquire())
        waiter = self.turn("build", job="build macos")

        def sleep(seconds):
            self.clock.sleep(seconds)
            holder.heartbeat()  # the holder's thread, beating while its cargo runs
        waiter.sleep = sleep
        self.clock.now += 3 * 3600
        holder.heartbeat()
        self.assertFalse(waiter.acquire())  # its own limit, never a takeover
        self.assertEqual(self.clock.slept, WAIT["build"])
        self.assertEqual(json.loads((self.folder / "held" / "owner.json").read_text())["token"], holder.token)

    def test_stale_by_heartbeat_only(self):
        dead = self.turn("pr", job="dead")
        self.assertTrue(dead.acquire())
        self.clock.now += STALE - 1
        self.assertTrue(self.turn("build").fresh(dead.holder()))
        self.clock.now += 1  # 15 min without a beat
        taker = self.turn("build", job="build macos")
        self.lines.clear()
        self.assertTrue(taker.acquire())
        self.assertIn("took over from dead (pr, run 77, runner ludovics-mac-mini), whose heartbeat stopped 15 min ago",
                      self.lines[0])
        dead.release()  # the dead job's release, had it come back, never removes the new holder's lock
        self.assertEqual(json.loads((self.folder / "held" / "owner.json").read_text())["token"], taker.token)
        self.assertIn("not released: this job's lock was taken over", self.lines[-1])

    def test_a_second_waiter_cannot_move_the_lock_the_first_took_over(self):
        dead = self.turn("pr", job="dead")
        dead.acquire()
        self.clock.now += STALE + 1
        stale = dead.holder()
        first, second = self.turn("main", job="first"), self.turn("pr", job="second")
        first.take_over(stale)
        first.folder.mkdir(exist_ok=True)
        self.assertTrue(first.try_take())
        second.take_over(stale)  # it saw the same stale lock before the first acted
        self.assertEqual(first.holder()["token"], first.token)

    def test_pull_requests_wait_while_parity_main_wants_or_holds_the_lock(self):
        self.folder.mkdir()
        main = self.turn("main", job="parity-main")
        main.want()  # arrived, waiting for someone else's turn
        pr = self.turn("pr", job="parity")

        def sleep(seconds):
            self.clock.sleep(seconds)
            main.want()  # parity-main's own waiting loop refreshes its marker
        pr.sleep = sleep
        self.assertFalse(pr.acquire())
        self.assertFalse((self.folder / "held").exists(), "the free lock was left to parity-main")
        self.assertEqual(self.clock.slept, WAIT["pr"])
        self.assertIn("waiting: parity-main (main, run 77, runner ludovics-mac-mini) wants it first", self.lines[0])
        self.assertIn("waited 60 min, the limit for this job; compiling anyway while parity-main", self.lines[-1])
        main.unwant()
        self.assertTrue(main.acquire())
        self.lines.clear()
        self.assertFalse(self.turn("pr", sleep=lambda seconds: (self.clock.sleep(seconds), main.heartbeat())).acquire())
        self.assertIn("parity-main (main, run 77, runner ludovics-mac-mini) holds it", self.lines[0])

    def test_parity_main_does_not_wait_for_a_pull_request_that_only_wants_it(self):
        self.folder.mkdir()
        self.turn("pr", job="parity").want()
        self.turn("build", job="build macos").want()
        main = self.turn("main", job="parity-main")
        self.assertTrue(main.acquire())
        self.assertEqual(self.clock.slept, 0)

    def test_a_dead_parity_mains_marker_blocks_nobody(self):
        self.folder.mkdir()
        self.turn("main", job="parity-main").want()
        self.clock.now += STALE
        self.assertTrue(self.turn("pr").acquire())
        self.assertEqual(self.clock.slept, 0)

    def test_parity_main_keeps_its_marker_when_it_goes_ahead_and_drops_it_on_release(self):
        holder = self.turn("pr", job="parity")
        holder.acquire()

        def sleep(seconds):
            self.clock.sleep(seconds)
            holder.heartbeat()
        main = self.turn("main", job="parity-main", sleep=sleep)
        self.assertFalse(main.acquire())
        self.assertEqual(self.clock.slept, WAIT["main"])
        self.assertIn("waited 20 min, the limit for this job; compiling anyway while parity (pr, run 77, runner "
                      "ludovics-mac-mini) holds it", self.lines[-1])
        self.assertTrue(main.wants_file.exists(), "no other pull request starts beside parity-main meanwhile")
        holder.release()
        self.clock.slept = 0
        waiting = self.turn("pr", job="next", sleep=lambda seconds: (self.clock.sleep(seconds), main.heartbeat()))
        self.assertFalse(waiting.acquire())
        self.assertEqual(self.clock.slept, WAIT["pr"])
        main.release()
        self.assertFalse(main.wants_file.exists())

    def test_a_bounded_wait_names_the_holder(self):
        holder = self.turn("main", job="parity-main")
        holder.acquire()
        self.lines.clear()
        build = self.turn("build", job="build macos",
                          sleep=lambda seconds: (self.clock.sleep(seconds), holder.heartbeat()))
        self.assertFalse(build.acquire())
        self.assertEqual(self.lines, [
            "  compile lock: waiting: parity-main (main, run 77, runner ludovics-mac-mini) holds it",
            "  compile lock: waited 10 min, the limit for this job; compiling anyway while parity-main (main, run 77, "
            "runner ludovics-mac-mini) holds it"])
        self.assertEqual(list(self.folder.glob("wants-*")), [], "only parity-main keeps a marker after its wait")

    def test_taken_after_a_wait_says_how_long_and_on_whom(self):
        holder = self.turn("pr", job="parity")
        holder.acquire()
        self.lines.clear()

        def sleep(seconds):
            self.clock.sleep(seconds)
            if self.clock.slept >= 300:
                holder.release()
        self.assertTrue(self.turn("main", job="parity-main", sleep=sleep).acquire())
        self.assertEqual(self.lines[-1], "  compile lock: taken after waiting 5.0 min for parity (pr, run 77, "
                                         "runner ludovics-mac-mini)")

    def test_fails_open_where_the_folder_cannot_be_made(self):
        blocker = Path(self.temp.name) / "a-file"
        blocker.write_text("")
        turn = self.turn("main", folder=blocker / "compile-lock")
        with turn:
            pass
        self.assertFalse(turn.held)
        self.assertEqual(len(self.lines), 1)
        self.assertIn("can't be used", self.lines[0])
        self.assertTrue(self.lines[0].endswith("; compiling without it"))

    def test_fails_open_on_odd_content(self):
        self.folder.mkdir()
        (self.folder / "held").mkdir()
        (self.folder / "held" / "owner.json").write_text("{\"token\": 5}\n::error::injected")
        turn = self.turn("pr")
        self.assertFalse(turn.acquire())
        self.assertEqual(self.clock.slept, 0)
        self.assertNotIn("\n", self.lines[0])
        self.assertIn("the lock folder holds something else", self.lines[0])
        (self.folder / "held" / "owner.json").unlink()
        (self.folder / "held").rmdir()
        (self.folder / "held").write_text("not a directory")
        self.assertFalse(self.turn("pr").acquire())

    def test_a_holders_description_cannot_start_a_workflow_command(self):
        holder = self.turn("pr", environ=dict(MAC, GITHUB_JOB="x\n::error::boom", RUNNER_NAME="r\r\n::warning::"))
        holder.acquire()
        self.lines.clear()
        self.turn("build", sleep=lambda seconds: (self.clock.sleep(seconds), holder.heartbeat())).acquire()
        for line in self.lines:
            self.assertNotIn("\n", line)
            self.assertNotIn("\r", line)
            self.assertTrue(line.startswith("  compile lock: "))

    def test_release_never_removes_someone_elses_lock(self):
        mine, theirs = self.turn("pr", job="mine"), self.turn("main", job="theirs")
        mine.acquire()
        self.clock.now += STALE
        self.assertTrue(theirs.acquire())
        mine.heartbeat()  # notices, and stops beating someone else's lock
        self.assertFalse(mine.held)
        mine.release()
        self.assertEqual(theirs.holder()["token"], theirs.token)

    def test_a_no_op_off_the_mac_mini(self):
        cases = [(MAC, "linux"), (MAC, "win32"), (dict(MAC, RUNNER_ENVIRONMENT="github-hosted"), "darwin"),
                 ({}, "darwin")]
        for environ, platform in cases:
            with self.turn("main", environ=environ, platform=platform) as turn:
                self.assertFalse(turn.held)
        with self.turn("off") as turn:
            self.assertFalse(turn.held)
        self.assertFalse(self.folder.exists())
        self.assertEqual(self.lines, [])

    def test_heartbeat_thread_keeps_the_lock_fresh(self):
        turn = Turn("pr", folder=self.folder, environ=MAC, platform="darwin", say=self.lines.append, beat=0.01)
        with turn:
            before = turn.holder()["beat"]
            for _ in range(200):
                if turn.holder()["beat"] != before:
                    break
                turn.stop.wait(0.01)
            self.assertNotEqual(turn.holder()["beat"], before)
        self.assertFalse((self.folder / "held").exists())

    def test_the_wrapper_runs_the_command_and_returns_its_status(self):
        with mock.patch.object(compile_lock, "LOCK_DIR", self.folder), mock.patch.object(compile_lock.signal, "signal"):
            code = compile_lock.main(["build", "--", sys.executable, "-c", "raise SystemExit(3)"])
        self.assertEqual(code, 3)
        self.assertEqual(compile_lock.main(["build"]), 2)
        self.assertEqual(compile_lock.main(["--probe"]), 0)  # what tools/ci/compile-lock.sh checks first

    def test_the_wrapper_holds_the_lock_while_its_command_runs(self):
        script = Path(self.temp.name) / "check.py"
        script.write_text("import os, sys; sys.exit(0 if os.path.isdir(sys.argv[1]) else 4)")
        with mock.patch.object(compile_lock, "LOCK_DIR", self.folder), mock.patch.object(compile_lock.signal, "signal"), \
                mock.patch.object(compile_lock, "active", return_value=True):
            code = compile_lock.main(["build", "--", sys.executable, str(script), str(self.folder / "held")])
        self.assertEqual(code, 0)
        self.assertFalse((self.folder / "held").exists())

    def test_the_module_runs_on_the_macs_system_python(self):
        import ast
        source = Path(compile_lock.__file__).read_text(encoding="utf-8")
        ast.parse(source, feature_version=(3, 9))  # /usr/bin/python3 before runner-env.sh

    def test_the_mac_compiles_take_their_turn(self):
        root = Path(__file__).resolve().parents[1]
        game = (root / ".github/workflows/game-build.yml").read_text(encoding="utf-8")
        self.assertEqual(game.count('lock="bash tools/ci/compile-lock.sh build --"'), 2)
        self.assertEqual(game.count("$lock cargo build --release --locked"), 2)
        main = (root / ".github/workflows/parity-main.yml").read_text(encoding="utf-8")
        self.assertIn("HORIZON_COMPILE_LOCK_ROLE: main", main)
        self.assertIn("HORIZON_COMPILE_LOCK_ROLE: 'off'", main)


if __name__ == "__main__":
    unittest.main()
