#!/usr/bin/env python3
"""Quick live region benchmark: identical spread-out players and fixed AI mobs.

Build horizon-client and horizon-bots in profile fast, then:
  python3 tools/bench_region_ticks.py --bin .local/parity-target/fast
Each mode gets a new flat seed-12345 world and scratch user directories. This
compares the retained serial execution with region execution in the same binary.
--before-bin can instead name a separately built pre-change engine directory.
No assets, generated worlds or player data belong in the resulting PR.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile
import threading
import time

try:
    from . import lab, server_bench as bench
except ImportError:
    import lab
    import server_bench as bench


def percentile(values, fraction):
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, math.ceil(len(ordered) * fraction) - 1)], 3)


def summary(values):
    return {name: percentile(values, fraction) for name, fraction in
            [('median', .5), ('p95', .95), ('p99', .99)]} if values else None


def wait_until(predicate, timeout, message):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.2)
    raise RuntimeError(message)


def command_barrier(server, commands):
    start = server.count()
    for command in commands:
        server.console(command)
    marker = f'region-bench-barrier-{start}'
    server.console('say ' + marker)
    wait_until(lambda: any(marker in line for line in server.since(start)), 300,
               'server did not finish setup commands')


def game_time(server):
    start = server.count()
    server.console('time query gametime')
    found = []
    def reply():
        for line in server.since(start):
            if match := re.search(r'The (?:game )?time is (\d+)', line):
                found.append(int(match[1]))
                return True
        return False
    wait_until(reply, 30, 'game-time query did not reply')
    return found[0]


def run(mode, args, out):
    directory = out / mode
    directory.mkdir()  # Never overwrite an existing world's directory.
    port = lab.free_port()
    properties = dict(bench.PROPERTIES, **{
        'server-port': str(port), 'server-ip': '127.0.0.1',
        'view-distance': '2', 'simulation-distance': '2',
        'level-type': 'minecraft\\:flat', 'generate-structures': 'false',
        'gamemode': 'survival', 'difficulty': 'normal', 'allow-flight': 'true',
    })
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in properties.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    homes = directory / 'scratch'
    homes.mkdir()
    environment = {
        'HORIZON_HOME': str(homes / 'game'),
        'HORIZON_LAUNCHER_HOME': str(homes / 'launcher'),
        'XDG_DATA_HOME': str(homes / 'data'),
        'XDG_CONFIG_HOME': str(homes / 'config'),
        'HORIZON_LAUNCHER_TRASH': str(homes / 'trash'),
        'HORIZON_LAUNCHER_CLIPBOARD': str(homes / 'clipboard'),
        'HORIZON_LAUNCHER_DESKTOP': str(homes / 'desktop'),
        'HORIZON_SIMULATION_MODE': 'legacy' if mode == 'control' else mode,
        'HORIZON_SIMULATION_WORKERS': str(args.workers),
        'HORIZON_REGION_METRICS': '1',
        'HORIZON_SPIKES': '0',
    }
    previous = {key: os.environ.get(key) for key in environment}
    os.environ.update(environment)
    binary_dir = args.before_bin if mode == 'legacy' and args.before_bin else args.bin
    server = None
    bots = None
    try:
        server = bench.Server([binary_dir / 'horizon-client', '--resources', lab.RESOURCES,
                               '--dedicated', '--nogui'], directory, out / f'{mode}.log')
        server.wait_for(bench.DONE, 600)
        command_barrier(server, [
            'gamerule spawn_mobs false', 'gamerule spawn_patrols false',
            'gamerule advance_time false', 'gamerule advance_weather false',
            'gamerule random_tick_speed 0', 'gamerule max_entity_cramming 0',
            'gamerule respawn_radius 0', 'time set midnight', 'weather clear', 'save-off', 'tick freeze',
        ])
        bot_lines = []
        bot_errors = (out / f'{mode}-bots.stderr').open('w')
        bots = subprocess.Popen([
            str(args.bin / 'horizon-bots'), '--server', f'127.0.0.1:{port}',
            '--bots', str(args.players), '--behaviour', 'stand', '--name', 'RegionBot',
            '--join-every', '.1', '--seconds', '1200', '--view-distance', '2',
            '--ground', '--seed', '12345', '--report', '-',
        ], stdout=subprocess.PIPE, stderr=bot_errors, text=True, env=lab.game_env())
        def read_bots():
            for line in bots.stdout:
                try:
                    value = json.loads(line)
                    if 'online' in value:
                        bot_lines.append(value)
                except ValueError:
                    pass
        reader = threading.Thread(target=read_bots, daemon=True)
        reader.start()
        wait_until(lambda: bot_lines and bot_lines[-1]['online'] == args.players,
                   300, f'{args.players} bots did not join')
        commands = ['effect give @a minecraft:resistance infinite 4 true']
        side = math.ceil(math.sqrt(args.players))
        for player in range(args.players):
            commands.append(f'tp RegionBot{player} {1024 + player % side * args.spacing} -60 '
                            f'{1024 + player // side * args.spacing}')
        command_barrier(server, commands)
        time.sleep(args.settle)
        # Each wave inserts one mob at every player, preserving interleaved
        # global entity order while the regions have their own local order.
        commands = []
        for mob in range(args.mobs_per_player):
            dx, dz = (mob % 10) * 2 - 9, (mob // 10) * 2 - 6
            species = args.species.split(',')
            commands.append(f'execute as @a at @s run summon {species[mob % len(species)]} ~{dx} ~ ~{dz} '
                            '{Tags:["region_bench"],PersistenceRequired:1b,Invulnerable:1b,Silent:1b}')
        command_barrier(server, commands)
        # Start both engines at the same simulation age even when their TPS
        # differs. Setup and spawning happened while the world was frozen.
        first_tick = game_time(server)
        warmup_ticks = args.warmup * 20
        server.console(f'tick step {warmup_ticks}')
        wait_until(lambda: game_time(server) == first_tick + warmup_ticks,
                   max(120, args.warmup * 20), 'fixed-tick warmup did not finish')
        # Reports contain exact whole-tick samples. The regular span log also
        # exposes each mob phase, instead of using snapshot-only timings.
        start_line = server.count()
        server.console(f'horizon-report {args.seconds}')
        server.console('tick unfreeze')
        wait_until(lambda: any('Recording a performance report' in line
                              for line in server.since(start_line)), 30, 'report did not start')
        start = time.monotonic()
        last_time = start
        _, last_cpu = bench.tree_usage(server.process.pid)
        load_start = os.getloadavg()
        cpu, rss = [], []
        while time.monotonic() - start < args.seconds:
            time.sleep(1)
            now = time.monotonic()
            memory, used = bench.tree_usage(server.process.pid)
            cpu.append((used - last_cpu) * 100 / (now - last_time))
            rss.append(memory / 1024 / 1024)
            last_time, last_cpu = now, used
            if not bot_lines or bot_lines[-1]['online'] != args.players:
                raise RuntimeError('a player disconnected during measurement')
        wait_until(lambda: bool(list((directory / 'reports').glob('perf-*.json'))),
                   30, 'performance report was not written')
        report = json.loads(next((directory / 'reports').glob('perf-*.json')).read_text())
        mob_count = sum(level['mobs'] for level in report['levels'])
        if report['players'] != args.players or mob_count != args.players * args.mobs_per_player:
            raise RuntimeError(f'workload changed: {report["players"]} players, {mob_count} mobs')
        lines = server.since(start_line)
        timed_lines = [line for line in lines if 'Horizon: slow tick ' in line][:report['ticks']['count']]
        mob_ms = [float(match[1]) for line in timed_lines
                  if (match := re.search(r'\btick_creature_entities ([0-9.]+)', line))]
        region_samples = [tuple(map(int, match.groups())) for line in lines
                          if (match := re.search(r'regions=(\d+) predicted=(\d+) committed=(\d+) replayed=(\d+) rng_conflicts=(\d+) read_conflicts=(\d+)', line))]
        commit_rounds = [match.groups() for line in lines
                         if (match := re.search(r'clean_rounds=(\d+) fallback_rounds=(\d+) fallback=(?:Some\()?(\w+)', line))]
        fallback_reasons = {}
        for _, fallback, reason in commit_rounds:
            if fallback == '1':
                fallback_reasons[reason] = fallback_reasons.get(reason, 0) + 1
        preparation_samples = [
            (match[1] == 'true', int(match[2])) for line in lines
            if (match := re.search(r'streaming range preparation: accepted=(true|false) coordinates=(\d+)', line))
        ]
        result = {
            'mode': mode, 'players': args.players, 'mobs': mob_count, 'warmup_ticks': warmup_ticks,
            'ticks': report['ticks'], 'mob_ms': summary(mob_ms),
            'load_average_start': load_start, 'load_average_end': os.getloadavg(),
            'cpu_percent': round(statistics.fmean(cpu), 2),
            'rss_mib_mean': round(statistics.fmean(rss), 2), 'rss_mib_max': round(max(rss), 2),
            'region_mean': dict(zip(('regions', 'predicted', 'committed', 'replayed', 'rng_conflicts', 'read_conflicts'),
                                    map(lambda values: round(statistics.fmean(values), 2),
                                        zip(*region_samples)))) if region_samples else None,
            'range_preparation': {
                'log_samples': len(preparation_samples),
                'accepted': sum(accepted for accepted, _ in preparation_samples),
                'discarded': sum(not accepted for accepted, _ in preparation_samples),
                'coordinates_max': max(count for _, count in preparation_samples),
            } if preparation_samples else None,
            'queues_at_end': report.get('queues_at_end'),
            'network_compression': report.get('network_compression'),
            'commit_rounds': {'clean': sum(int(clean) for clean, _, _ in commit_rounds),
                              'fallback': sum(int(fallback) for _, fallback, _ in commit_rounds),
                              'fallback_reasons': fallback_reasons},
            'mob_span_samples': len(mob_ms), 'spans': report['spans'],
        }
        print(json.dumps({key: value for key, value in result.items() if key != 'spans'}), flush=True)
        return result
    finally:
        if bots is not None:
            bots.terminate()
            try:
                bots.wait(timeout=10)
            except subprocess.TimeoutExpired:
                bots.kill()
                bots.wait()
            bot_errors.close()
        if server is not None:
            server.stop()
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', type=Path, required=True)
    parser.add_argument('--before-bin', type=Path)
    parser.add_argument('--players', type=int, default=100)
    parser.add_argument('--mobs-per-player', type=int, default=70)
    parser.add_argument('--species', default='zombie', help='comma-separated: a mixed population takes turns')
    parser.add_argument('--spacing', type=int, default=512)
    parser.add_argument('--workers', type=int, default=8)
    parser.add_argument('--settle', type=int, default=20)
    parser.add_argument('--warmup', type=int, default=15, help='simulation seconds, stepped as exactly N*20 frozen ticks')
    parser.add_argument('--seconds', type=int, default=20)
    parser.add_argument('--modes', default='legacy,parallel')
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    args.bin = args.bin.resolve()
    if args.before_bin:
        args.before_bin = args.before_bin.resolve()
    out = args.out.resolve() if args.out else Path(tempfile.mkdtemp(prefix='horizon-region-bench-'))
    out.mkdir(parents=True, exist_ok=True)
    print(f'Benchmark artifacts: {out}', flush=True)
    modes = args.modes.split(',')
    if any(mode not in ('legacy', 'control', 'serial', 'parallel', 'verify') for mode in modes):
        parser.error('--modes supports legacy,control,serial,parallel,verify')
    results = [run(mode, args, out) for mode in modes]
    (out / 'results.json').write_text(json.dumps({
        'workload': vars(args) | {'bin': str(args.bin), 'before_bin': str(args.before_bin), 'out': str(out)},
        'results': results,
    }, indent=2, default=str) + '\n')


if __name__ == '__main__':
    main()
