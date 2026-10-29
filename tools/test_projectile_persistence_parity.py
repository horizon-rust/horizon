#!/usr/bin/env python3
"""Compare projectile unload, owner saves and guest attribution with vanilla."""
import argparse
import contextlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import time
import uuid
import zipfile

try:
    from . import anvil_dump, lab
    from .test_locator_bar_parity import generate_world, server_dir
    from .test_dedicated_server_parity import Server
    from .server_fixture import (
        SaveClient, compile_client, isolated_env, offline_uuid, scalar, wait_for,
    )
except ImportError:
    import anvil_dump
    import lab
    from test_locator_bar_parity import generate_world, server_dir
    from test_dedicated_server_parity import Server
    from server_fixture import (
        SaveClient, compile_client, isolated_env, offline_uuid, scalar, wait_for,
    )


class Console:
    def __init__(self, server):
        self.server = server
        self.sequence = 0
        self.dimension = None

    def __call__(self, command):
        self.sequence += 1
        marker = f'projectile-barrier-{self.sequence}'
        start = len(self.server.lines)
        self.server.console(command)
        self.server.console('say ' + marker)
        self.server.wait_for(marker, timeout=60)
        return '\n'.join(line for line in self.server.lines[start:] if marker not in line)

    def world(self, command):
        if self.dimension:
            command = f'execute in {self.dimension} run {command}'
        return self(command)

    def count(self, kind, tag=None):
        selector = f'type={kind}' + (f',tag={tag}' if tag else '')
        reply = self.world(f'execute if entity @e[{selector}]')
        match = re.search(r'Test passed\. Count: (\d+)', reply)
        if match:
            return int(match[1])
        assert 'Test failed' in reply, reply
        return 0

    def loaded(self):
        reply = self('execute if loaded 8 100 8')
        assert 'Test passed' in reply or 'Test failed' in reply, reply
        return 'Test passed' in reply

    def step(self, ticks):
        target = self.game_time() + ticks
        self(f'tick step {ticks}')
        wait_for(lambda: self.game_time() >= target, 'ticks did not complete', timeout=60)

    def game_time(self, dimension=None):
        command = 'time query gametime'
        reply = self(f'execute in {dimension} run {command}' if dimension else command)
        match = re.search(r'time is (\d+)', reply)
        assert match, reply
        return int(match[1])

    def entity_uuid(self, selector, path='UUID', dimension=None):
        command = f'data get entity {selector} {path}'
        reply = self(f'execute in {dimension} run {command}') if dimension else self.world(command)
        match = re.search(r'\[I;([^]]+)\]', reply)
        assert match, reply
        values = [int(value) for value in match[1].split(',')]
        assert len(values) == 4, reply
        return values

    def item_clock(self, dimension):
        selector = '@e[type=minecraft:item,tag=dimension-clock-fixture,distance=..1024,limit=1]'
        reply = self(f'execute in {dimension} run data get entity {selector} Age')
        match = re.search(r'following entity data: (-?\d+)s\b', reply)
        assert match, reply
        return {'age': int(match[1]), 'uuid': self.entity_uuid(selector, dimension=dimension)}

    def save(self):
        reply = self('save-all flush')
        assert 'Saved the game' in reply, reply

    def vector(self, selector, path):
        reply = self(f'data get entity {selector} {path}')
        values = [float(value) for value in re.findall(r'(-?\d+(?:\.\d+)?(?:[Ee][+-]?\d+)?)d', reply)]
        assert len(values) == 3 and all(map(math.isfinite, values)), reply
        return values


@contextlib.contextmanager
def running(command, directory, label):
    server = Server(command, directory, label, environment=isolated_env())
    clients = []
    try:
        server.wait_for(r'Done \(')
        yield server, Console(server), clients
    finally:
        try:
            failures = []
            for client in clients:
                try:
                    client.close()
                except Exception as error:
                    if client.process.poll() is None:
                        client.process.kill()
                        client.process.wait(timeout=10)
                    failures.append(error)
            if failures:
                raise failures[0]
        finally:
            server.stop()


def join(server, console, clients, port, name, build, classes, dependencies, label):
    client = SaveClient(port, name, build, classes, dependencies, label)
    clients.append(client)
    client.wait_ready()
    console('op ' + name)
    console('gamemode creative ' + name)
    return client


def saved_projectiles(world, kind):
    directory = world / 'dimensions/minecraft/overworld/entities'
    return [entity for region in directory.glob('*.mca')
            for chunk in anvil_dump.region_chunks(region).values()
            for entity in chunk.get('Entities', []) if entity.get('id') == kind]


