#!/usr/bin/env python3
"""A guest's fireball hits the local host and credits collateral kills to its shooter.

The pinned server supplies the reference with three real players. Horizon uses an
in-frame survival host and two wire clients. Linux needs a display or the usual
vscreen environment. Each run keeps its world and transcripts in --output.
"""
import argparse
import contextlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import uuid

try:
    from . import lab
    from .server_fixture import SaveClient, compile_client, isolated_env as game_env, offline_uuid, wait_for
    from .test_locator_bar_parity import generate_world, server_dir
    from .test_projectile_persistence_parity import Console, join, running
    from .test_world_reopen_parity import game_env as display_env
except ImportError:
    import lab
    from server_fixture import SaveClient, compile_client, isolated_env as game_env, offline_uuid, wait_for
    from test_locator_bar_parity import generate_world, server_dir
    from test_projectile_persistence_parity import Console, join, running
    from test_world_reopen_parity import game_env as display_env


SHOOTER, HOST, OTHER = 'ImpactGuest', 'Player', 'ImpactOther'
SHOT = '@e[type=minecraft:fireball,limit=1]'
PIG = '@e[type=minecraft:pig,tag=host-impact,limit=1]'
OBJECTIVE = 'impact_kills'


class InFrameHost:
    def __init__(self, binary, world, directory):
        with contextlib.ExitStack() as setup:
            endpoint = setup.enter_context(tempfile.TemporaryDirectory(prefix='hz-impact-'))
            self.socket = Path(endpoint) / 'c.sock'
            self.work = directory / 'host'
            shutil.copytree(world, self.work / 'saves/HostImpact')
            (self.work / 'saves/HostImpact/session.lock').unlink(missing_ok=True)
            (self.work / 'options.txt').write_text('renderDistance:3\nsimulationDistance:3\nmaxFps:60\n')
            (self.work / 'horizon-graphics.txt').write_text('workers:4\nlodDistance:0\n')
            env = game_env()
            env['HORIZON_SERVER_THREAD'] = '0'
            if sys.platform != 'darwin' and not env.get('DISPLAY'):
                display, _ = display_env()
                for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY'):
                    if key in display:
                        env[key] = display[key]
            self.log = setup.enter_context((directory / 'host.log').open('w'))
            self.control_log = setup.enter_context((directory / 'host-control.jsonl').open('w'))
            self.process = subprocess.Popen(
                [str(binary), '--resources', str(lab.RESOURCES), '--world', 'HostImpact',
                 '--creative', '--in-frame', '--offscreen', '--render-distance', '3',
                 '--width', '640', '--height', '360', '--name', HOST,
                 '--control-socket', str(self.socket)],
                cwd=self.work, env=env, stdout=self.log, stderr=subprocess.STDOUT)
            self.resources = setup.pop_all()

    def raw(self, command):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(60)
            connection.connect(str(self.socket))
            connection.sendall((command + '\n').encode())
            with connection.makefile('rb') as stream:
                response = json.loads(stream.readline())
        self.control_log.write(json.dumps({'command': command, 'reply': response}) + '\n')
        self.control_log.flush()
        assert 'error' not in response, (command, response)
        return response

    @property
    def lines(self):
        return self.raw('messages 200')['chat']

    def start(self):
        wait_for(lambda: self.socket.exists() or self.process.poll() is not None,
                 'host control socket did not open', timeout=180)
        assert self.process.poll() is None, 'host exited before opening its control socket'
        wait_for(lambda: (state := self.raw('state')).get('screen') == 'playing' and state.get('player'),
                 'host did not enter its world', timeout=180)
        return self.raw('lan open 0 commands')['port']

    def close(self):
        try:
            if self.process.poll() is None:
                try:
                    self.raw('quit')
                except (OSError, ValueError, AssertionError):
                    pass
                self.process.wait(timeout=45)
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=10)
            self.resources.close()


class HostConsole(Console):
    def __init__(self, host):
        self.host = host
        self.dimension = None

    def __call__(self, command):
        # The socket returns chat newest first. Each query below emits one line.
        return '\n'.join(self.host.raw('command ' + command)['chat'][:1])


