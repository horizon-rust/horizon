#!/usr/bin/env python3
"""Exercise online saves and the dedicated engine's snapshot lease on Linux."""
import argparse
import gzip
import zipfile
import ctypes
import select
import struct
import sys
import threading
import tempfile
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import uuid
import contextlib
import re

try:
    from . import anvil_dump, lab, oracle_harness as oh, test_rust_parity as base
    from .test_dedicated_server_parity import Server, write_server_dir
except ImportError:
    import anvil_dump
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, write_server_dir


def isolated_env():
    inherited = dict(os.environ)
    for key in ['HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME']:
        inherited.pop(key, None)
    return lab.game_env(inherited, HORIZON_TELEMETRY_URL='off')


def wait_for(predicate, message, timeout=120):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(message)


def offline_uuid(name):
    return str(uuid.UUID(bytes=hashlib.md5(('OfflinePlayer:' + name).encode()).digest(), version=3))


def scalar(value):
    return value[1] if isinstance(value, tuple) else value


def player_state(world, name, *, identity=None):
    identity = identity or offline_uuid(name)
    nbt = anvil_dump.read_dat(world / 'players/data' / f'{identity}.dat')
    inventory = {scalar(item['Slot']): (item['id'], scalar(item.get('count', 1))) for item in nbt['Inventory']}
    equipment = {key: value['id'] for key, value in nbt.get('equipment', {}).items()}
    stats = json.loads((world / 'players/stats' / f'{identity}.json').read_text())
    advancements = json.loads((world / 'players/advancements' / f'{identity}.json').read_text())
    play_time = stats.get('stats', {}).get('minecraft:custom', {}).get('minecraft:play_time', 0)
    assert isinstance(play_time, int) and play_time > 0, (name, stats)
    return dict(inventory=inventory, equipment=equipment, dimension=nbt['Dimension'],
                position=[scalar(v) for v in nbt['Pos']], stats_version=stats['DataVersion'], play_time_positive=True,
                advancement_done=advancements.get('minecraft:story/root', {}).get('done', False))


class SaveClient:
    def __init__(self, port, name, build, classes, dependencies, label):
        self.name = name
        self.batch = 0
        self.release = build / f'{label}.release'
        self.ready = self.release.with_name(self.release.name + '.ready')
        self.output = build / f'{label}.txt'
        self.release.unlink(missing_ok=True)
        self.ready.unlink(missing_ok=True)
        self.release.with_name(self.release.name + '.commands').unlink(missing_ok=True)
        self.log = (build / f'{label}.log').open('w')
        self.process = subprocess.Popen(['java', '-Xmx512m', '-Duser.timezone=UTC', '-cp',
                                         str(classes) + os.pathsep + dependencies,
                                         'dev.horizonlab.parity.VanillaProtocolClient', '127.0.0.1', str(port),
                                         name, str(self.output), 'save', str(self.release)],
                                        stdout=self.log, stderr=subprocess.STDOUT, env=isolated_env())

    def wait_ready(self):
        wait_for(lambda: self.ready.exists() or self.process.poll() is not None,
                 f'{self.output}: client did not load')
        assert self.ready.exists(), self.output.read_text() if self.output.exists() else 'client failed'

    def commands(self, server, commands):
        self.batch += 1
        path = self.release.with_name(self.release.name + '.commands')
        temporary = path.with_name(path.name + '.new')
        temporary.write_text('\n'.join(commands) + '\n')
        temporary.replace(path)
        wait_for(lambda: any(f'<{self.name}> save fixture {self.batch}' in line for line in server.lines),
                 f'{self.name}: fixture commands did not finish')

    def close(self):
        self.release.touch()
        try:
            self.process.wait(timeout=40)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise
        finally:
            self.log.close()
        transcript = self.output.read_text()
        assert not any(line.startswith('error ') for line in transcript.splitlines()), '\n'.join(transcript.splitlines()[-8:])
        return transcript


def save_flush(server):
    start = len(server.lines)
    server.console('save-all flush')
    wait_for(lambda: any('Saved the game' in line for line in server.lines[start:]),
             f'{server.log_path}: forced save failed')