def dimension_inputs(directory, build, label, command, dependencies):
    definitions = {}
    for key, dimension in [('any', None), ('nether', 'minecraft:the_nether'), ('overworld', 'minecraft:overworld')]:
        conditions = {} if dimension is None else {'player': {
            'type': 'minecraft:location_check', 'predicate': {'dimension': dimension}}}
        definitions[key] = {'criteria': {'kill': {
            'trigger': 'minecraft:player_killed_entity', 'conditions': conditions}}, 'requirements': [['kill']]}
    definitions['nether']['rewards'] = {'experience': 5}
    resources = build / (label + '-dimension-resources')
    if resources.exists():
        shutil.rmtree(resources)
    resources.mkdir()
    for entry in lab.RESOURCES.iterdir():
        if entry.name != 'data':
            (resources / entry.name).symlink_to(entry.resolve(), target_is_directory=entry.is_dir())
    (resources / 'data').mkdir()
    for entry in (lab.RESOURCES / 'data').iterdir():
        (resources / 'data' / entry.name).symlink_to(entry.resolve(), target_is_directory=entry.is_dir())
    namespace = resources / 'data/council_audit'
    assert not namespace.exists() and not namespace.is_symlink(), namespace
    custom = namespace / 'advancement'
    custom.mkdir(parents=True)
    pack = directory / 'world/datapacks/council-audit'
    (pack / 'data/council_audit/advancement').mkdir(parents=True)
    with zipfile.ZipFile(dependencies.split(os.pathsep)[0]) as jar:
        version = json.loads(jar.read('version.json'))
    data_format = [version['pack_version']['data_major'], version['pack_version']['data_minor']]
    (pack / 'pack.mcmeta').write_text(json.dumps({'pack': {
        'description': 'Projectile kill credit fixture', 'min_format': data_format, 'max_format': data_format}}) + '\n')
    for key, definition in definitions.items():
        content = json.dumps(definition, sort_keys=True) + '\n'
        (custom / f'{key}.json').write_text(content)
        (pack / f'data/council_audit/advancement/{key}.json').write_text(content)
    command = list(command)
    if '--resources' in command:
        command[command.index('--resources') + 1] = resources
    return command, version['world_version']


def saved_advancements(world, client, version):
    path = world / 'players/advancements' / (offline_uuid(client.name) + '.json')
    saved = json.loads(path.read_text())
    assert saved['DataVersion'] == version, saved
    result = {}
    for key in ('any', 'nether', 'overworld'):
        progress = saved.get('council_audit:' + key)
        if progress is None:
            result[key] = False
        else:
            assert isinstance(progress['done'], bool), progress
            assert progress['done'] == ('kill' in progress['criteria']), progress
            result[key] = progress['done']
    return result


def unload_case(label, command, world, build, classes, dependencies):
    directory = build / (label + '-unload')
    port = server_dir(directory, world)
    kinds = ['minecraft:arrow', 'minecraft:small_fireball', 'minecraft:fireball']
    with running(command, directory, label + '-unload') as (server, console, clients):
        client = join(server, console, clients, port, 'ProjectileGuest', build, classes, dependencies,
                      label + '-unload-guest')
        client.commands(server, ['tp @s 8.5 100 8.5'])
        console('tick freeze')
        wait_for(console.loaded, 'original chunk did not load')
        for index, kind in enumerate(kinds):
            console(f'summon {kind} {8.5 + index} 120 8.5 '
                    '{Owner:[I;11,22,33,44],Motion:[0.01d,0d,0d],acceleration_power:0.0d}')
        console.step(5)
        console.save()
        for kind in kinds:
            assert console.count(kind) == 1, (label, 'before unload', kind)
            assert len(saved_projectiles(directory / 'world', kind)) == 1, (label, 'saved', kind)
        console('tick unfreeze')
        client.commands(server, ['tp @s 2008.5 100 2008.5'])
        wait_for(lambda: not console.loaded(), 'original chunk did not unload', timeout=90)
        for kind in kinds:
            wait_for(lambda: console.count(kind) == 0, f'unloaded {kind} remains live', timeout=30)
            assert console.count(kind) == 0, (label, 'unloaded projectile remains live', kind)
        console('tick freeze')
        client.commands(server, ['tp @s 8.5 100 8.5'])
        wait_for(console.loaded, 'original chunk did not reload', timeout=90)
        for kind in kinds:
            wait_for(lambda: console.count(kind) != 0, f'{kind} did not reload', timeout=30)
            assert console.count(kind) == 1, (label, 'duplicate after reload', kind)
        return {'saved': 3, 'unloaded': 0, 'reloaded': 3}


