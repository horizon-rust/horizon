#!/usr/bin/env python3
"""Turns at compiling for the Mac mini's self-hosted runners (#19151).

parity-main, pull requests' parity jobs, the Nightly's (and a release's) Mac legs run as one macOS user on one
machine, and compiled side by side: parity-main's probe prebuild took 2 h and more where it takes well under one on a
quiet Mac. Each of them takes this lock around its heavy compile, one at a time:

  - parity-main (role `main`) leaves a "wants" marker as it arrives. A pull request's parity job (role `pr`) starts
    no compile while parity-main holds the lock or wants it. parity-main never waits for a pull request that only
    wants it, nor does a Nightly or release leg (role `build`).
  - A holder writes a heartbeat every minute, from a thread, so long cargo calls keep it fresh. A lock (or a marker)
    is stale only when its heartbeat is 15 minutes old, never after a fixed time: a real prebuild can take 2 h.
    A holder that dies stops beating, and the next waiter takes the lock over 15 minutes later.
  - Every wait is bounded (WAIT): then the job compiles anyway, with a line naming the holder.
  - Fail open: a folder that can't be made, read or written, or odd content in it, is said once, and the job
    compiles as if there were no lock. No job fails because of it.

Active only where RUNNER_ENVIRONMENT is self-hosted on macOS, which is the Mac mini's runners: the runner sets that
variable for every job, so no workflow can forget to switch it on, and the hosted, Linux and Windows legs and
developers' machines (no such variable) never touch the folder. The role `off` leaves a job out.

The folder, LOCK_DIR, is beside the runners (~/.actions-runners/<runner>/_work) and in none of their work folders.
Pull requests' jobs can write it, so what is read there is only data: no symlink is followed, files are size-capped
JSON, and a holder's description is cut to printable characters before it reaches a log.

Taking the lock is atomic: a holder's directory is written in full under a name of its own, then renamed to `held`,
which fails while `held` exists. A stale `held` is renamed to a tombstone named after its token, so of two waiters
that saw the same stale lock only the first moves it, and the second can't move the lock the first then took.
Releasing renames `held` away only when its token is the caller's. flock would release with a dead process for free,
but not with a hung one, and its lock would be invisible; the heartbeat gives both visibility and staleness.

Usage as a wrapper (game-build.yml, through tools/ci/compile-lock.sh):
  python3 tools/compile_lock.py build -- cargo build --release ...
"""

import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import threading
import time
import uuid

LOCK_DIR = None  # ~/.actions-runners/compile-lock, resolved by each Turn so a missing home fails open
# Each role's longest wait, in seconds, well inside its job's timeout (the PR's body has the arithmetic):
# parity-main 300 min, pull request parity 240 min with up to two prebuilds (the frame_identity step is a second
# tools/parity.py run), the Nightly's and release's Mac legs 60 min (they took up to 44 min).
WAIT = {"main": 20 * 60, "pr": 60 * 60, "build": 10 * 60}
STALE = 15 * 60
BEAT = 60
POLL = 15
# A tombstone stays until its heartbeat is this old, long after any waiter that saw it stale has acted.
TOMBSTONE = 4 * STALE
MAX_FILE = 4096
PRINTABLE = re.compile(r"[^ -~]")


class Odd(Exception):
    """What the folder holds is not what this module writes."""


def active(environ=None, platform=None):
    environ = os.environ if environ is None else environ
    return (platform or sys.platform) == "darwin" and environ.get("RUNNER_ENVIRONMENT") == "self-hosted"


def clean(text, limit=80):
    """text as one short printable line: nothing a lock file holds can start a workflow command or a new line."""
    return PRINTABLE.sub("?", str(text))[:limit]


def describe(info):
    if not isinstance(info, dict):
        return "an unknown job"
    return clean(f"{info.get('job') or '?'} ({info.get('role') or '?'}, run {info.get('run') or '?'}, "
                 f"runner {info.get('runner') or '?'})", 160)


def read_json(path):
    """The JSON object at path, None when there is no file; Odd for a symlink, a big or a malformed file."""
    try:
        status = os.lstat(path)
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(status.st_mode) or status.st_size > MAX_FILE:
        raise Odd(f"{path.name} is not a small regular file")
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.loads(handle.read(MAX_FILE))
    except FileNotFoundError:
        return None
    except ValueError as error:
        raise Odd(f"{path.name}: {error}")
    if not isinstance(data, dict) or not isinstance(data.get("token"), str) \
            or not isinstance(data.get("beat"), (int, float)):
        raise Odd(f"{path.name} has no token and heartbeat")
    return data


def write_json(path, data):
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}")
    with open(temporary, "x", encoding="utf-8") as handle:
        json.dump(data, handle)
    os.replace(temporary, path)


def remove_tree(path):
    """A directory this module made (files only), without following anything in it."""
    try:
        if os.path.islink(path) or not path.is_dir():
            path.unlink()
            return
        for entry in path.iterdir():
            entry.unlink()
        path.rmdir()
    except FileNotFoundError:
        pass