def set_players(server, clients):
    for index, client in enumerate(clients, 1):
        name = client.name
        for command in [f'op {name}', f'gamemode creative {name}', f'tp {name} {index + 3}.5 100 4.5']:
            server.console(command)
        server.console(f'say fixture op {name}')
        server.wait_for(f'fixture op {name}')
        client.commands(server, [f'item replace entity @s hotbar.8 with minecraft:diamond {index + 6}',
                                 'item replace entity @s armor.head with minecraft:diamond_helmet',
                                 'item replace entity @s weapon.offhand with minecraft:shield',
                                 'advancement grant @s only minecraft:story/root'])


def assert_players(world, names):
    states = []
    for index, name in enumerate(names, 1):
        state = player_state(world, name)
        assert state['inventory'][8] == ('minecraft:diamond', index + 6), state
        assert state['equipment']['head'] == 'minecraft:diamond_helmet', state
        assert state['equipment']['offhand'] == 'minecraft:shield', state
        assert state['dimension'] == 'minecraft:overworld', state
        assert state['position'][0] == index + 3.5, state
        assert state['advancement_done'], state
        states.append(state)
    return states


def online_player_save(label, command, world, build, classes, dependencies):
    directory = build / f'{label}-online-save'
    port = lab.free_port()
    write_server_dir(directory, port, world)
    server = Server(command, directory, f'{label}-online-save', environment=isolated_env())
    clients = []
    names = ['SaveOne', 'SaveTwo']
    try:
        server.wait_for(r'Done \(')
        clients = [SaveClient(port, name, build, classes, dependencies, f'{label}-{name}') for name in names]
        for client in clients:
            client.wait_ready()
        set_players(server, clients)
        server.console('save-off')
        save_flush(server)
        saved = assert_players(directory / 'world', names)
        for client in clients:
            client.close()
        clients = []
        wait_for(lambda: sum('left the game' in line for line in server.lines) >= 2, 'players did not leave')
        clients = [SaveClient(port, name, build, classes, dependencies, f'{label}-{name}-rejoin') for name in names]
        for client in clients:
            client.wait_ready()
        save_flush(server)
        assert assert_players(directory / 'world', names) == saved
        return saved
    finally:
        for client in clients:
            with contextlib.suppress(Exception):
                client.close()
        server.stop()

def replace_uuid(path, key, identity):
    """Change one fixture tag while retaining every other NBT byte."""
    raw = gzip.decompress(path.read_bytes())
    name = key.encode()
    marker = b'\x0b' + struct.pack('>H', len(name)) + name + struct.pack('>i', 4)
    tag = marker + uuid.UUID(identity).bytes
    if marker in raw:
        assert raw.count(marker) == 1
        start = raw.index(marker)
        raw = raw[:start] + tag + raw[start + len(tag):]
    else:
        assert key == 'singleplayer_uuid' and raw.endswith(b'\x00\x00')
        assert set(anvil_dump.read_dat(path)) == {'Data'}
        raw = raw[:-2] + tag + raw[-2:]
    path.write_bytes(gzip.compress(raw, mtime=0))


def legacy_nbt_string(value):
    raw = value.encode('utf-8')
    return struct.pack('>H', len(raw)) + raw


def legacy_nbt_payload(kind, value):
    if kind == 1:
        return struct.pack('>b', value)
    if kind == 3:
        return struct.pack('>i', value)
    if kind == 4:
        return struct.pack('>q', value)
    if kind == 5:
        return struct.pack('>f', value)
    if kind == 6:
        return struct.pack('>d', value)
    if kind == 8:
        return legacy_nbt_string(value)
    if kind == 9:
        element, values = value
        return struct.pack('>bi', element, len(values)) + b''.join(
            legacy_nbt_payload(element, item) for item in values)
    if kind == 10:
        return b''.join(bytes([item_kind]) + legacy_nbt_string(name)
                        + legacy_nbt_payload(item_kind, item_value)
                        for name, (item_kind, item_value) in value) + b'\0'
    raise AssertionError(f'unsupported legacy NBT tag {kind}')