def owner_case(label, command, world, build, classes, dependencies):
    directory = build / (label + '-owner')
    port = server_dir(directory, world)
    name = 'ProjectileGuest'
    expected = list(struct.unpack('>4i', uuid.UUID(offline_uuid(name)).bytes))
    with running(command, directory, label + '-owner-first') as (server, console, clients):
        client = join(server, console, clients, port, name, build, classes, dependencies, label + '-first')
        client.commands(server, ['tp @s 8.5 100 8.5 0 0'])
        console('tick freeze')
        console('forceload add 0 0')
        console('summon minecraft:fireball 8.5 101.6 10.5 {ExplosionPower:2b}')
        entities = client.release.with_name(client.release.name + '.entities')
        wait_for(lambda: entities.exists() and 'minecraft:fireball' in entities.read_text(),
                 'client did not observe the fireball')
        client.commands(server, ['@attack minecraft:fireball'])
        console.save()
        saved = saved_projectiles(directory / 'world', 'minecraft:fireball')
        assert len(saved) == 1, (label, saved)
        assert scalar(saved[0].get('Owner')) == expected, (label, 'deflector UUID lost', saved)
        saved_position = [scalar(value) for value in saved[0]['Pos']]
        saved_motion = [scalar(value) for value in saved[0]['Motion']]
        acceleration = scalar(saved[0].get('acceleration_power', 0.1))
        # Avoid keeping the shot's chunk forced during restart.
        console('forceload remove all')
        console.save()
    with running(command, directory, label + '-owner-reopen') as (server, console, clients):
        console('tick freeze')
        bystander = join(server, console, clients, port, 'ProjectileOther', build, classes, dependencies,
                         label + '-other')
        bystander.commands(server, ['tp @s 2.5 100 2.5'])
        owner = join(server, console, clients, port, name, build, classes, dependencies, label + '-rejoin')
        owner.commands(server, ['tp @s 8.5 100 8.5 0 0'])
        wait_for(lambda: console.count('minecraft:fireball') == 1, 'saved fireball did not load')
        shot = '@e[type=minecraft:fireball,limit=1]'
        position = console.vector(shot, 'Pos')
        motion = console.vector(shot, 'Motion')
        owner_data = console(f'data get entity {shot} Owner')
        owner_match = re.search(r'\[I;([^]]+)\]', owner_data)
        assert owner_match and [int(value) for value in owner_match[1].split(',')] == expected, (label, owner_data)
        assert position[:2] == saved_position[:2] == [8.5, 101.6], (label, position)
        assert motion[:2] == saved_motion[:2] == [0.0, 0.0] and motion[2] > 0, (label, motion)
        assert position[2] >= saved_position[2], (label, position)
        allowed = []
        z, speed = saved_position[2], saved_motion[2]
        # Recorded startups advance at most four ordinary flight ticks.
        for _ in range(5):
            allowed.append((z, speed))
            speed = (speed + acceleration) * 0.95
            z += speed
        assert any(math.isclose(position[2], z, rel_tol=1e-6, abs_tol=1e-6)
                   and math.isclose(motion[2], speed, rel_tol=1e-6, abs_tol=1e-6)
                   for z, speed in allowed), (label, 'startup flight exceeded four ticks', position, motion)
        console(f'summon minecraft:ghast 8.5 100 {position[2] + 6} {{NoAI:1b,Tags:["projectile-fixture"]}}')
        console(f'summon minecraft:pig 10.5 101.5 {position[2] + 4} '
                '{NoAI:1b,Health:1.0f,Tags:["projectile-fixture"]}')
        assert console.count('minecraft:ghast', 'projectile-fixture') == 1
        assert console.count('minecraft:pig', 'projectile-fixture') == 1
        ghast = console.vector('@e[type=minecraft:ghast,tag=projectile-fixture,limit=1]', 'Pos')
        pig = console.vector('@e[type=minecraft:pig,tag=projectile-fixture,limit=1]', 'Pos')
        types = json.loads((lab.RESOURCES / 'entity-types.json').read_text())
        ghast_near = ghast[2] - types['minecraft:ghast']['width'] / 2
        assert ghast_near > position[2] + types['minecraft:fireball']['width'] / 2, (label, 'ghast overlaps shot')
        assert pig[0] - types['minecraft:pig']['width'] / 2 > position[0] + 0.3, (label, 'pig intersects ray')
        impact = [position[0], position[1], ghast_near - 0.3]
        assert math.dist(pig, impact) < 4.0, (label, 'pig outside blast', pig, impact)
        console.step(30)
        assert console.count('minecraft:ghast', 'projectile-fixture') == 0, (label, 'reloaded shot did not kill ghast')
        assert console.count('minecraft:pig', 'projectile-fixture') == 0, (label, 'reloaded blast did not kill pig')
        console.save()
        kills = []
        for client in (owner, bystander):
            stats = client.statistics(server).get('minecraft:killed', {})
            kills.append([stats.get('minecraft:ghast', 0), stats.get('minecraft:pig', 0)])
        assert kills == [[1, 1], [0, 0]], (label, 'reloaded shot credited to the wrong player', kills)
        console('fill 7 99 14 9 99 18 minecraft:stone')
        owner.commands(server, ['tp @s 8.5 100 8.5 0 8'])
        credit = []
        cow_kills = []
        experience = []
        for delay, death in [(0, 'generic'), (100, 'generic'), (101, 'generic'),
                             (100, 'void'), (100, 'wither')]:
            console('summon minecraft:cow 8.5 100 16.5 {NoAI:1b,Tags:["credit-fixture"]}')
            assert console.count('minecraft:cow', 'credit-fixture') == 1
            health = console('data get entity @e[type=minecraft:cow,tag=credit-fixture,limit=1] Health')
            assert re.search(r'10\.0f\b', health), health
            console('summon minecraft:fireball 8.5 101.6 10.5 {ExplosionPower:0b}')
            entities = owner.release.with_name(owner.release.name + '.entities')
            wait_for(lambda: entities.exists() and 'minecraft:fireball' in entities.read_text(),
                     'client did not observe the credit fixture fireball')
            owner.commands(server, ['@attack minecraft:fireball'])
            console.save()
            saved = saved_projectiles(directory / 'world', 'minecraft:fireball')
            assert len(saved) == 1 and scalar(saved[0].get('Owner')) == expected, (label, saved)
            for _ in range(20):
                console.step(1)
                health = console('data get entity @e[type=minecraft:cow,tag=credit-fixture,limit=1] Health')
                if re.search(r'4\.0f\b', health):
                    break
            assert re.search(r'4\.0f\b', health), health
            if death == 'wither':
                console.step(delay - 11)
                console('damage @e[type=minecraft:cow,tag=credit-fixture,limit=1] 3 minecraft:generic')
                health = console('data get entity @e[type=minecraft:cow,tag=credit-fixture,limit=1] Health')
                assert re.search(r'1\.0f\b', health), health
                console.step(11)
            elif delay:
                console.step(delay)
            console('kill @e[type=minecraft:experience_orb]')
            assert console.count('minecraft:experience_orb') == 0
            if death == 'void':
                console('tp @e[type=minecraft:cow,tag=credit-fixture,limit=1] 8.5 -130 16.5')
            elif death == 'wither':
                console('effect give @e[type=minecraft:cow,tag=credit-fixture,limit=1] minecraft:wither 1 5')
            else:
                console('damage @e[type=minecraft:cow,tag=credit-fixture,limit=1] 100 minecraft:generic')
            console.step(25)
            experience.append(console.count('minecraft:experience_orb') > 0)
            assert console.count('minecraft:cow', 'credit-fixture') == 0
            console.save()
            stats = owner.statistics(server)
            credit.append(stats.get('minecraft:custom', {}).get('minecraft:mob_kills', 0))
            cow_kills.append(stats.get('minecraft:killed', {}).get('minecraft:cow', 0))
            other = bystander.statistics(server)
            assert other.get('minecraft:custom', {}).get('minecraft:mob_kills', 0) == 0
        assert credit == [3, 4, 4, 5, 5], (label, 'player damage memory boundary differs', credit)
        assert cow_kills == [0, 0, 0, 0, 0], (label, 'environmental death awarded a direct kill', cow_kills)
        assert experience == [True, False, False, False, False], (label, 'experience outlives damage memory', experience)
        return {'owner': expected, 'kills': kills, 'damage_memory': credit,
                'cow_kills': cow_kills, 'experience': experience}


