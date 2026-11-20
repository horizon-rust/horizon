#!/usr/bin/env python3
"""Reuse a vanilla oracle's last results when nothing it reads has changed.

tools/parity.py puts `java` and `javac` links to this script first on PATH.
A run is identified by its command line, stdin when it is a file, the real
program and its -version text, and the environment (see IGNORED: oracles take
settings from it, such as VanillaChunkGenerationOracle's HORIZON_* variables).
After a successful run, the files it created or changed under
the paths its arguments name (classpath and sourcepath entries included) are
recorded as its outputs, with their size and mtime and its stdout and stderr;
every other file under those paths is recorded as an input with its content
hash. The next run of the same command replays the recorded output instead
of running the oracle if every input has the same content, no file was added
beside them, and the outputs are exactly as it left them. On a Rust-only
change a harness then pays only for its Rust side.

Files an oracle writes outside its named paths are not tracked. A harness
that depends on them fails on reuse; tools/parity.py then reruns it fresh and
stops reusing its oracle results.

A run that reads stdin from a pipe, or fails, is never cached.
PARITY_JAVA_CACHE=0 disables reuse (tools/parity.py --fresh).

Each call appends one JSON line saying what the shim did and why to
PARITY_SHIM_DECISIONS (tools/parity.py sums them up after a run). With
PARITY_SHIM_VERIFY=1 a replay is checked instead: the oracle runs anyway, and
if its stdout, stderr or outputs differ from the record's, the record is
removed and the call fails.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import threading
import time

try:
    from . import file_lock
except ImportError:
    import file_lock  # run as a script (the java and javac links): its own folder is on sys.path

CACHE = Path(os.environ.get("PARITY_SHIM_CACHE", ".local/parity-runner/oracle-cache"))
PATH_OPTIONS = {"-cp", "-classpath", "--class-path", "-sourcepath", "--source-path", "-p", "--module-path"}
# Variables left out of the key: they change between runs of the same call, and no oracle reads them (tools/parity.py's
# bookkeeping, a CI job's ids and the commits and token its parity workflows pass (parity.yml, parity-main.yml,
# parity-hosted.yml), an ssh or terminal session's state, the per-harness game folder). Every other variable, one
# never seen before included, is part of the key.
IGNORED_PREFIXES = ("PARITY_", "GITHUB_", "RUNNER_", "ACTIONS_", "BLACKSMITH_", "SSH_", "XDG_SESSION_")
IGNORED = {"PWD", "OLDPWD", "SHLVL", "_", "TERM", "TERM_SESSION_ID", "COLUMNS", "LINES", "INVOCATION_ID",
           "JOURNAL_STREAM", "SYSTEMD_EXEC_PID", "HORIZON_HOME", "BASE_SHA", "HEAD_SHA", "MAIN_SHA", "GH_TOKEN",
           "HARNESSES", "SHARD"}
# Variables naming a file or folder an oracle reads: what is in it is part of the key, as the name stays when it
# changes. HORIZON_ORDER's file is written from Horizon's own ticket order (tools/test_decorated_region_parity.py).
CONTENT_VARIABLES = {"HORIZON_ORDER", "HORIZON_DATAPACKS"}


# How deep the shim may run inside itself before it stops: it never needs to, since the real program
# gets PARITY_SHIM_REAL_PATH as its PATH, but a wrapper that puts the shim back would start one
# process after another until the machine runs out of memory.
MAX_DEPTH = 4


def real(name):
    """The first `name` on PARITY_SHIM_REAL_PATH that is not this script (a nested run's PATH holds the shim)."""
    me = Path(__file__).resolve()
    for folder in os.environ.get("PARITY_SHIM_REAL_PATH", "").split(os.pathsep):
        found = folder and shutil.which(name, path=folder)
        if found and Path(found).resolve() != me:
            return found
    sys.exit(f"parity shim: no real {name} on PARITY_SHIM_REAL_PATH")


def enter():
    """Count this run and give what it starts the real PATH, so a wrapper looking `java` up on PATH
    (a JDK selector, a build wrapper) finds the real one and not the shim again."""
    depth = int(os.environ.get("PARITY_SHIM_DEPTH") or 0)
    if depth >= MAX_DEPTH:
        sys.exit(f"parity shim: started inside itself {depth} times: what {os.path.basename(sys.argv[0])} runs "
                 "leads back to the shim (a wrapper that puts it back on PATH?)")
    os.environ["PARITY_SHIM_DEPTH"] = str(depth + 1)
    os.environ["PATH"] = os.environ.get("PARITY_SHIM_REAL_PATH", "")


# How long a shim waits for the hash database to be ready before it runs its oracle without the cache. Creating it
# takes milliseconds (a few writes and fsyncs); the rest is waiting for the shims ahead of it, each holding the lock
# for as long. Well past that, a holder is stuck (suspended, or on a disk that stalls): waiting longer gains nothing.
INIT_SECONDS = 15
# Setup's own waits on SQLite are short, so the deadline is checked between them: it runs over by about this at most.
SETUP_BUSY_SECONDS = 0.1
TABLE = "select 1 from sqlite_master where type = 'table' and name = 'hashes'"


class Uncached(Exception):
    """The hash database was not ready in time: this run goes to the real program, as with PARITY_JAVA_CACHE=0."""


def in_wal_mode(path):
    """The database exists and is in WAL mode: its header's read and write versions are 2."""
    try:
        with open(path, "rb") as stream:
            return stream.read(20)[18:20] == b"\x02\x02"
    except OSError:
        return False


def busy(error):
    return isinstance(error, sqlite3.OperationalError) and any(word in str(error) for word in ("locked", "busy"))


def prepare(path, deadline):
    """Make the hash database ready, in place: created if missing, switched to WAL, its table made; each step does
    nothing if already done. Shims starting together on an empty cache each switched it to WAL, and those that lost
    failed ("database is locked": 63 of 960 on Linux, 2026-11-20), failing their harness's first run. Now one does it
    under hashes.lock while the others wait for the lock. A shim from before this lock ignores it: meeting one
    mid-way, this retries until the deadline. The file is never replaced, as another process may have it open."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path.with_name("hashes.lock"), "a+b") as handle:
        while not file_lock.lock(handle, wait=False):
            if time.monotonic() >= deadline:
                raise Uncached("another process held hashes.lock throughout")
            time.sleep(0.02)
        def in_time():
            if time.monotonic() >= deadline:
                raise Uncached("the database stayed locked")
            return True

        while in_time():
            try:
                db = sqlite3.connect(path, timeout=SETUP_BUSY_SECONDS)
                try:
                    # Answers the mode it is in: still another while a connection keeps it from switching.
                    if db.execute("pragma journal_mode=wal").fetchone()[0] == "wal" and in_time():
                        db.execute("create table if not exists hashes (path text primary key, size int, mtime int, "
                                   "sha text)")
                        db.commit()
                        return
                finally:
                    db.close()
            except sqlite3.OperationalError as error:
                if not busy(error):
                    raise
            time.sleep(0.02)


class Hashes:
    """Content hashes keyed by path, size and mtime, shared by parallel harnesses."""

    def __init__(self):
        path = CACHE / "hashes.sqlite"
        # The clock starts before the warm probe, which keeps the usual busy timeout (no less patient than before
        # the lock): time it spent waiting counts against the setup that may follow.
        deadline = time.monotonic() + INIT_SECONDS
        # A warm cache: the header and one schema query, no lock.
        if in_wal_mode(path):
            self.db = sqlite3.connect(path, timeout=60)
            try:
                if self.db.execute(TABLE).fetchone():
                    return
            except sqlite3.OperationalError as error:
                if not busy(error):
                    raise
            self.db.close()
        prepare(path, deadline)
        self.db = sqlite3.connect(path, timeout=60)

    def file(self, path):
        stat = os.stat(path)
        row = self.db.execute("select size, mtime, sha from hashes where path = ?", (path,)).fetchone()
        if row and row[0] == stat.st_size and row[1] == stat.st_mtime_ns:
            return row[2]
        with open(path, "rb") as stream:
            sha = hashlib.file_digest(stream, "sha256").hexdigest()
        self.db.execute("insert or replace into hashes values (?, ?, ?, ?)", (path, stat.st_size, stat.st_mtime_ns, sha))
        self.db.commit()
        return sha


def tree(root):
    for directory, _, files in os.walk(root):
        for name in files:
            yield os.path.join(directory, name)


def named_paths(arguments):
    """Every filesystem path the arguments name, with classpath lists split."""
    paths = []
    for index, argument in enumerate(arguments):
        if index and arguments[index - 1] in PATH_OPTIONS:
            paths += [entry for entry in argument.split(os.pathsep) if entry]
        elif argument.startswith("-D") and "=" in argument:
            paths.append(argument.split("=", 1)[1])
        elif argument.startswith("-javaagent:"):
            # The jar is an input; its options, after "=", are in the command line already.
            paths.append(argument.removeprefix("-javaagent:").split("=", 1)[0])
        elif not argument.startswith("-"):
            paths.append(argument)
    return [os.path.abspath(path) for path in paths if path]


def snapshot(paths):
    state = {}
    for path in paths:
        files = tree(path) if os.path.isdir(path) else [path]
        for file in files:
            try:
                stat = os.stat(file)
                state[file] = [stat.st_size, stat.st_mtime_ns]
            except FileNotFoundError:
                state[file] = None
    return state


def stdin_identity(hashes):
    try:
        stat = os.fstat(0)
    except OSError:
        return "closed"
    if os.path.samestat(stat, os.stat(os.devnull)) if os.path.exists(os.devnull) else False:
        return "null"
    import stat as modes
    if modes.S_ISREG(stat.st_mode):
        # Hash the content without moving the shared offset (no /proc on macOS).
        digest, offset = hashlib.sha256(), 0
        while chunk := os.pread(0, 1 << 20, offset):
            digest.update(chunk)
            offset += len(chunk)
        return "f:" + digest.hexdigest()
    if modes.S_ISCHR(stat.st_mode):
        return "tty"
    return None  # a pipe or socket: its content is unknown, so never cache


def run_and_tee(command):
    """Run the real program, forwarding and recording its stdout and stderr."""
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    captured = {"stdout": bytearray(), "stderr": bytearray()}

    def pump(source, sink, buffer):
        for chunk in iter(lambda: source.read1(65536), b""):
            buffer += chunk
            sink.write(chunk)
            sink.flush()

    threads = [threading.Thread(target=pump, args=(process.stdout, sys.stdout.buffer, captured["stdout"])),
               threading.Thread(target=pump, args=(process.stderr, sys.stderr.buffer, captured["stderr"]))]
    for thread in threads:
        thread.start()
    status = process.wait()
    for thread in threads:
        thread.join()
    return status, captured


def files_under(paths):
    found = set()
    for path in paths:
        if os.path.isdir(path):
            found.update(tree(path))
        elif os.path.isfile(path):
            found.add(path)
    return found


def note_outputs(outputs):
    """Tell tools/parity.py which files are oracle outputs, which it keeps when clearing large outputs."""
    listing = os.environ.get("PARITY_SHIM_OUTPUTS")
    if listing:
        with open(listing, "a") as stream:
            stream.writelines(f"{path}\n" for path in outputs)


def stale(saved, paths, hashes):
    """Why the recorded run can't be replayed (a reason and its details), or None if it can: its inputs are
    unchanged, nothing was added beside them, and its outputs are as it left them."""
    outputs = saved["outputs"]
    found = files_under(paths) - set(outputs)
    if found != set(saved["inputs"]):
        return "input-set", {"added": examples(found - set(saved["inputs"])),
                             "removed": examples(set(saved["inputs"]) - found)}
    moved = [path for path, stamp in outputs.items() if snapshot([path]).get(path) != stamp]
    if moved:
        return "output-stamp", {"outputs": examples(moved)}
    changed = [path for path, sha in saved["inputs"].items() if hashes.file(path) != sha]
    if changed:
        return "input-content", {"inputs": examples(changed)}
    return None


def examples(paths, count=3):
    """How many paths, and the first few: enough to see what keeps a call from being replayed."""
    paths = sorted(paths)
    return {"count": len(paths), "first": [os.path.relpath(path) for path in paths[:count]]}


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def content_digest(path, hashes):
    path = os.path.abspath(path)
    if os.path.isfile(path):
        return hashes.file(path)
    if os.path.isdir(path):
        return digest([[os.path.relpath(file, path), hashes.file(file)] for file in sorted(tree(path))])
    return "missing"


def environment_digests(environment, hashes):
    """{name: digest of its value} for every variable in the key. Only digests are kept, never values."""
    kept = {}
    for name, value in environment.items():
        if name in IGNORED or name.startswith(IGNORED_PREFIXES):
            continue
        content = content_digest(value, hashes) if name in CONTENT_VARIABLES else ""
        kept[name] = hashlib.sha256(os.fsencode(value) + b"\0" + content.encode()).hexdigest()[:16]
    return kept


def version_digest(program, environment):
    """A digest of `program -version`'s output: another JDK under the same path, or a JDK updated in place, gives
    other results. Kept per real path, size, mtime and JAVA_HOME (a launcher that picks its JDK from it)."""
    path = os.path.realpath(program)
    stat = os.stat(path)
    cached = CACHE / "versions" / digest([path, stat.st_size, stat.st_mtime_ns, environment.get("JAVA_HOME")])
    try:
        return cached.read_text()
    except FileNotFoundError:
        pass
    result = subprocess.run([program, "-version"], stdin=subprocess.DEVNULL, capture_output=True, timeout=120)
    value = digest([result.returncode, result.stdout.hex(), result.stderr.hex()])
    cached.parent.mkdir(parents=True, exist_ok=True)
    temporary = cached.with_suffix(f".{os.getpid()}.tmp")
    temporary.write_text(value)
    temporary.replace(cached)
    return value


def unrecorded(index, variables, version):
    """Why no record matches this call's key: what changed since the call last ran with this command line."""
    try:
        last = json.loads(index.read_text())
    except FileNotFoundError:
        return "no-record", {}
    if last["version"] != version:
        return "java-version", {}
    if last["environment"] != variables:
        before, now = last["environment"], variables
        return "environment", {"variables": sorted(name for name in before.keys() | now.keys()
                                                   if before.get(name) != now.get(name))}
    return "no-record", {}  # recorded once, then removed (a failed verification)


def decide(name, reason, seconds=None, key=None, **details):
    """Append what the shim did with this call to PARITY_SHIM_DECISIONS: one whole line in one write to a file opened
    for appending, so parallel shims' lines never interleave."""
    path = os.environ.get("PARITY_SHIM_DECISIONS")
    if not path:
        return
    line = json.dumps({"harness": os.environ.get("PARITY_SHIM_HARNESS"), "program": name, "reason": reason,
                       "seconds": None if seconds is None else round(seconds, 3), "key": key and key[:12], **details})
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
    try:
        os.write(descriptor, (line + "\n").encode())
    finally:
        os.close(descriptor)


def mismatches(saved, key, status, captured, hashes, outputs):
    """How what the oracle just did differs from what replaying its record would have given: which stream or output,
    and sizes, never the content or a digest of it (oracle output is game data, and this goes to CI logs)."""
    if status != 0:
        return [f"exit status {status}, recorded 0"]
    found = []
    for stream in ("stdout", "stderr"):
        recorded, now = (CACHE / f"{key}.{stream}").read_bytes(), bytes(captured[stream])
        if recorded != now:
            found.append(f"{stream}: {len(now)} bytes, recorded {len(recorded)}")
    for path, sha in saved.get("output_sha", {}).items():
        if not os.path.isfile(path):
            found.append(f"output {os.path.relpath(path)}: missing")
        elif hashes.file(path) != sha:
            found.append(f"output {os.path.relpath(path)}: other content ({os.path.getsize(path)} bytes, recorded "
                         f"{saved['outputs'][path][0]})")
    # A replay would not have made these at all.
    found += [f"output {os.path.relpath(path)}: not recorded" for path in sorted(set(outputs) - set(saved["outputs"]))]
    return found


def main():
    name = os.path.basename(sys.argv[0])
    arguments = sys.argv[1:]
    program = real(name)
    environment = dict(os.environ)  # as the harness set it, before enter() points PATH at the real programs

    def passthrough(reason):
        decide(name, reason)
        os.execv(program, [program, *arguments])

    enter()
    if os.environ.get("PARITY_JAVA_CACHE") == "0":
        passthrough("passthrough-off")
    if "-version" in arguments or "--version" in arguments:
        passthrough("passthrough-version")
    try:
        hashes = Hashes()
    except Uncached as reason:
        print(f"parity shim: the oracle cache in {CACHE} was not ready within {INIT_SECONDS} s ({reason}): "
              f"{name} runs without it", file=sys.stderr, flush=True)
        passthrough("passthrough-uncached")
    stdin = stdin_identity(hashes)
    if stdin is None:
        passthrough("passthrough-pipe")
    paths = named_paths(arguments)
    command = {"program": os.path.realpath(program), "arguments": arguments, "cwd": os.getcwd(), "stdin": stdin}
    variables = environment_digests(environment, hashes)
    version = version_digest(program, environment)
    key = digest({"command": command, "environment": variables, "version": version})
    record = CACHE / f"{key}.json"
    # The last key this command line ran under, to say why a call found no record.
    index = CACHE / "calls" / f"{digest(command)}.json"
    saved = json.loads(record.read_text()) if record.exists() else None
    if saved is None:
        reason, details = unrecorded(index, variables, version)
    else:
        reason, details = stale(saved, paths, hashes) or (None, {})
    verify = saved is not None and reason is None and os.environ.get("PARITY_SHIM_VERIFY") == "1"
    if saved is not None and reason is None and not verify:
        note_outputs(saved["outputs"])
        sys.stdout.buffer.write((CACHE / f"{key}.stdout").read_bytes())
        sys.stderr.buffer.write((CACHE / f"{key}.stderr").read_bytes())
        decide(name, "hit", saved.get("seconds"), key)
        return 0
    before = snapshot(paths)
    inputs = {path: hashes.file(path) for path in files_under(paths)}
    started = time.monotonic()
    status, captured = run_and_tee([program, *arguments])
    seconds = time.monotonic() - started
    after = snapshot(paths)
    outputs = {path: stamp for path, stamp in after.items() if stamp is not None and before.get(path) != stamp}
    if verify:
        found = mismatches(saved, key, status, captured, hashes, outputs)
        if found:
            for path in (record, CACHE / f"{key}.stdout", CACHE / f"{key}.stderr"):
                path.unlink(missing_ok=True)
            decide(name, "verify-mismatch", seconds, key, differs=found)
            print(f"parity shim: VERIFY FAILED in {os.environ.get('PARITY_SHIM_HARNESS') or 'a harness'}: "
                  f"{name} {' '.join(arguments)[:500]} (key {key[:12]}) gave other results than replaying its record "
                  f"would have: {'; '.join(found)}. The record is removed.", file=sys.stderr, flush=True)
            return status or 1
        reason = "verify-ok"
        if "output_sha" not in saved:
            details = {"note": "stdout and stderr only: the record predates output hashes"}
    if status != 0:
        decide(name, "passthrough-failed", seconds, key, miss=reason)
        return status
    for path in outputs:
        inputs.pop(path, None)
    (CACHE / f"{key}.stdout").write_bytes(captured["stdout"])
    (CACHE / f"{key}.stderr").write_bytes(captured["stderr"])
    temporary = record.with_suffix(f".{os.getpid()}.tmp")
    temporary.write_text(json.dumps({"command": command, "environment": variables, "version": version,
                                     "inputs": inputs, "outputs": outputs, "seconds": round(seconds, 3),
                                     "output_sha": {path: hashes.file(path) for path in outputs}}))
    temporary.replace(record)
    index.parent.mkdir(exist_ok=True)
    temporary = index.with_suffix(f".{os.getpid()}.tmp")
    temporary.write_text(json.dumps({"key": key, "environment": variables, "version": version}))
    temporary.replace(index)
    note_outputs(outputs)
    decide(name, reason, seconds, key, **details)
    return 0


if __name__ == "__main__":
    sys.exit(main())