def write_legacy_horizon_level(path):
    """Write the exact embedded-player shape from Horizon revision 94539db."""
    tag = lambda kind, value: (kind, value)
    player = [
        ('Health', tag(5, 17.25)),
        ('foodLevel', tag(3, 16)),
        ('foodSaturationLevel', tag(5, 2.5)),
        ('XpLevel', tag(3, 7)),
        ('XpP', tag(5, 0.25)),
        ('XpTotal', tag(3, 53)),
        ('playerGameType', tag(3, 0)),
        ('Rotation', tag(9, (5, [15.0, -30.0]))),
        ('Inventory', tag(9, (10, [[
            ('Slot', tag(1, 8)),
            ('id', tag(8, 'minecraft:diamond')),
            ('count', tag(3, 5)),
        ]]))),
        ('Dimension', tag(8, 'minecraft:overworld')),
        ('Pos', tag(9, (6, [12.5, 80.0, -4.25]))),
    ]
    data = [
        ('DataVersion', tag(3, 5023)),
        ('version', tag(3, 19133)),
        ('LevelName', tag(8, 'Old Horizon')),
        ('GameType', tag(3, 0)),
        ('Time', tag(4, 1200)),
        ('DayTime', tag(4, 1200)),
        ('SpawnX', tag(3, 0)),
        ('SpawnY', tag(3, 64)),
        ('SpawnZ', tag(3, 0)),
        ('initialized', tag(1, 1)),
        ('clearWeatherTime', tag(3, 0)),
        ('rainTime', tag(3, 0)),
        ('thunderTime', tag(3, 0)),
        ('raining', tag(1, 0)),
        ('thundering', tag(1, 0)),
        ('allowCommands', tag(1, 0)),
        ('hardcore', tag(1, 0)),
        ('Difficulty', tag(1, 2)),
        ('WorldGenSettings', tag(10, [
            ('seed', tag(4, 123)),
            ('generate_features', tag(1, 1)),
            ('bonus_chest', tag(1, 0)),
        ])),
        ('Player', tag(10, player)),
    ]
    root = bytes([10]) + legacy_nbt_string('') + legacy_nbt_payload(10, [('Data', (10, data))])
    path.write_bytes(gzip.compress(root, mtime=0))


def player_records(world):
    return {str(path.relative_to(world)): path.read_bytes()
            for path in (world / 'players').rglob('*') if path.is_file()}


