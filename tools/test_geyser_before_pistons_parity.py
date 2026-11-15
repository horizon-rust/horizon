#!/usr/bin/env python3
"""A geyser pushes before the tick's moving pistons. Dedicated servers.

`Level.tickBlockEntities` runs the tickers in the order they joined the list. A potent sulfur block's
(`PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER`) is there from its chunk's load; a moving piston's joins when its
move starts. So a geyser adds its 0.2 to an entity's motion first, and a piston's push of the same tick comes after:
when the entity is held by a cobweb (`Entity.move` zeroes the motion of an entity with a `stuckSpeedMultiplier`,
even for a piston's move), the push wipes the geyser's 0.2 out.

A vanilla dedicated server and Horizon's each load a copy of one flat world vanilla generated first. Ticking is
frozen. Each lane is a continuous geyser (lava, potent sulfur, a water block between glass) with a cobweb over the
water, and a primed TNT without gravity in the cobweb, half over the column. In one lane a piston, powered as the
lane is built, pushes a stone block against the TNT, keeping it over the column; the control lane has no piston.
A third lane has the order the other way round: its piston starts moving in the first step and its geyser is placed
after that step, so the geyser's ticker joins the list after the piston's and its 0.2 comes after the push of the
second step. The servers are stepped; after every step each TNT's exact Pos and Motion are read with `data get`. The
transcripts must match vanilla's (the oracle) line for line, and vanilla's must show the push wiping the geyser's
motion out in the second lane and keeping it in the third.
"""
import re
import sys
import time

try:
    from . import test_locator_bar_parity as locator
    from .test_dedicated_server_parity import Server
except ImportError:
    import test_locator_bar_parity as locator
    from test_dedicated_server_parity import Server

NAME = 'geyser-before-pistons-parity'
TICKS = 4
# tag, its lane's x, a piston
LANES = [('control', 4, False), ('pushed', 12, True), ('late', 20, True)]
# The lanes whose geyser is placed after the first step, once their piston is moving.
LATE = {'late'}
FORCELOAD = 'forceload add 0 0 31 31'
NUMBER = re.compile(r'(-?[0-9][0-9.E-]*)[df]')


def geyser(x):
    return f'setblock {x} -60 8 minecraft:potent_sulfur[potent_sulfur_state=continuous]'


def lane(x, piston, late=False):
    """Lava in the floor at (x, -61, 8), the sulfur on it (stone until later for a late lane), a water block
    between four glass blocks, a cobweb on it, and a piston at x + 3 facing west that pushes a stone block at
    x + 2 into x + 1, powered by a redstone block behind it."""
    glass = [f'setblock {x + dx} -59 {8 + dz} minecraft:glass' for dx, dz in ((1, 0), (-1, 0), (0, 1), (0, -1))]
    lines = [f'setblock {x} -61 8 minecraft:lava', f'setblock {x} -60 8 minecraft:stone', *glass,
             f'setblock {x} -59 8 minecraft:water', f'setblock {x} -58 8 minecraft:cobweb']
    if not late:
        lines.append(geyser(x))
    if piston:
        lines += [f'setblock {x + 2} -58 8 minecraft:stone', f'setblock {x + 3} -58 8 minecraft:piston[facing=west]',
                  f'setblock {x + 4} -58 8 minecraft:redstone_block']
    return lines


BLOCKS = ['time set 6000', *[line for tag, x, piston in LANES for line in lane(x, piston, tag in LATE)]]
AFTER_FIRST_STEP = [geyser(x) for tag, x, _piston in LANES if tag in LATE]
ENTITIES = [f'summon minecraft:tnt {x + 1}.0 -58.0 8.5 {{fuse:200s,NoGravity:1b,Motion:[0.0d,0.0d,0.0d],Tags:["{tag}"]}}'
            for tag, x, _piston in LANES]


def run(label, command, world, build):
    directory = build / label
    locator.server_dir(directory, world)
    server = Server(command, directory, label)
    lines = []
    try:
        server.wait_for(r'Done \(', timeout=600)
        server.console('tick freeze')
        server.console(FORCELOAD)
        server.console('say READY')
        server.wait_for(r'READY', timeout=120)
        time.sleep(3)
        for command_line in BLOCKS:
            server.console(command_line)
        time.sleep(1)
        for command_line in ENTITIES:
            server.console(command_line)
        time.sleep(1)
        for tick in range(TICKS + 1):
            if tick:
                server.console('tick step 1')
                time.sleep(1.0)
            if tick == 1:
                for command_line in AFTER_FIRST_STEP:
                    server.console(command_line)
                time.sleep(0.5)
            start = len(server.lines)
            for tag, *_rest in LANES:
                for path in ('Pos', 'Motion'):
                    server.console(f'data get entity @e[tag={tag},limit=1] {path}')
            server.console(f'say END {tick}')
            server.wait_for(rf'END {tick}\b', timeout=120)
            answers = [l.split('INFO]: ', 1)[-1] for l in server.lines[start:]
                       if 'entity data:' in l or 'No entity was found' in l or 'Found no' in l]
            for i, (tag, *_rest) in enumerate(LANES):
                pos, motion = answers[2 * i:2 * i + 2] if len(answers) >= 2 * i + 2 else ('', '')
                if 'entity data:' not in pos:
                    lines.append(f'tick {tick} {tag} gone')
                else:
                    lines.append(f'tick {tick} {tag} pos {" ".join(NUMBER.findall(pos))}'
                                 f' motion {" ".join(NUMBER.findall(motion))}')
    finally:
        server.stop()
    return lines



def proves(vanilla):
    """Why vanilla's transcript does not show what this harness is for, or None."""
    def motion_y(tick, tag):
        line = next(l for l in vanilla if l.startswith(f'tick {tick} {tag} '))
        return float(line.split('motion ')[1].split()[1]) if ' motion ' in line else None
    if not any((motion_y(t, 'control') or 0.0) > 0.0 for t in range(1, TICKS + 1)):
        return 'the geyser did not push the control TNT'
    if not any(motion_y(t, 'pushed') == 0.0 and (motion_y(t, 'control') or 0.0) > 0.0 for t in range(1, TICKS + 1)):
        return "the piston's push never wiped the geyser's motion out"
    # The late geyser's first push is in step 2, after the piston's push of that step.
    if not (motion_y(2, 'late') or 0.0) > 0.0:
        return "the late geyser's motion did not outlast the piston's push"
    return None


def main():
    lock, build, vanilla_server, horizon_server, _client = locator.sides(NAME)
    world = locator.generate_world(vanilla_server, build)
    vanilla = run('vanilla', vanilla_server, world, build)
    (build / 'vanilla.txt').write_text('\n'.join(vanilla) + '\n', encoding='utf-8')
    why = proves(vanilla)
    if why:
        raise AssertionError(f'vanilla is no proof: {why}:\n' + '\n'.join(vanilla))
    horizon = run('horizon', horizon_server, world, build)
    (build / 'horizon.txt').write_text('\n'.join(horizon) + '\n', encoding='utf-8')
    if vanilla != horizon:
        differ = [f'- {v}\n+ {h}' for v, h in zip(vanilla, horizon) if v != h]
        raise AssertionError(f'{len(differ)} lines differ from vanilla:\n' + '\n'.join(differ[:24]))
    print(f'geyser before pistons: {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