class Turn:
    """The lock as a context manager: entering waits for a turn (bounded), leaving releases it.

    clock and sleep are time.time and time.sleep unless a test passes its own; beat=None starts no heartbeat
    thread (a test calls heartbeat())."""

    def __init__(self, role, folder=None, environ=None, platform=None, clock=time.time, sleep=time.sleep,
                 say=None, beat=BEAT):
        environ = os.environ if environ is None else environ
        self.role, self.complained = role, False
        self.say_line = say or (lambda line: print(line, flush=True))
        self.on = active(environ, platform) and role != "off"
        try:
            self.folder = Path(folder or LOCK_DIR or Path.home() / ".actions-runners" / "compile-lock")
        except Exception as error:
            if self.on:
                self.fail_open(f"no folder for the lock ({error})")
            self.on, self.folder = False, None
        self.clock, self.sleep, self.beat_every = clock, sleep, beat
        self.token = uuid.uuid4().hex
        self.info = {"token": self.token, "role": role, "run": environ.get("GITHUB_RUN_ID", ""),
                     "job": " / ".join(part for part in (environ.get("GITHUB_WORKFLOW", ""),
                                                         environ.get("GITHUB_JOB", "")) if part),
                     "runner": environ.get("RUNNER_NAME", ""), "pid": os.getpid()}
        self.held = self.wanting = False
        self.taken_at = None
        self.stop = threading.Event()
        self.thread = None
        self.guard = threading.Lock()

    # Paths
    @property
    def held_dir(self):
        return self.folder / "held"

    @property
    def wants_file(self):
        return self.folder / f"wants-{self.token}.json"

    def say(self, line):
        try:
            self.say_line(f"  compile lock: {line}")
        except Exception:
            pass

    def fail_open(self, error):
        if not self.complained:
            self.complained = True
            self.say(f"{clean(error, 200)}; compiling without it")

    # Reading the folder
    def holder(self):
        """The holder's record, None when nobody holds the lock. Odd for anything else."""
        try:
            status = os.lstat(self.held_dir)
        except FileNotFoundError:
            return None
        if not stat.S_ISDIR(status.st_mode):
            raise Odd("`held` is not a directory")
        info = read_json(self.held_dir / "owner.json")
        if info is None:
            if not os.path.lexists(self.held_dir):
                return None  # released between the two reads
            raise Odd("`held` has no owner.json")
        return info

    def fresh(self, info):
        return self.clock() - info["beat"] < STALE

    def wanting_main(self):
        """Fresh wants markers of parity-main, other than this job's."""
        found = []
        for path in self.folder.glob("wants-*.json"):
            if path == self.wants_file:
                continue
            try:
                info = read_json(path)
            except Odd:
                continue  # one odd marker never blocks anybody
            if info is None:
                continue
            if not self.fresh(info):
                if self.clock() - info["beat"] > TOMBSTONE:
                    try:
                        path.unlink()
                    except OSError:
                        pass
                continue
            if info.get("role") == "main":
                found.append(info)
        return found

    def clear_tombstones(self):
        """Old tombstones, and what a job killed while taking the lock left (new-*)."""
        for path in [*self.folder.glob("gone-*"), *self.folder.glob("new-*")]:
            try:
                info = read_json(path / "owner.json") if path.is_dir() and not path.is_symlink() else None
            except Odd:
                info = None
            if info is None and path.name.startswith("new-"):
                continue  # perhaps a job between making it and writing its owner.json
            if info is None or self.clock() - info["beat"] > TOMBSTONE:
                remove_tree(path)

    # Writing
    def stamp(self):
        return dict(self.info, beat=self.clock())

    def want(self):
        write_json(self.wants_file, self.stamp())
        self.wanting = True

    def unwant(self):
        self.wanting = False
        try:
            self.wants_file.unlink()
        except FileNotFoundError:
            pass

    def try_take(self):
        """True when this job now holds the lock."""
        staging = self.folder / f"new-{self.token}"
        staging.mkdir()
        try:
            write_json(staging / "owner.json", self.stamp())
            try:
                os.rename(staging, self.held_dir)
            except OSError:
                if not os.path.lexists(self.held_dir):
                    raise
                return False
        finally:
            if staging.exists():
                remove_tree(staging)
        return True

    def take_over(self, stale):
        """Move a stale lock out of the way: to its own tombstone, which a second waiter can't rename onto."""
        tombstone = self.folder / f"gone-{clean(stale['token'], 64).replace('/', '?')}"
        try:
            os.rename(self.held_dir, tombstone)
        except OSError:
            return  # another waiter moved it first, or it was released
        try:
            moved = read_json(tombstone / "owner.json")
        except Odd:
            moved = None
        if not moved or moved["token"] != stale["token"]:
            # Moved a lock taken meanwhile: give it back (fails only if a third job holds the lock by now).
            try:
                os.rename(tombstone, self.held_dir)
            except OSError:
                pass
            return
        self.say(f"took over from {describe(stale)}, whose heartbeat stopped "
                 f"{(self.clock() - stale['beat']) / 60:.0f} min ago")

    def heartbeat(self):
        """Refresh this job's lock and wants marker, only while they are its own."""
        with self.guard:
            if self.held:
                try:
                    info = self.holder()
                except Odd:
                    info = None
                if info and info["token"] == self.token:
                    write_json(self.held_dir / "owner.json", self.stamp())
                else:
                    self.held = False
                    self.say("this job's lock was taken over (its heartbeat had stopped for 15 min)")
            if self.wanting:
                self.want()

    def beat_loop(self):
        while not self.stop.wait(self.beat_every):
            try:
                self.heartbeat()
            except Exception as error:
                self.fail_open(f"heartbeat: {error}")

    # The turn
    def acquire(self):
        """Wait for a turn, at most WAIT[role]; True when this job holds the lock."""
        if not self.on:
            return False
        if self.role not in WAIT:
            self.fail_open(f"unknown role {clean(self.role)}")
            return False
        try:
            return self.wait()
        except Exception as error:
            self.fail_open(f"{self.folder} can't be used ({error})")
            try:
                self.unwant()
            except Exception:
                pass
            return False

    def wait(self):
        self.folder.mkdir(parents=True, exist_ok=True)
        self.clear_tombstones()
        started = self.clock()
        self.want()
        waited_on, attempts = None, 0
        while True:
            attempts += 1
            if attempts > 50:  # renames that keep failing without anyone holding the lock
                raise Odd("the lock can't be taken")
            try:
                holder = self.holder()
            except Odd as error:
                self.fail_open(f"the lock folder holds something else ({error})")
                self.unwant()
                return False
            if holder and not self.fresh(holder):
                self.take_over(holder)
                continue
            blocker = holder
            if not blocker and self.role == "pr":
                blocker = next(iter(self.wanting_main()), None)
            if not blocker:
                if self.try_take():
                    self.held, self.taken_at = True, self.clock()
                    self.unwant()
                    waited = self.clock() - started
                    self.say(f"taken after waiting {waited / 60:.1f} min for {describe(waited_on)}" if waited_on
                             else "taken, no wait")
                    return True
                continue
            if waited_on is None or blocker.get("token") != waited_on.get("token"):
                verb = "holds it" if blocker is holder else "wants it first"
                self.say(f"waiting: {describe(blocker)} {verb}")
            waited_on = blocker
            attempts = 0
            if self.clock() - started >= WAIT[self.role]:
                verb = "holds it" if blocker is holder else "wants it first"
                self.say(f"waited {WAIT[self.role] // 60} min, the limit for this job; compiling anyway while "
                         f"{describe(blocker)} {verb}")
                if self.role != "main":
                    self.unwant()  # parity-main keeps its marker: no pull request starts beside it meanwhile
                return False
            self.sleep(POLL)
            self.want()

    def release(self):
        self.stop.set()
        if self.thread:
            self.thread.join(timeout=5)
        if not self.on:
            return
        try:
            with self.guard:
                if self.wanting:
                    self.unwant()
                if not self.held:
                    return
                self.held = False
                tombstone = self.folder / f"gone-{self.token}"
                info = self.holder()
                if not info or info["token"] != self.token:
                    self.say("not released: this job's lock was taken over")
                    return
                os.rename(self.held_dir, tombstone)
                moved = read_json(tombstone / "owner.json")
                if not moved or moved["token"] != self.token:
                    os.rename(tombstone, self.held_dir)  # someone else's, taken meanwhile: put it back
                    return
                remove_tree(tombstone)
                self.say(f"released after {(self.clock() - self.taken_at) / 60:.1f} min")
        except Exception as error:
            self.fail_open(f"release: {error}")

    def __enter__(self):
        self.acquire()
        if self.on and (self.held or self.wanting) and self.beat_every:
            try:
                self.thread = threading.Thread(target=self.beat_loop, name="compile-lock heartbeat", daemon=True)
                self.thread.start()
            except Exception as error:
                self.thread = None
                self.fail_open(f"no heartbeat thread ({error})")
        return self

    def __exit__(self, *exception):
        self.release()
        return False


def main(argv):
    """compile_lock.py <role> -- <command>: run the command in a turn; its exit status is this one's.
    compile_lock.py --probe: exit 0, so tools/ci/compile-lock.sh knows this Python can run the module."""
    if argv == ["--probe"]:
        return 0
    if len(argv) < 3 or argv[1] != "--":
        print("usage: compile_lock.py <main|pr|build|off> -- <command>...", file=sys.stderr)
        return 2
    role, command = argv[0], argv[2:]
    # A cancelled job's SIGTERM ends this process through the finally blocks, which release the lock.
    for name in ("SIGTERM", "SIGHUP"):
        if hasattr(signal, name):
            signal.signal(getattr(signal, name), lambda received, _frame: sys.exit(128 + received))
    try:
        turn = Turn(role)
    except Exception as error:  # never keeps the command from running
        print(f"  compile lock: {clean(error, 200)}; compiling without it", flush=True)
        turn = None
    if turn is None:
        return subprocess.call(command)
    with turn:
        process = subprocess.Popen(command)
        try:
            return process.wait()
        except BaseException:
            process.terminate()
            try:
                process.wait(timeout=30)
            except Exception:
                process.kill()
            raise


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