def owner_player_save(label, command, source, build, classes, dependencies, *, snapshot=False,
                      cases=('present-owner', 'missing-owner', 'online-owner', 'offline-owner')):
    """Save, archive and reopen populated worlds with independent owner identities."""
    results = {}
    names = ['SaveOne', 'SaveTwo']
    unrelated = offline_uuid('SnapshotOwner')
    for case in cases:
        directory = build / f'{label}-{case}'
        port = lab.free_port()
        write_server_dir(directory, port, source)
        world = directory / 'world'
        owner = unrelated if case in ('present-owner', 'missing-owner') else offline_uuid('SaveTwo')
        replace_uuid(world / 'level.dat', 'singleplayer_uuid', owner)
        if case == 'present-owner':
            for kind, suffix in [('data', 'dat'), ('stats', 'json'), ('advancements', 'json')]:
                origin = world / 'players' / kind / f'{offline_uuid("SaveTwo")}.{suffix}'
                destination = origin.with_name(f'{owner}.{suffix}')
                shutil.copyfile(origin, destination)
                if kind == 'data':
                    replace_uuid(destination, 'UUID', owner)
        elif case == 'missing-owner':
            (world / 'players/data' / f'{owner}.dat').unlink(missing_ok=True)
        active = names[:1] if case == 'offline-owner' else names
        for index, name in enumerate(names):
            stats = world / 'players/stats' / f'{offline_uuid(name)}.json'
            data = json.loads(stats.read_text())
            data['stats'].setdefault('minecraft:custom', {})['minecraft:jump'] = 91 + index
            data['stats']['minecraft:custom']['minecraft:play_time'] = 10000 * (index + 1)
            stats.write_text(json.dumps(data))
        initial = player_records(world)
        inactive = {key: value for key, value in initial.items()
                    if not any(offline_uuid(name) in key for name in active)}
        with tempfile.TemporaryDirectory(prefix='ho-') as control_dir:
            endpoint = Path(control_dir) / 'ctl.sock'
            launched = list(command)
            if snapshot:
                # Horizon's dedicated flag consumes the following arguments.
                index = launched.index('--dedicated')
                launched[index:index] = ['--control-socket', endpoint]
            server = Server(launched, directory, f'{label}-{case}', environment=isolated_env())
            clients = []
            held = False
            expected = {}
            archive = build / f'{label}-{case}.zip'
            try:
                server.wait_for(r'Done \(')
                clients = [SaveClient(port, name, build, classes, dependencies, f'{label}-{case}-{name}')
                           for name in active]
                for client in clients:
                    client.wait_ready()
                set_players(server, clients)
                for index, client in enumerate(clients):
                    client.commands(server, [f'item replace entity @s hotbar.8 with minecraft:emerald {31 + index}',
                                             f'tp @s {14.5 + index} 100 4.5',
                                             'advancement revoke @s only minecraft:story/root'])
                save_flush(server)

                def check_saved():
                    for index, name in enumerate(active):
                        state = player_state(world, name)
                        assert state['inventory'][8] == ('minecraft:emerald', 31 + index), (case, name, state)
                        assert state['position'][0] == 14.5 + index, (case, name, state)
                        assert not state['advancement_done'], (case, name, state)
                        stats = json.loads((world / 'players/stats' / f'{offline_uuid(name)}.json').read_text())
                        assert stats['stats']['minecraft:custom']['minecraft:jump'] == 91 + index
                        assert stats['stats']['minecraft:custom']['minecraft:play_time'] > 10000 * (index + 1)
                        assert scalar(anvil_dump.read_dat(world / 'level.dat')['Data']['singleplayer_uuid']) == list(
                            struct.unpack('>4i', uuid.UUID(owner).bytes))
                        expected[name] = state
                    current = player_records(world)
                    assert {key: current.get(key) for key in inactive} == inactive, (case, 'offline records changed')
                    assert set(current) == set(initial), (case, 'synthetic player created')

                check_saved()
                if snapshot:
                    response = control(endpoint, 'backup-begin owner-regression')
                    held = response.get('ok') is True
                    assert response == {'ok': True, 'snapshot': 'owner-regression', 'world': 'world'}, response
                    check_saved()
                    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
                        for path in world.rglob('*'):
                            if path.is_file() and path.name != 'session.lock':
                                output.write(path, path.relative_to(world))
                    assert control(endpoint, 'backup-end owner-regression') == {'ok': True, 'snapshot': 'owner-regression'}
                    held = False
                for client in clients:
                    client.close()
                clients = []
            finally:
                try:
                    if held and server.process.poll() is None:
                        control(endpoint, 'backup-end owner-regression')
                finally:
                    for client in clients:
                        with contextlib.suppress(Exception):
                            client.close()
                    server.stop()
            # Restore the actual held archive, then join the same profiles again.
            if snapshot:
                shutil.rmtree(world)
                with zipfile.ZipFile(archive) as saved:
                    saved.extractall(world)
            server = Server(launched, directory, f'{label}-{case}-reopen', environment=isolated_env())
            clients = []
            try:
                server.wait_for(r'Done \(')
                clients = [SaveClient(port, name, build, classes, dependencies, f'{label}-{case}-{name}-reopen')
                           for name in active]
                for client in clients:
                    client.wait_ready()
                save_flush(server)
                for name in active:
                    assert player_state(world, name) == expected[name], (case, name, 'reopen lost player state')
                check_saved()
                current = player_records(world)
                assert {key: current.get(key) for key in inactive} == inactive
                assert set(current) == set(initial), (case, 'reopen created a synthetic player')
                for client in clients:
                    transcript = client.close()
                    assert 'minecraft:emerald' in transcript, (case, client.name, 'restored inventory not sent')
                clients = []
                results[case] = expected
            finally:
                for client in clients:
                    with contextlib.suppress(Exception):
                        client.close()
                server.stop()
    return results