def health(console, name):
    response = console(f'data get entity {name} Health')
    found = re.search(r'\b' + re.escape(name) + r' has the following entity data: ([0-9.]+)f', response)
    assert found, (name, response)
    return float(found[1])


def score(console, name):
    response = console(f'scoreboard players get {name} {OBJECTIVE}')
    found = re.search(r'\b' + re.escape(name) + r' has (-?\d+) \[' + re.escape(OBJECTIVE) + r'\]', response)
    assert found, response
    return int(found[1])


def wire_uuid(transcript, name):
    profiles = re.findall(r'^login minecraft:login_finished ([0-9a-f-]+) ' + re.escape(name) + r'\b',
                          transcript, re.MULTILINE)
    assert profiles == [offline_uuid(name)], (name, profiles)
    return profiles[0]


def impact(console, server, shooter, other, directory):
    names = [SHOOTER, HOST, OTHER]
    for command in [
        'tick freeze', 'gamerule spawn_mobs false', 'gamerule natural_health_regeneration false',
        'difficulty normal', 'time set noon',
        'fill 2 99 2 24 99 24 minecraft:stone', 'fill 2 100 2 24 110 24 minecraft:air',
        *[f'gamemode survival {name}' for name in names],
        f'tp {HOST} 8.5 100 16.5 180 0', f'tp {SHOOTER} 8.5 100 8.5 0 0',
        f'tp {OTHER} 20.5 100 20.5 180 0',
        f'effect give {HOST} minecraft:resistance infinite 2 true',
        f'scoreboard objectives add {OBJECTIVE} minecraft.killed:minecraft.pig',
        *[f'scoreboard players set {name} {OBJECTIVE} 0' for name in names],
    ]:
        console(command)
    survival = console('execute if entity @a[gamemode=survival]')
    assert re.search(r'Test passed\. Count: 3\b', survival), survival
    before = [health(console, name) for name in names]
    assert before == [20.0, 20.0, 20.0], before
    if isinstance(console, HostConsole):
        assert not server.raw('status')['creative']
        server.raw('hurtlog')
    console('summon minecraft:pig 10.5 100 16.5 {NoAI:1b,Health:1.0f,Tags:["host-impact"]}')
    console('summon minecraft:fireball 8.5 101.6 10.5 {ExplosionPower:2b}')
    entities = shooter.release.with_name(shooter.release.name + '.entities')
    wait_for(lambda: entities.exists() and 'minecraft:fireball' in entities.read_text(),
             'shooter did not receive the fireball')
    shooter.commands(server, ['@attack minecraft:fireball'])
    owner = console.entity_uuid(SHOT, 'Owner')
    expected = list(struct.unpack('>4i', uuid.UUID(offline_uuid(SHOOTER)).bytes))
    assert owner == expected, ('wire deflection did not set the shooter UUID', owner)
    position, motion = console.vector(SHOT, 'Pos'), console.vector(SHOT, 'Motion')
    assert position == [8.5, 101.6, 10.5], position
    assert motion[:2] == [0.0, 0.0] and motion[2] > 0, motion
    pig = console.vector(PIG, 'Pos')
    types = json.loads((lab.RESOURCES / 'entity-types.json').read_text())
    assert pig[0] - types['minecraft:pig']['width'] / 2 > position[0] + 0.3, ('pig is on the ray', pig)
    assert math.dist(pig, [8.5, 101.6, 15.9]) < 4, ('pig is outside the blast', pig)
    observation = {'owner_uuid': owner, 'shot_uuid': console.entity_uuid(SHOT),
                   'position': position, 'motion': motion, 'collateral': pig, 'before': before, 'flight': []}
    for tick in range(1, 15):
        console.step(1)
        remaining = console.count('minecraft:fireball')
        event = {'tick': tick, 'remaining': remaining}
        if remaining:
            event['position'] = console.vector(SHOT, 'Pos')
        observation['flight'].append(event)
        if not remaining:
            break
    assert not remaining, ('fireball never reached the host', observation)
    after = [health(console, name) for name in names]
    observation['after'] = after
    if isinstance(console, HostConsole):
        observation['hurtlog'] = server.raw('hurtlog')
    console.step(25)
    observation['collateral_remaining'] = console.count('minecraft:pig', 'host-impact')
    observation['scores'] = [score(console, name) for name in names]
    observation['wire_kills'] = [client.statistics(server).get('minecraft:killed', {}).get('minecraft:pig', 0)
                                 for client in [shooter, other]]
    (directory / 'observation.json').write_text(json.dumps(observation, indent=2) + '\n')
    assert after[0] == before[0] and after[2] == before[2], ('shooter or bystander was hurt', observation)
    assert 0 < after[1] < before[1], ('survival host did not take damage', observation)
    assert observation['collateral_remaining'] == 0, ('collateral survived', observation)
    if isinstance(console, HostConsole):
        hurts = {player['name']: player['hurts'] for player in observation['hurtlog']['players']}
        assert any(hurt['source'] == 'minecraft:fireball' and hurt['hurt'] for hurt in hurts['host']), observation
        assert not any(hurt['hurt'] for hurt in hurts[SHOOTER]), observation
    assert observation['scores'] == [1, 0, 0], ('collateral credited to the wrong player', observation['scores'])
    assert observation['wire_kills'] == [1, 0], observation
    return {'scores': observation['scores'], 'wire_kills': observation['wire_kills'],
            'host_hurt': True, 'shooter_unhurt': True, 'bystander_unhurt': True, 'collateral_killed': True}