def damage_owner_case(label, command, world, build, classes, dependencies, kind, hold=0):
    directory = build / f'{label}-{kind}'
    port = server_dir(directory, world)
    name = 'ProjectileGuest'
    expected = list(struct.unpack('>4i', uuid.UUID(offline_uuid(name)).bytes))
    cow = '@e[type=minecraft:cow,tag=reconnect-credit-fixture,distance=..1024,limit=1]'
    observed = {'case': kind, 'side': label, 'hold_seconds': hold}
    dimension = kind in ('dimension', 'dimension_flight')
    sentinel = '@e[type=minecraft:cow,tag=dimension-restoration-fixture,distance=..1024,limit=1]'
    dimensions = [('overworld', 'minecraft:overworld'), ('nether', 'minecraft:the_nether')]

    def counters(client, server):
        stats = client.statistics(server)
        return [stats.get('minecraft:custom', {}).get('minecraft:mob_kills', 0),
                stats.get('minecraft:killed', {}).get('minecraft:cow', 0)]

    def score(console, holder, objective='reconnect_kills'):
        reply = console(f'scoreboard players get {holder} {objective}')
        match = re.search(rf'\b{re.escape(holder)} has (-?\d+) \[{re.escape(objective)}\]', reply)
        assert match, reply
        return int(match[1])

    def health(console, wanted):
        reply = console.world(f'data get entity {cow} Health')
        assert re.search(rf'\b{wanted}\.0f\b', reply), reply

    def xp(client, server, console):
        client.commands(server, ['execute store success score @s dim_reward_ok '
                                 'store result score @s dim_reward run experience query @s points'])
        assert score(console, client.name, 'dim_reward_ok') == 1
        return score(console, client.name, 'dim_reward')

    def wire_uuid(client):
        matches = re.findall(r'^login minecraft:login_finished ([0-9a-f-]+) '
                             + re.escape(client.name) + r' ', client.output.read_text(), re.M)
        assert len(matches) == 1, (client.output, matches)
        return str(uuid.UUID(matches[0]))

    try:
        if dimension:
            command, data_version = dimension_inputs(directory, build, label, command, dependencies)
            for holder in (name, 'ProjectileOther'):
                assert not (directory / 'world/players/advancements' / (offline_uuid(holder) + '.json')).exists()
        with running(command, directory, f'{label}-{kind}') as (server, console, clients):
            console('tick freeze')
            bystander = join(server, console, clients, port, 'ProjectileOther', build, classes, dependencies,
                             f'{label}-{kind}-other')
            bystander.commands(server, ['tp @s 2.5 100 2.5'])
            owner = join(server, console, clients, port, name, build, classes, dependencies,
                         f'{label}-{kind}-first')
            first_owner = owner
            owner.commands(server, ['tp @s 8.5 100 8.5 0 8'])
            baseline = counters(owner, server)
            assert baseline == counters(bystander, server) == [0, 0], (label, kind, baseline)
            if dimension:
                console('scoreboard objectives add dim_reward dummy')
                console('scoreboard objectives add dim_reward_ok dummy')
                observed['reward_baseline'] = [xp(owner, server, console), xp(bystander, server, console)]
                assert observed['reward_baseline'] == [0, 0], observed
            console('scoreboard objectives add reconnect_kills totalKillCount')
            for holder in (name, 'ProjectileOther', 'Player'):
                console(f'scoreboard players set {holder} reconnect_kills 0')
                assert score(console, holder) == 0
            console('fill 7 99 14 9 99 18 minecraft:stone')
            console('summon minecraft:cow 8.5 100 16.5 {NoAI:1b,Tags:["reconnect-credit-fixture"]}')
            assert console.count('minecraft:cow', 'reconnect-credit-fixture') == 1
            health(console, 10)
            observed['cow_uuid'] = console.entity_uuid(cow)
            if dimension:
                console('summon minecraft:cow 12.5 100 20.5 '
                        '{NoAI:1b,NoGravity:1b,Tags:["dimension-restoration-fixture"]}')
                observed['sentinel_uuid'] = console.entity_uuid(sentinel)
            console('summon minecraft:fireball 8.5 101.6 10.5 {ExplosionPower:0b}')
            entities = owner.release.with_name(owner.release.name + '.entities')
            wait_for(lambda: entities.exists() and 'minecraft:fireball' in entities.read_text(),
                     'owner did not observe the damage memory fireball')
            owner.commands(server, ['@attack minecraft:fireball'])
            observed['shot_owner'] = console.entity_uuid('@e[type=minecraft:fireball,limit=1]', 'Owner')
            assert observed['shot_owner'] == expected, (label, kind, observed)
            if kind == 'dimension_flight':
                observed['flight_time_before'] = console.game_time()
                owner.commands(server, ['execute in minecraft:the_nether run tp @s 8.5 100 8.5'])
                wait_for(lambda: 'Test passed' in console(
                    f'execute as {name} at @s if dimension minecraft:the_nether'),
                    'owner did not move before impact')
                console.dimension = 'minecraft:overworld'
                observed['flight_time_after'] = console.game_time()
                assert observed['flight_time_after'] == observed['flight_time_before'], observed
                health(console, 10)
                assert console.entity_uuid(cow) == observed['cow_uuid'], observed
                shot = '@e[type=minecraft:fireball,distance=..1024,limit=1]'
                observed['flight_owner'] = console.entity_uuid(shot, 'Owner')
                assert observed['flight_owner'] == expected, observed
                reply = console.world(f'data get entity {shot} Pos')
                observed['flight_position'] = [float(value) for value in re.findall(
                    r'(-?\d+(?:\.\d+)?(?:[Ee][+-]?\d+)?)d', reply)]
                assert observed['flight_position'] == [8.5, 101.6, 10.5], observed
            for _ in range(20):
                console.step(1)
                reply = console.world(f'data get entity {cow} Health')
                if re.search(r'4\.0f\b', reply):
                    break
            health(console, 4)
            assert counters(owner, server) == baseline
            observed['frozen_time_before'] = console.game_time()
            transition_start = time.monotonic()
            if kind in ('reconnect', 'offline'):
                clients.remove(owner)
                owner.close()
                wait_for(lambda: any(name + ' left the game' in line for line in server.lines),
                         'owner departure was not processed')
                listing = console('list')
                assert 'ProjectileOther' in listing and name not in listing, listing
                if kind == 'reconnect':
                    owner = join(server, console, clients, port, name, build, classes, dependencies,
                                 f'{label}-{kind}-new')
                    assert owner.process.pid != first_owner.process.pid
            elif kind == 'connected':
                time.sleep(hold)
            elif kind == 'dimension':
                owner.commands(server, ['execute in minecraft:the_nether run tp @s 8.5 100 8.5'])
                wait_for(lambda: 'Test passed' in console(
                    f'execute as {name} at @s if dimension minecraft:the_nether'),
                    'owner did not move to the Nether')
                console.dimension = 'minecraft:overworld'
            elif kind == 'dimension_flight':
                assert 'Test passed' in console(
                    f'execute as {name} at @s if dimension minecraft:the_nether')
            else:
                raise AssertionError(kind)
            observed['wait_seconds'] = round(time.monotonic() - transition_start, 3)
            if kind != 'offline':
                baseline = counters(owner, server)
                assert baseline == [0, 0], (label, kind, baseline)
                assert owner.process.poll() is None
                assert name in console('list')
            observed['frozen_time_after'] = console.game_time()
            assert observed['frozen_time_after'] == observed['frozen_time_before'], observed
            health(console, 4)
            assert console.entity_uuid(cow) == observed['cow_uuid'], observed
            loaded = console.world('execute if loaded 8 100 16')
            assert 'Test passed' in loaded, loaded
            assert bystander.process.poll() is None
            if dimension:
                observed['clock_before'] = {}
                observed['dimension_time_before'] = {key: console.game_time(dimension) for key, dimension in dimensions}
                assert all(value == observed['frozen_time_after'] for value in observed['dimension_time_before'].values())
                for key, dimension in dimensions:
                    wait_for(lambda: 'Test passed' in console(f'execute in {dimension} if loaded 12 100 12'),
                             dimension + ' clock witness chunk did not load')
                    console(f'execute in {dimension} run summon minecraft:item 12.5 100 12.5 '
                            '{Item:{id:"minecraft:stone",count:1},Age:0s,PickupDelay:32767s,NoGravity:1b,'
                            'Tags:["dimension-clock-fixture"]}')
                    observed['clock_before'][key] = console.item_clock(dimension)
                    assert observed['clock_before'][key]['age'] == 0
                assert observed['clock_before']['overworld']['uuid'] != observed['clock_before']['nether']['uuid']
            console.world('kill @e[type=minecraft:experience_orb]')
            assert console.count('minecraft:experience_orb') == 0
            console.world(f'damage {cow} 100 minecraft:generic')
            console.step(25)
            assert console.game_time() == observed['frozen_time_after'] + 25
            assert console.count('minecraft:cow', 'reconnect-credit-fixture') == 0
            observed['experience'] = console.count('minecraft:experience_orb') > 0
            observed['bystander'] = counters(bystander, server)
            observed['scores'] = [score(console, holder) for holder in (name, 'Player', 'ProjectileOther')]
            if kind == 'offline':
                owner = join(server, console, clients, port, name, build, classes, dependencies,
                             f'{label}-{kind}-late')
                assert owner.process.pid != first_owner.process.pid
            observed['owner_after'] = counters(owner, server)
            observed['guest_delta'] = observed['owner_after'][0] - baseline[0]
            assert owner.process.poll() is None
            if dimension:
                observed['clock_after'] = {key: console.item_clock(dimension) for key, dimension in dimensions}
                observed['dimension_time_after'] = {key: console.game_time(dimension) for key, dimension in dimensions}
                for key, _ in dimensions:
                    assert observed['dimension_time_after'][key] == observed['dimension_time_before'][key] + 25, observed
                    assert observed['clock_after'][key]['uuid'] == observed['clock_before'][key]['uuid'], observed
                    assert observed['clock_after'][key]['age'] == 25, observed
                observed['reward_points'] = xp(owner, server, console)
                observed['bystander_points'] = xp(bystander, server, console)
                assert observed['owner_after'] == [1, 0] and observed['guest_delta'] == 1, observed
                assert observed['scores'] == [1, 0, 0] and observed['bystander'] == [0, 0], observed
                assert observed['experience'] is True, observed
                assert observed['reward_points'] == 5 and observed['bystander_points'] == 0, observed
                assert console.entity_uuid(sentinel) == observed['sentinel_uuid'], observed
                reply = console.world(f'data get entity {sentinel} Health')
                assert re.search(r'\b10\.0f\b', reply), reply
                assert 'Test passed' in console(f'execute as {name} at @s if dimension minecraft:the_nether')
                assert 'Test passed' in console('execute as ProjectileOther at @s if dimension minecraft:overworld')
                assert console.game_time('minecraft:overworld') == observed['dimension_time_after']['overworld']
                observed['restoration_checked'] = True
        observed['wire_uuid_before'] = wire_uuid(first_owner)
        observed['wire_uuid_after'] = wire_uuid(owner)
        assert observed['wire_uuid_before'] == observed['wire_uuid_after'] == offline_uuid(name), observed
        assert observed['shot_owner'] == list(struct.unpack('>4i', uuid.UUID(observed['wire_uuid_after']).bytes))
        wanted = 0 if kind == 'offline' else 1
        assert observed['experience'] is True, (label, kind, observed)
        assert observed['bystander'] == [0, 0], (label, kind, observed)
        assert observed['owner_after'] == [wanted, 0], (label, kind, 'guest credit differs', observed)
        assert observed['guest_delta'] == wanted, (label, kind, observed)
        assert observed['scores'] == [wanted, 0, 0], (label, kind, 'owner, host, bystander credit differs', observed)
        result = {key: observed[key] for key in ('guest_delta', 'owner_after', 'scores', 'bystander', 'experience')}
        if dimension:
            observed['bystander_wire_uuid'] = wire_uuid(bystander)
            assert observed['bystander_wire_uuid'] == offline_uuid(bystander.name), observed
            observed['advancements'] = saved_advancements(directory / 'world', owner, data_version)
            observed['bystander_advancements'] = saved_advancements(directory / 'world', bystander, data_version)
            assert observed['advancements'] == {'any': True, 'nether': True, 'overworld': False}, observed
            assert observed['bystander_advancements'] == {'any': False, 'nether': False, 'overworld': False}, observed
            observed['saved_xp'] = []
            for client, dimension, total in [(owner, 'minecraft:the_nether', 5),
                                              (bystander, 'minecraft:overworld', 0)]:
                profile = uuid.UUID(wire_uuid(client))
                saved = anvil_dump.read_dat(directory / 'world/players/data' / (str(profile) + '.dat'))
                assert scalar(saved['UUID']) == list(struct.unpack('>4i', profile.bytes)), observed
                assert scalar(saved['Dimension']) == dimension, observed
                assert scalar(saved['XpTotal']) == total and scalar(saved['XpLevel']) == 0, observed
                observed['saved_xp'].append(scalar(saved['XpTotal']))
            result.update(advancements=observed['advancements'], reward_points=observed['reward_points'],
                          clock_ages=[observed['clock_after'][key]['age'] for key, _ in dimensions],
                          restoration_checked=observed['restoration_checked'])
        return result
    finally:
        (build / f'{label}-{kind}-observations.json').write_text(json.dumps(observed, indent=2) + '\n')