def legacy_embedded_player_save(binary, resources, source, build):
    """Open the old same-version Horizon level through the dedicated engine."""
    directory = build / 'legacy-embedded-player'
    port = lab.free_port()
    write_server_dir(directory, port, source)
    world = directory / 'world'
    shutil.rmtree(world / 'players', ignore_errors=True)
    write_legacy_horizon_level(world / 'level.dat')
    server = Server([binary, '--resources', resources, '--dedicated', '--nogui'], directory,
                    'legacy-embedded-player', environment=isolated_env())
    try:
        server.wait_for(r'Done \(')
        # Dedicated startup saves world metadata before announcing saved levels.
        server.wait_for(r"Saving chunks for level 'ServerLevel\[Old Horizon\]'")
        data = anvil_dump.read_dat(world / 'level.dat')['Data']
        assert 'Player' not in data, 'legacy Data.Player survived metadata save'
        identity = '00000000-0000-0000-0000-000000000000'
        player_path = world / 'players/data' / f'{identity}.dat'
        assert player_path.is_file(), (
            'dedicated startup removed legacy Data.Player without migrating it to the owner player file')
        assert 'singleplayer_uuid' in data, 'legacy player migration did not declare its owner UUID'
        assert scalar(data['singleplayer_uuid']) == [0, 0, 0, 0], data
        player = anvil_dump.read_dat(player_path)
        assert scalar(player['UUID']) == [0, 0, 0, 0], player
        assert scalar(player['DataVersion']) == 5023, player
        item = player['Inventory'][0]
        assert (item['id'], scalar(item['Slot']), scalar(item['count'])) == (
            'minecraft:diamond', 8, 5), player
        assert [scalar(value) for value in player['Pos']] == [12.5, 80.0, -4.25], player
        assert [scalar(value) for value in player['Rotation']] == [15.0, -30.0], player
        assert player['Dimension'] == 'minecraft:overworld', player
        assert scalar(player['Health']) == 17.25, player
        assert scalar(player['foodLevel']) == 16, player
        assert scalar(player['XpLevel']) == 7 and scalar(player['XpTotal']) == 53, player
        return dict(
            world_data_version=scalar(data['DataVersion']),
            player_uuid=identity,
            inventory='minecraft:diamond x5 in slot 8',
            position=[12.5, 80.0, -4.25],
            rotation=[15.0, -30.0],
            health=17.25,
            experience=[7, 53],
            embedded_player_removed=True,
            startup_save_observed=True,
        )
    finally:
        server.stop()



def local_owner_save(binary, resources, source, build):
    """Keep the saved owner distinct from the profile in both local modes."""
    results = {}
    with tempfile.TemporaryDirectory(prefix='hl-') as scratch:
        scratch = Path(scratch)
        with (build / 'local-owner-display.log').open('w') as display_log:
            display = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '640x480x24',
                                        '-nolisten', 'tcp'], stdout=subprocess.PIPE, stderr=display_log, text=True)
            try:
                assert select.select([display.stdout], [], [], 5)[0], 'Xvfb did not become ready'
                display_name = ':' + display.stdout.readline().strip()
                for mode in ('in-frame', 'integrated'):
                    directory = build / f'local-owner-{mode}'
                    shutil.rmtree(directory, ignore_errors=True)
                    world = directory / 'saves/world'
                    shutil.copytree(source, world)
                    (world / 'session.lock').unlink(missing_ok=True)
                    identity = offline_uuid('SnapshotOwner')
                    replace_uuid(world / 'level.dat', 'singleplayer_uuid', identity)
                    for kind, suffix in [('data', 'dat'), ('stats', 'json'), ('advancements', 'json')]:
                        origin = world / 'players' / kind / f'{offline_uuid("SaveTwo")}.{suffix}'
                        destination = origin.with_name(f'{identity}.{suffix}')
                        shutil.copyfile(origin, destination)
                        if kind == 'data':
                            replace_uuid(destination, 'UUID', identity)
                    inactive = {key: value for key, value in player_records(world).items() if identity not in key}
                    environment = isolated_env()
                    environment.update(DISPLAY=display_name, HORIZON_HOME=str(directory),
                                       HORIZON_SERVER_THREAD='1' if mode == 'integrated' else '0')
                    environment.pop('WAYLAND_DISPLAY', None)
                    for reopened in (False, True):
                        endpoint = scratch / f'{mode}.sock'
                        endpoint.unlink(missing_ok=True)
                        command = [binary, '--resources', resources, '--control-socket', endpoint,
                                   '--world', 'world', '--name', 'SaveOne', '--offscreen', '--no-pause',
                                   '--render-distance', '2', '--width', '640', '--height', '480']
                        log = (build / f'local-owner-{mode}-{reopened}.log').open('w')
                        process = subprocess.Popen(list(map(str, command)), cwd=directory, env=environment,
                                                   stdout=log, stderr=subprocess.STDOUT)
                        try:
                            wait_for(lambda: endpoint.exists() or process.poll() is not None,
                                     f'{mode}: no control socket')
                            assert process.poll() is None, (mode, process.returncode)
                            wait_for(lambda: control(endpoint, 'state').get('player') is not None,
                                     f'{mode}: local owner did not spawn')
                            if not reopened:
                                for text in ['gamemode creative',
                                             'item replace entity @s hotbar.8 with minecraft:emerald 47',
                                             'advancement revoke @s only minecraft:story/root']:
                                    reply = control(endpoint, 'command ' + text)
                                    assert 'error' not in reply, reply
                            assert control(endpoint, 'save').get('ok') is True
                            state = player_state(world, 'SnapshotOwner', identity=identity)
                            assert state['inventory'][8] == ('minecraft:emerald', 47), (mode, reopened, state)
                            assert not state['advancement_done'], (mode, reopened, state)
                            owner = scalar(anvil_dump.read_dat(world / 'level.dat')['Data']['singleplayer_uuid'])
                            assert owner == list(struct.unpack('>4i', uuid.UUID(identity).bytes)), (mode, owner)
                            current = player_records(world)
                            assert {key: current.get(key) for key in inactive} == inactive, mode
                            assert control(endpoint, 'quit').get('ok') is True
                            process.wait(timeout=120)
                            assert process.returncode == 0, (mode, process.returncode)
                            results[mode] = dict(saved_uuid=identity, profile_uuid=offline_uuid('SaveOne'),
                                                 inventory_preserved=True, advancements_preserved=True, reopened=True)
                        finally:
                            if process.poll() is None:
                                process.kill()
                                process.wait(timeout=10)
                            log.close()
            finally:
                display.terminate()
                display.wait(timeout=10)
    return results