def native_case(binary, world, build, classes, dependencies):
    directory = build / 'horizon'
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir()
    host = InFrameHost(binary, world, directory)
    clients = []
    profiles = {}
    try:
        port = host.start()
        for name, label in [(SHOOTER, 'shooter'), (OTHER, 'other')]:
            client = SaveClient(port, name, directory, classes, dependencies, label)
            clients.append(client)
            client.wait_ready()
        result = impact(HostConsole(host), host, *clients, directory)
    finally:
        try:
            failures = []
            for client in reversed(clients):
                try:
                    profiles[client.name] = wire_uuid(client.close(), client.name)
                except Exception as error:
                    if client.process.poll() is None:
                        client.process.kill()
                        client.process.wait(timeout=10)
                    failures.append(error)
            if failures:
                raise failures[0]
        finally:
            host.close()
    result['wire_profiles'] = profiles
    return result


def vanilla_case(command, world, build, classes, dependencies):
    directory = build / 'vanilla'
    port = server_dir(directory, world)
    with running(command, directory, 'vanilla-host-impact') as (server, console, clients):
        join(server, console, clients, port, HOST, directory, classes, dependencies, 'host-player')
        shooter = join(server, console, clients, port, SHOOTER, directory, classes, dependencies, 'shooter')
        other = join(server, console, clients, port, OTHER, directory, classes, dependencies, 'other')
        result = impact(console, server, shooter, other, directory)
    profiles = {client.name: wire_uuid(client.output.read_text(), client.name) for client in clients}
    result['wire_profiles'] = {name: profiles[name] for name in [SHOOTER, OTHER]}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--world', type=Path)
    parser.add_argument('--output', type=Path, default=lab.LOCAL / 'projectile-host-impact-parity')
    parser.add_argument('--side', choices=['both', 'horizon', 'vanilla'], default='both')
    args = parser.parse_args()
    build = args.output.resolve()
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes, dependencies = compile_client(build)
    command = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    binary = args.binary.resolve() if args.binary else lab.cargo_target() / 'release/horizon-client'
    if args.binary is None and args.side != 'vanilla':
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'],
                       cwd=lab.ROOT, check=True)
    world = args.world.resolve() if args.world else generate_world(command, build)
    results = {}
    if args.side in ['both', 'vanilla']:
        results['vanilla'] = vanilla_case(command, world, build, classes, dependencies)
    if args.side in ['both', 'horizon']:
        results['horizon'] = native_case(binary, world, build, classes, dependencies)
    if args.side == 'both':
        assert results['horizon'] == results['vanilla'], results
    report.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, sort_keys=True), flush=True)


if __name__ == '__main__':
    main()