def dimension_blast_case(label, command, world, build, classes, dependencies):
    directory = build / f'{label}-dimension-blast'
    port = server_dir(directory, world)
    names = ('ProjectileGuest', 'ProjectileOther')
    observed = {'side': label}
    tag = 'dimension-blast-fixture'
    shot = '@e[type=minecraft:fireball,distance=..1024,limit=1]'

    def score(console, name, objective):
        response = console(f'scoreboard players get {name} {objective}')
        match = re.search(rf'\b{re.escape(name)} has (-?\d+) \[{re.escape(objective)}\]', response)
        assert match, response
        return int(match[1])

    def health(client, server, console):
        client.commands(server, ['execute store success score @s flight_health_ok '
                                 'store result score @s flight_health run data get entity @s Health 1000'])
        assert score(console, client.name, 'flight_health_ok') == 1
        return score(console, client.name, 'flight_health')

    def stats(client, server):
        value = client.statistics(server)
        return [value.get('minecraft:custom', {}).get('minecraft:mob_kills', 0),
                value.get('minecraft:killed', {}).get('minecraft:cow', 0),
                value.get('minecraft:killed', {}).get('minecraft:pig', 0)]

    try:
        with running(command, directory, f'{label}-dimension-blast') as (server, console, clients):
            console('tick freeze')
            console('gamerule natural_health_regeneration false')
            console('gamerule spawn_mobs false')
            other = join(server, console, clients, port, names[1], build, classes, dependencies,
                         f'{label}-dimension-blast-other')
            owner = join(server, console, clients, port, names[0], build, classes, dependencies,
                         f'{label}-dimension-blast-owner')
            console('fill 3 99 6 14 99 22 minecraft:bedrock')
            owner.commands(server, ['tp @s 8.5 100 8.5 0 8'])
            other.commands(server, ['tp @s 6.5 100 16.5', 'gamemode survival @s',
                                     'effect give @s minecraft:resistance 60 2 true'])
            for objective in ['flight_health', 'flight_health_ok']:
                console(f'scoreboard objectives add {objective} dummy')
            console('scoreboard objectives add flight_kills totalKillCount')
            for name in (*names, 'Player'):
                console(f'scoreboard players set {name} flight_kills 0')
                assert score(console, name, 'flight_kills') == 0
            assert stats(owner, server) == stats(other, server) == [0, 0, 0]
            assert health(owner, server, console) == health(other, server, console) == 20000
            console(f'summon minecraft:cow 8.5 100 16.5 {{NoAI:1b,Tags:["{tag}"]}}')
            console(f'summon minecraft:pig 10.5 100 16.5 {{NoAI:1b,Health:1.0f,Tags:["{tag}"]}}')
            assert console.count('minecraft:cow', tag) == console.count('minecraft:pig', tag) == 1
            console('summon minecraft:fireball 8.5 101.6 10.5 {ExplosionPower:2b}')
            entities = owner.release.with_name(owner.release.name + '.entities')
            wait_for(lambda: entities.exists() and 'minecraft:fireball' in entities.read_text(),
                     'owner did not observe the blast fireball')
            owner.commands(server, ['@attack minecraft:fireball'])
            expected = list(struct.unpack('>4i', uuid.UUID(offline_uuid(owner.name)).bytes))
            observed['shot_owner'] = console.entity_uuid(shot, 'Owner')
            assert observed['shot_owner'] == expected
            observed['time_before'] = console.game_time()
            owner.commands(server, ['execute in minecraft:the_nether run tp @s 8.5 100 15.5'])
            wait_for(lambda: 'Test passed' in console(
                f'execute as {owner.name} at @s if dimension minecraft:the_nether'),
                'blast owner did not move to the Nether')
            for z in (6, 22):
                wait_for(lambda z=z: 'Test passed' in console(
                    f'execute in minecraft:the_nether if loaded 8 100 {z}'),
                    'remote owner floor chunks did not load')
            owner.commands(server, ['fill 3 99 6 14 99 22 minecraft:bedrock',
                                    'fill 3 100 6 14 104 22 minecraft:air', 'gamemode survival @s',
                                    'damage @s 1 minecraft:generic'])
            assert 'Test passed' in console(
                'execute in minecraft:the_nether if block 8 99 15 minecraft:bedrock')
            for y in (100, 101, 102):
                assert 'Test passed' in console(
                    f'execute in minecraft:the_nether if block 8 {y} 15 minecraft:air')
            observed['owner_canary_health'] = health(owner, server, console)
            assert observed['owner_canary_health'] == 19000, observed
            assert 'Test passed' in console(
                f'execute as {owner.name} if entity @s[gamemode=survival]')
            assert 'Test passed' in console(
                f'execute as {other.name} at @s if dimension minecraft:overworld')
            console.dimension = 'minecraft:overworld'
            assert console.entity_uuid(shot, 'Owner') == expected
            assert console.game_time() == observed['time_before'], observed
            assert console.count('minecraft:cow', tag) == console.count('minecraft:pig', tag) == 1
            observed['other_before'] = health(other, server, console)
            assert observed['other_before'] == 20000
            console.step(30)
            observed['time_after'] = console.game_time()
            assert observed['time_after'] == observed['time_before'] + 30, observed
            assert console.count('minecraft:cow', tag) == console.count('minecraft:pig', tag) == 0
            observed['owner_after_health'] = health(owner, server, console)
            observed['other_after_health'] = health(other, server, console)
            observed['owner_stats'] = stats(owner, server)
            observed['other_stats'] = stats(other, server)
            observed['scores'] = [score(console, name, 'flight_kills') for name in (*names, 'Player')]
            assert observed['owner_after_health'] == observed['owner_canary_health'], observed
            assert 0 < observed['other_after_health'] < observed['other_before'], observed
            assert observed['owner_stats'] == [2, 1, 1], observed
            assert observed['other_stats'] == [0, 0, 0] and observed['scores'] == [2, 0, 0], observed
            assert 'Test passed' in console(
                f'execute as {owner.name} at @s if dimension minecraft:the_nether')
            assert 'Test passed' in console(
                f'execute as {other.name} at @s if dimension minecraft:overworld')
        observed['wire_profiles'] = {}
        for client in (owner, other):
            profiles = re.findall(r'^login minecraft:login_finished ([0-9a-f-]+) '
                                  + re.escape(client.name) + r' ', client.output.read_text(), re.M)
            assert profiles == [offline_uuid(client.name)], profiles
            observed['wire_profiles'][client.name] = profiles[0]
        assert observed['shot_owner'] == list(struct.unpack(
            '>4i', uuid.UUID(observed['wire_profiles'][owner.name]).bytes))
        return {'owner_unharmed': True, 'other_hurt': True, 'owner_stats': observed['owner_stats'],
                'other_stats': observed['other_stats'], 'scores': observed['scores']}
    finally:
        (build / f'{label}-dimension-blast-observations.json').write_text(json.dumps(observed, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--world', type=Path)
    parser.add_argument('--output', type=Path, default=lab.LOCAL / 'projectile-persistence-parity')
    parser.add_argument('--side', choices=['both', 'vanilla', 'horizon'], default='both')
    parser.add_argument('--case', choices=['all', 'unload', 'owner', 'reconnect', 'connected', 'offline',
                                         'dimension', 'dimension_flight', 'dimension_blast'], default='all')
    args = parser.parse_args()
    build = args.output.resolve()
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes, dependencies = compile_client(build)
    vanilla = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    binary = args.binary.resolve() if args.binary else None
    if binary is None and args.side != 'vanilla':
        subprocess.run(['cargo', 'build', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'],
                       cwd=lab.ROOT, check=True, capture_output=True)
        binary = lab.cargo_target() / 'release' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    horizon = [binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    world = args.world or generate_world(vanilla, build)
    commands = [(label, command) for label, command in [('vanilla', vanilla), ('horizon', horizon)]
                if args.side in ('both', label)]
    results = {label: {} for label, _ in commands}
    reconnect_waits = []
    for name, case in [('unload', unload_case), ('owner', owner_case), ('reconnect', None),
                       ('offline', None), ('dimension', None), ('dimension_flight', None),
                       ('dimension_blast', dimension_blast_case), ('connected', None)]:
        if args.case in ('all', name):
            for label, command in commands:
                if case is None:
                    hold = max(reconnect_waits, default=9) + 1 if name == 'connected' else 0
                    results[label][name] = damage_owner_case(label, command, world, build, classes, dependencies,
                                                             name, hold)
                    observation = json.loads((build / f'{label}-{name}-observations.json').read_text())
                    if name == 'reconnect':
                        reconnect_waits.append(observation['wait_seconds'])
                    elif name == 'connected':
                        assert observation['wait_seconds'] >= hold > max(reconnect_waits, default=0), observation
                else:
                    results[label][name] = case(label, command, world, build, classes, dependencies)
    if args.side == 'both':
        assert results['horizon'] == results['vanilla'], results
    report.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, sort_keys=True), flush=True)


if __name__ == '__main__':
    main()