def control(path, command):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(120)
        connection.connect(str(path))
        connection.sendall((command + '\n').encode())
        with connection.makefile('r') as source:
            return json.loads(source.readline())


class WorldObserver:
    def __init__(self, world):
        if sys.platform != 'linux':
            raise RuntimeError('the snapshot observer requires Linux inotify')
        self.world = world
        self.ready = threading.Event()
        self.stop = threading.Event()
        self.release_requested = threading.Event()
        self.released = threading.Event()
        self.positive = threading.Event()
        self.error = None
        self.held_events = []
        self.release_events = 0
        self.thread = threading.Thread(target=self._run, name='world-write-observer')
        self.thread.start()
        if not self.ready.wait(5):
            self.close()
            raise AssertionError('world observer did not become ready')
        try:
            self.assert_held()
        except BaseException:
            self.close()
            raise

    def _run(self):
        descriptor = -1
        try:
            libc = ctypes.CDLL(None, use_errno=True)
            descriptor = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
            if descriptor < 0:
                raise OSError(ctypes.get_errno(), 'inotify_init1')
            mutations = 0x00000fc6
            watched = {}
            for directory in [self.world, *[path for path in self.world.rglob('*') if path.is_dir()]]:
                watch = libc.inotify_add_watch(descriptor, os.fsencode(directory), mutations)
                if watch < 0:
                    raise OSError(ctypes.get_errno(), str(directory))
                watched[watch] = directory
            self.ready.set()
            deadline = time.monotonic() + 240
            while not self.stop.is_set():
                if time.monotonic() >= deadline:
                    raise TimeoutError('world observer exceeded 240 seconds')
                if not select.select([descriptor], [], [], 0.02)[0]:
                    if self.release_requested.is_set():
                        self.released.set()
                    continue
                events = os.read(descriptor, 65536)
                offset = 0
                while offset < len(events):
                    watch, mask, _, size = struct.unpack_from('iIII', events, offset)
                    name = events[offset + 16:offset + 16 + size].split(b'\0', 1)[0]
                    offset += 16 + size
                    if mask & 0x00004000:
                        raise RuntimeError('world observer event queue overflowed')
                    if not mask & mutations:
                        continue
                    path = watched[watch] / os.fsdecode(name)
                    if self.released.is_set():
                        self.release_events += 1
                        self.positive.set()
                    elif len(self.held_events) < 8:
                        self.held_events.append((str(path.relative_to(self.world)), hex(mask)))
        except BaseException as error:
            self.error = error
            self.ready.set()
        finally:
            if descriptor >= 0:
                os.close(descriptor)

    def assert_held(self):
        assert self.error is None, self.error
        assert not self.held_events, ('world mutations while held', self.held_events)
        assert self.thread.is_alive(), 'world observer stopped early'

    def allow_release(self):
        self.release_requested.set()
        assert self.released.wait(5), 'world observer did not drain held events'
        self.assert_held()

    def assert_release_observed(self):
        assert self.positive.wait(5), 'observer did not detect release writes'
        assert self.error is None, self.error
        assert not self.held_events, self.held_events

    def close(self):
        self.stop.set()
        self.thread.join(timeout=5)
        assert not self.thread.is_alive(), 'world observer did not join'


