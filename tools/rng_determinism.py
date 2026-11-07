#!/usr/bin/env python3
"""Make unseeded randomness repeatable within Horizon (and optionally a vanilla oracle run).

Vanilla's `RandomSource.create()` (and `RandomSupport.generateUniqueSeed()`) is the only
nondeterministic random source in the game: `(uniquifier * STEP) ^ System.nanoTime()`. Horizon
mirrors it in `horizon_core::random::unique_seed()`. This tool can replace the clock term with one
master seed on the Horizon side (and, optionally, in a vanilla oracle run):

  * Java: a `-javaagent` (`mods/horizon/.../parity/rng/Agent.java`) patches
    `RandomSupport.generateUniqueSeed()` and `RandomSource.createThreadLocalInstance()`.
  * Horizon: set `HORIZON_RNG_SEED`; `unique_seed()` then returns a repeatable sequence. Setting
    `HORIZON_STILL=1` pins it to seed 0, as it already pins visual time.

Same sequence, not necessarily same order: two implementations still have to create and consume
streams in the same order to match. `--trace` logs the creation order on each side
(`create INDEX SEED SITE`) so a divergence points at the first stream that differs. For effects
whose order legitimately differs (particles), derive a named stream with
`horizon_core::random::pinned_seed("name")` /
`Deterministic.streamSeed("name")` instead of taking the next shared value.

Usage:
  python3 tools/rng_determinism.py verify [--seed N] [--count N] [--trace] [--fresh]
  python3 tools/rng_determinism.py agent                  # build and print the agent jar
  python3 tools/rng_determinism.py env [--seed N] [--trace PATH]     # shell exports for Horizon
  python3 tools/rng_determinism.py java [--seed N] [--trace PATH] -- java -cp ... Main

`verify` is the parity harness (`tools/test_rng_determinism_parity.py`): it runs
RngDeterminismOracle and `client_probe rng_determinism` on one seed and requires identical
output, then compares the two traces when --trace is given.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

try:
    from . import lab, oracle_harness
    from . import test_rust_parity as base
except ImportError:
    import lab
    import oracle_harness
    import test_rust_parity as base

ROOT = lab.ROOT
AGENT_DIR = lab.LOCAL / 'rng-determinism'
AGENT_JAR = AGENT_DIR / 'rng-agent.jar'
AGENT_SOURCES = sorted((ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/rng').glob('*.java'))
AGENT_CLASS = 'dev.horizonlab.parity.rng.Agent'
DEFAULT_COUNT = 64


def seed_text(value):
    """A seed from decimal or `0x` text, printed the way both sides parse it."""
    if isinstance(value, int):
        return str(value)
    return str(int(str(value).replace('_', ''), 0))


def ensure_agent(fresh=False):
    """Compile the agent once per source/JDK change; return its jar."""
    if not AGENT_SOURCES:
        raise SystemExit('the determinism agent sources are missing under mods/horizon/.../parity/rng')
    lock, _ = lab.config()
    java_major = lock['minecraft']['java_major']
    digest = hashlib.sha256()
    digest.update(str(java_major).encode())
    digest.update(subprocess.check_output(['java', '--version'], text=True).encode())
    for source in AGENT_SOURCES:
        digest.update(source.name.encode())
        digest.update(source.read_bytes())
    fingerprint = digest.hexdigest()
    stamp = AGENT_DIR / 'agent.json'
    if not fresh and AGENT_JAR.is_file() and stamp.is_file():
        try:
            if json.loads(stamp.read_text(encoding='utf-8'))['sha256'] == fingerprint:
                return AGENT_JAR
        except (ValueError, KeyError):
            pass
    AGENT_DIR.mkdir(parents=True, exist_ok=True)
    classes = AGENT_DIR / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    subprocess.run(['javac', '--release', str(java_major), '-proc:none', '-d', classes,
                    *map(str, AGENT_SOURCES)], cwd=ROOT, check=True)
    manifest = AGENT_DIR / 'MANIFEST.MF'
    manifest.write_text(
        f'Premain-Class: {AGENT_CLASS}\nAgent-Class: {AGENT_CLASS}\n'
        'Can-Retransform-Classes: false\n', encoding='utf-8')
    AGENT_JAR.unlink(missing_ok=True)
    subprocess.run(['jar', '--create', '--file', AGENT_JAR, '--manifest', manifest,
                    '-C', classes, '.'], cwd=ROOT, check=True)
    stamp.write_text(json.dumps({'sha256': fingerprint, 'java_major': java_major}, indent=1),
                     encoding='utf-8')
    return AGENT_JAR


def jvm_options(seed, trace=None):
    """The `-javaagent` and seed options for a Java oracle or the game."""
    options = [f'-javaagent:{ensure_agent()}', f'-Dhorizon.rng.seed={seed_text(seed)}']
    if trace:
        options.append(f'-Dhorizon.rng.trace={trace}')
    return options


def environ(seed, trace=None):
    """The environment for a Horizon process with the same seed."""
    env = {'HORIZON_RNG_SEED': seed_text(seed)}
    if trace:
        env['HORIZON_RNG_TRACE'] = str(trace)
    return env


def make_input(count):
    def build(directory):
        path = directory / 'inputs.txt'
        path.write_text(f'{count}\n', encoding='utf-8')
        return path, count
    return build


def trace_events(path):
    """The comparable part of a trace: `(index, seed)` per created stream."""
    events = []
    if not Path(path).is_file():
        return events
    for line in Path(path).read_text(encoding='utf-8', errors='replace').splitlines():
        fields = line.split()
        if len(fields) >= 3 and fields[0] == 'create':
            events.append((int(fields[1]), int(fields[2])))
    return events


def compare_traces(java_trace, rust_trace):
    """Raise with the first stream whose index or seed differs; return the event count."""
    java, rust = trace_events(java_trace), trace_events(rust_trace)
    if len(java) != len(rust):
        raise AssertionError(
            f'RNG traces differ in length: Java created {len(java)} unseeded streams, Horizon {len(rust)}')
    for (ji, js), (ri, rs) in zip(java, rust):
        if (ji, js) != (ri, rs):
            raise AssertionError(
                f'RNG trace diverges at stream {ji}: Java seed {js}, Horizon seed {rs}')
    return len(java)


def verify(seed=0, count=DEFAULT_COUNT, fresh=False, trace=False):
    """Run the Java oracle and the Rust probe on a fixed seed and require identical output."""
    ensure_agent(fresh=fresh)
    build = lab.LOCAL / 'rng-determinism' / 'parity'
    build.mkdir(parents=True, exist_ok=True)
    java_trace = build / 'trace-java.txt'
    rust_trace = build / 'trace-rust.txt'
    for path in (java_trace, rust_trace):
        path.unlink(missing_ok=True)
    jvm = jvm_options(seed, java_trace if trace else None)
    env = environ(seed, rust_trace if trace else None)
    difference = oracle_harness.differential(
        'rng-determinism', 'RngDeterminismOracle', 'rng_determinism_probe', make_input(count),
        scope='unseeded random sources under one master seed',
        sources=[*AGENT_SOURCES, Path(__file__).resolve()],
        jvm=jvm, rust_env=env)
    if trace:
        streams = compare_traces(java_trace, rust_trace)
        print(f'{streams:,} unseeded RNG streams were created in the same order', flush=True)
    return difference


def shell_exports(seed, trace=None):
    lines = [f'export HORIZON_RNG_SEED={seed_text(seed)}']
    if trace:
        lines.append(f'export HORIZON_RNG_TRACE={trace}')
    return '\n'.join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command')

    verify_parser = sub.add_parser('verify', help='run the Java oracle and Rust probe on one seed')
    verify_parser.add_argument('--seed', default='0', help='master seed (decimal or 0x hex)')
    verify_parser.add_argument('--count', type=int, default=DEFAULT_COUNT, help='streams to create')
    verify_parser.add_argument('--trace', action='store_true', help='also compare creation order')
    verify_parser.add_argument('--fresh', action='store_true', help='rebuild the agent')

    agent_parser = sub.add_parser('agent', help='build (if needed) and print the agent jar')
    agent_parser.add_argument('--fresh', action='store_true', help='rebuild the agent')

    env_parser = sub.add_parser('env', help='print shell exports for Horizon')
    env_parser.add_argument('--seed', default='0', help='master seed (decimal or 0x hex)')
    env_parser.add_argument('--trace', help='write the creation trace to this file')

    java_parser = sub.add_parser('java', help='run a Java command with the determinism agent')
    java_parser.add_argument('--seed', default='0', help='master seed (decimal or 0x hex)')
    java_parser.add_argument('--trace', help='write the creation trace to this file')
    java_parser.add_argument('command', nargs=argparse.REMAINDER, help='the java command')

    args = parser.parse_args(argv)
    if args.command == 'verify':
        verify(args.seed, args.count, args.fresh, args.trace)
    elif args.command == 'agent':
        print(ensure_agent(fresh=args.fresh))
    elif args.command == 'env':
        print(shell_exports(args.seed, args.trace))
    elif args.command == 'java':
        command = list(args.command)
        if command and command[0] == '--':
            command.pop(0)
        if not command:
            parser.error('java needs a command after --')
        program = shutil.which(command[0])
        if program is None:
            parser.error(f'{command[0]} is not on PATH')
        options = jvm_options(args.seed, args.trace)
        os.execv(program, [program, *options, *command[1:]])
    else:
        parser.print_help()
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