def disk_tree(world):
    return {str(path.relative_to(world)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in world.rglob('*') if path.is_file()}


def game_time(endpoint):
    response = control(endpoint, 'console time query gametime')
    match = re.search(r'time is (\d+)', '\n'.join(response.get('output', [])))
    assert match, response
    return int(match.group(1))


def sprint(server, endpoint, ticks):
    target = game_time(endpoint) + ticks
    assert 'error' not in control(endpoint, f'console tick sprint {ticks}')
    wait_for(lambda: game_time(endpoint) >= target, 'tick sprint did not advance simulation', timeout=120)


def snapshot_case(binary, resources, build, classes, dependencies):
    directory = build / 'engine'
    port = lab.free_port()
    write_server_dir(directory, port)
    with (directory / 'server.properties').open('a') as props:
        props.write('view-distance=2\nsimulation-distance=2\npause-when-empty-seconds=1\n')
    endpoint = Path(tempfile.mkdtemp(prefix='hs-')) / 'control.sock'
    command = [binary, '--resources', resources, '--test-terrain', '--control-socket', endpoint, '--dedicated', '--nogui']
    server = Server(command, directory, 'snapshot-engine', environment=isolated_env())
    clients = []
    held = False
    observer = None
    world = directory / 'world'
    try:
        server.wait_for(r'Done \(')
        clients = [SaveClient(port, name, build, classes, dependencies, name) for name in ['SaveOne', 'SaveTwo']]
        for client in clients:
            client.wait_ready()
        set_players(server, clients)
        assert 'error' not in control(endpoint, 'console save-off')
        assert 'error' not in control(endpoint, 'console save-all flush')
        assert_players(world, ['SaveOne', 'SaveTwo'])
        regions = {name: value for name, value in disk_tree(world).items() if name.endswith('.mca')}
        sprint(server, endpoint, 6100)
        assert {name: value for name, value in disk_tree(world).items() if name.endswith('.mca')} == regions
        assert 'error' not in control(endpoint, 'console save-all flush')
        assert control(endpoint, 'backup-end cancelled') == {'ok': True, 'snapshot': 'cancelled'}
        assert 'error' in control(endpoint, 'backup-begin cancelled')
        response = control(endpoint, 'backup-begin live')
        assert response == {'ok': True, 'snapshot': 'live', 'world': 'world'}, response
        held = True
        before_time = scalar(anvil_dump.read_dat(world / "level.dat")["Data"]["Time"])
        before = disk_tree(world)
        observer = WorldObserver(world)
        assert control(endpoint, 'backup-begin live') == response
        assert 'error' in control(endpoint, 'backup-begin other')
        assert 'error' in control(endpoint, 'backup-end other')
        assert 'error' in control(endpoint, 'console save-all flush')
        clients[0].commands(server, ['item replace entity @s hotbar.8 with minecraft:emerald 31'])
        for command in ['save-on', 'save-off',
                        'setblock 4 99 4 minecraft:gold_block', 'datapack disable vanilla',
                        'execute in minecraft:the_nether run setblock 0 80 0 minecraft:gold_block']:
            reply = control(endpoint, 'console ' + command)
            assert 'error' not in reply, (command, reply)
        sprint(server, endpoint, 6100)
        clients[0].close()
        wait_for(lambda: any('SaveOne left the game' in line for line in server.lines), 'guest did not depart')
        clients[0] = SaveClient(port, 'SaveOne', build, classes, dependencies, 'SaveOne-held-rejoin')
        clients[0].wait_ready()
        for client in clients:
            transcript = client.close()
            if client.name == 'SaveOne':
                assert '31 minecraft:emerald' in transcript, 'held reconnect loaded stale inventory'
        clients = []
        wait_for(lambda: control(endpoint, 'server-status')['paused'], 'empty pause did not occur')
        assert disk_tree(world) == before, 'held world files changed'
        assert 'error' not in control(endpoint, 'console stop')
        status = control(endpoint, 'server-status')
        assert status['snapshot'] == 'live' and status['stop_pending'], status
        assert not status['autosave'], status
        assert server.process.poll() is None, 'stop bypassed snapshot ownership'
        assert disk_tree(world) == before
        observer.allow_release()
        assert control(endpoint, 'backup-end live') == {'ok': True, 'snapshot': 'live'}
        observer.assert_release_observed()
        held = False
        server.process.wait(timeout=120)
        assert server.process.returncode == 0, server.lines[-30:]
        assert disk_tree(world) != before
        after_time = scalar(anvil_dump.read_dat(world / 'level.dat')['Data']['Time'])
        assert after_time >= before_time + 6100, (before_time, after_time)
        one = player_state(world, 'SaveOne')
        assert one['inventory'][8] == ('minecraft:emerald', 31), one
        return dict(held_files=len(before), held_mutations=len(observer.held_events), release_events=observer.release_events,
                    stop_deferred=True, autosave_preserved=True, players=2)
    finally:
        try:
            if observer is not None:
                observer.close()
        finally:
            for client in clients:
                with contextlib.suppress(Exception):
                    client.close()
            try:
                if held and server.process.poll() is None:
                    control(endpoint, 'backup-end live')
            finally:
                try:
                    server.stop()
                finally:
                    shutil.rmtree(endpoint.parent, ignore_errors=True)


def compile_client(build):
    lock, jars, libraries = base.verify_artifacts()
    classes = build / 'classes'
    classes.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    oh.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-sourcepath',
                   lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes,
                   lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaProtocolClient.java'],
                  build, 'compile-save-client')
    return classes, dependencies


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=lab.ROOT / 'target/debug/horizon-client')
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--output', type=Path, default=lab.LOCAL / 'persistence-snapshot')
    parser.add_argument('--local-owners', action='store_true',
                        help='also check integrated and in-frame owners using Xvfb')
    parser.add_argument('--owner-source', type=Path,
                        help='run only owner regressions from a populated SaveOne and SaveTwo world')
    parser.add_argument('--legacy-owner-only', action='store_true',
                        help='run only the same-version embedded-player migration on the dedicated engine')
    args = parser.parse_args()
    if sys.platform != 'linux':
        parser.error('the native snapshot check requires Linux inotify and Unix sockets')
    build = args.output.resolve()
    build.mkdir(parents=True, exist_ok=True)
    if args.legacy_owner_only:
        if not args.owner_source:
            parser.error('--legacy-owner-only requires --owner-source')
        result = legacy_embedded_player_save(
            args.binary.resolve(), args.resources.resolve(), args.owner_source.resolve(), build)
        (build / 'legacy-owner-results.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, sort_keys=True))
        return
    shutil.rmtree(build / 'engine', ignore_errors=True)
    classes, dependencies = compile_client(build)
    command = [args.binary.resolve(), '--resources', args.resources.resolve(), '--dedicated', '--nogui']
    result = {}
    if args.owner_source:
        source = args.owner_source.resolve()
    else:
        result['snapshot'] = snapshot_case(args.binary.resolve(), args.resources.resolve(), build, classes, dependencies)
        source = build / 'engine/world'
    result['owners'] = owner_player_save('horizon', command, source, build, classes, dependencies, snapshot=True)
    if args.local_owners:
        result['local_owners'] = local_owner_save(args.binary.resolve(), args.resources.resolve(), source, build)
    (build / 'results.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
