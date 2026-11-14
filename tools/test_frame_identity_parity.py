#!/usr/bin/env python3
"""The picture a change leaves alone stays the same, pixel for pixel (docs/design/hdr-linear-light.md, Tests 1 and 2).

Pictures of the game hold Minecraft's textures and can't be committed, so this test is differential: it builds the base
(BASE_SHA on the parity runner, else the merge-base with origin/main) and the change, draws the same fixed scenes with
each offscreen under HORIZON_STILL=1 (a frozen scene draws the same frame every run, docs/agent-control.md), and
requires the decoded pixels to be identical. The oracle is the base build, not vanilla. It fails any change that moves
the picture on purpose, so no automatic selection runs it (named_only in tools/parity_hosts.json): a pull request that
must leave the picture alone carries the frame-identity label (docs/release-flow.md).

Sessions, each a fresh game and world (seed 7) per build:

- vanilla: HDR off and no shader pack: day over water, night with light sources, stained glass, the Nether,
  entities (a hurt cow, a spider, an armour stand in enchanted armour, an arrow, an End crystal, a chest, a bell,
  a banner, a framed item),
  and the HUD over light: chat over the sun, chat and the crosshair over lava, the pause
  menu over it.
- golden_hour: the Golden Hour example pack in SDR, the same scenes.
- golden_hour_hdr: Golden Hour with offscreen HDR output on (HORIZON_HDR_OUTPUT=pq), on the float frame: each
  screenshot and its -hdr.png.
- hdr_toggle: vanilla started with HDR on (the float frame), shot, HDR turned off from its settings page, shot.
- oit, visbuffer: Improved Transparency, and the visibility buffer (HORIZON_VISBUFFER=1), with HDR off.
- float_hdr, float_oit, float_visbuffer: HDR on, which draws the float frame: forward, with Improved Transparency
  and with the visibility buffer, the last on a 300-nit display with the HUD at half brightness.
- float_unlit: HDR on and the Vanilla look, on that display.
- entity_off, entity_stage, and each with _hdr: the entity stage's fixture pack
  (crates/horizon-shader-api/tests/samples/entity-stage) without and with its [entity] stage, which is vanilla's
  own: the entities, in SDR, and with HDR output on and the block entities kept on the GPU; and items, which the
  stage doesn't draw, under a dynamic light (Horizon Extras' dynamic lights are on in these sessions).
- horizon_hdr, and float_dim and horizon_hdr_dim: Horizon's HDR look drawn by its pack
  (crates/horizon-shader-api/packs/horizon_hdr) with HDR output on, against float_hdr, the look itself; and both
  on a 300-nit display, where the tone curve bends.

The float frame's HDR shots are also held to their SDR ones in each build (hdr_gates, Tests 3 and 4): with the
Vanilla look the picture is HDR off's, and with Horizon's the SDR screenshot is the HDR shot clipped at white, white
HUD shows at UI Brightness and nothing passes the peak.

Both builds draw on the same adapter: Mesa's lavapipe where its Vulkan driver is installed (Linux), else the machine's
default. When the base and the change differ only in files no game binary is built from (tools/, tests/, .github/,
Markdown outside crates/), the base reuses the change's binaries: the comparison is then of two runs of one build.
Each session also checks it drew what it is for (the pack changes the picture, HDR shots have their -hdr.png), so a
pack that fails to load, or HDR that stays off, in both builds doesn't pass as identical.
In each build the fixture's entity stage must draw the entities as the pack without it does, within the rounding of
the decode and encode around the stage, and leave what it doesn't draw exactly as it is (entity_gates).
And in each build Horizon's HDR look drawn by its pack must be the look the game draws without a pack, within the
rounding of the pack's route, where the pack reaches (look_gates).
--build-base builds it anyway. The base builds in a worktree and target folder of its own under .local/frame-identity/,
kept between runs so that a later base rebuilds only what changed.

On a difference it writes diff-<scene>.png beside the two shots: the base dimmed to grey, differing pixels in red.
--sessions draws some of the sessions only. One that a gate holds to another's shots (the look's pack to the look,
the Vanilla look to HDR off) isn't drawn without that one.

  test_frame_identity_parity.py [--base REV] [--build-base] [--bin-dir DIR] [--base-bin-dir DIR] [--sessions a,b]
"""
import argparse
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = lab.ROOT
RUN = lab.LOCAL / 'frame-identity'
BASE_SRC = RUN / 'base-src'
BASE_TARGET = RUN / 'target-base'
EXE = '.exe' if os.name == 'nt' else ''
SIZE = (854, 480)
SEED = '7'
# 🤖 Agent: changes under these, and Markdown outside crates/, build no game binary, so the base can reuse the
# change's. Not all of docs/: horizon-port-pack embeds docs/modpacks/atm10-mods.csv. Not all of tools/ either:
# horizon-mod-api-gen generates Rust from tools/native-hook-fragments/ (BUILT_FROM_TOOLS).
NO_BINARY = ('tools/', 'tests/', '.github/')
BUILT_FROM_TOOLS = ('tools/native-hook-fragments/',)
BUILD = ['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib', '--bins']
# 🤖 Agent: Where the overworld scenes are built, high above seed 7's spawn, so terrain below hardly shows.
X, Y, Z = 8, 160, 8
# 🤖 Agent: The entities' corner, out of every other viewpoint's sight.
EX, EZ = X - 28, Z - 30
# 🤖 Agent: And the items' corner, as far on the other side.
IX, IZ = X + 30, Z - 30
# 🤖 Agent: The packs sessions draw with: where each is, and the table its manifest is cut at with the file it then
# has no use for (the entity stage's fixture without its stage and the keys only that stage reads).
SAMPLES = ROOT / 'crates/horizon-shader-api/tests/samples'
PACKS = {'golden_hour': (ROOT / 'crates/horizon-shader-api/packs/golden_hour', None, None),
         'horizon_hdr': (ROOT / 'crates/horizon-shader-api/packs/horizon_hdr', None, None),
         'entity_stage': (SAMPLES / 'entity-stage', None, None),
         'entity_off': (SAMPLES / 'entity-stage', '\n[entity]', 'vanilla.wgsl')}


def git(*args, cwd=ROOT):
    return subprocess.run(['git', *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def base_revision(requested):
    rev = requested or os.environ.get('BASE_SHA') or git('merge-base', 'HEAD', 'origin/main')
    return git('rev-parse', f'{rev}^{{commit}}')


def pack_at(base, name):
    """Whether revision `base` has the pack session `name` draws with (true for a session without one)."""
    pack = SESSIONS[name][1].get('HORIZON_SHADER_PACK')
    if pack is None:
        return True
    manifest = (PACKS[pack][0] / 'horizon-shaders.toml').relative_to(ROOT).as_posix()
    return subprocess.run(['git', 'cat-file', '-e', f'{base}:{manifest}'], cwd=ROOT,
                          capture_output=True).returncode == 0


def needs_own_build(base):
    """Whether the base can't reuse the change's binaries: it differs in a file a game binary may be built from."""
    changed = [p for p in git('diff', '--name-only', base, 'HEAD').splitlines() if p]
    return [p for p in changed if not ((p.startswith(NO_BINARY) and not p.startswith(BUILT_FROM_TOOLS))
                                       or (p.endswith('.md') and not p.startswith('crates/')))]


def checkout_base(base):
    """The base's sources in a worktree of their own, moved to `base` in place, so that only the files that changed
    since the last run get new times and the base's target folder rebuilds incrementally."""
    git('worktree', 'prune')
    listed = git('worktree', 'list', '--porcelain')
    if BASE_SRC.exists() and f'worktree {BASE_SRC.as_posix()}' not in listed.replace('\\', '/'):
        shutil.rmtree(BASE_SRC)
    if not BASE_SRC.exists():
        BASE_SRC.parent.mkdir(parents=True, exist_ok=True)
        git('worktree', 'add', '--detach', '--force', str(BASE_SRC), base)
    else:
        git('checkout', '--detach', '--force', base, cwd=BASE_SRC)
        git('clean', '-fdq', cwd=BASE_SRC)


def build(cwd, target, jobs):
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    command = BUILD + (['-j', str(jobs)] if jobs else [])
    started = time.monotonic()
    subprocess.run(command, cwd=cwd, env=env, check=True)
    print(f'built {cwd} in {time.monotonic() - started:.0f} s', flush=True)
    return target / 'fast'


def lavapipe():
    """Mesa's software Vulkan driver's manifest, where it is installed."""
    for folder in ('/usr/share/vulkan/icd.d', '/usr/local/share/vulkan/icd.d', '/etc/vulkan/icd.d'):
        for path in sorted(Path(folder).glob('lvp_icd*.json')) if Path(folder).is_dir() else []:
            return path
    return None


# 🤖 Agent: PNG, with the standard library only.

def read_png(path):
    """Width, height, bytes per pixel, filtered stream, and (depth, colour type)."""
    data = Path(path).read_bytes()
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError(f'{path}: not a PNG')
    at, idat, header = 8, [], None
    while at < len(data):
        length, kind = struct.unpack('>I4s', data[at:at + 8])
        body = data[at + 8:at + 8 + length]
        if kind == b'IHDR':
            header = struct.unpack('>IIBBBBB', body)
        elif kind == b'IDAT':
            idat.append(body)
        at += 12 + length
    width, height, depth, colour, _, _, interlace = header
    channels = {0: 1, 2: 3, 4: 2, 6: 4}[colour]
    if interlace or depth not in (8, 16):
        raise ValueError(f'{path}: {depth}-bit, interlace {interlace} is not what the game writes')
    return width, height, channels * depth // 8, zlib.decompress(b''.join(idat)), (depth, colour)


def unfilter(width, height, bpp, raw):
    """The pixel rows of a decompressed PNG stream."""
    stride = width * bpp
    rows, previous = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        row = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        if kind == 1:
            for i in range(bpp, stride):
                row[i] = (row[i] + row[i - bpp]) & 255
        elif kind == 2:
            for i in range(stride):
                row[i] = (row[i] + previous[i]) & 255
        elif kind == 3:
            for i in range(stride):
                left = row[i - bpp] if i >= bpp else 0
                row[i] = (row[i] + ((left + previous[i]) >> 1)) & 255
        elif kind == 4:
            for i in range(stride):
                a = row[i - bpp] if i >= bpp else 0
                b = previous[i]
                c = previous[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(row)
        previous = row
    return rows


def write_png(path, width, height, rows):
    """An 8-bit RGB PNG of `rows` (bytes, width * 3 each)."""
    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    stream = b''.join(b'\0' + bytes(r) for r in rows)
    Path(path).write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                           + chunk(b'IDAT', zlib.compress(stream, 6)) + chunk(b'IEND', b''))


def compare(base_path, head_path, diff_path):
    """None when the two PNGs hold the same pixels, else what differs (writing the diff image, unless None)."""
    a, b = read_png(base_path), read_png(head_path)
    if a[:3] != b[:3] or a[4] != b[4]:
        return f'size or format differs: {a[:3] + (a[4],)} against {b[:3] + (b[4],)}'
    if a[3] == b[3]:
        return None
    width, height, bpp, _, (depth, colour) = a
    rows_a, rows_b = unfilter(*a[:4]), unfilter(*b[:4])
    if rows_a == rows_b:
        return None
    if diff_path is None:
        return 'the pixels differ'
    channel = depth // 8
    differing, largest, picture = 0, 0, []
    for ra, rb in zip(rows_a, rows_b):
        out = bytearray()
        for x in range(width):
            pa, pb = ra[x * bpp:(x + 1) * bpp], rb[x * bpp:(x + 1) * bpp]
            grey = pa[0] // 3 if colour in (0, 4) else (pa[0] + pa[channel] + pa[2 * channel]) // 9
            if pa != pb:
                differing += 1
                step = max(abs(int.from_bytes(pa[i:i + channel], 'big') - int.from_bytes(pb[i:i + channel], 'big'))
                           for i in range(0, bpp, channel))
                largest = max(largest, step)
                out += bytes((255, grey, grey))
            else:
                out += bytes((grey, grey, grey))
        picture.append(out)
    write_png(diff_path, width, height, picture)
    return f'{differing} of {width * height} pixels ({100 * differing / (width * height):.3f}%), largest step {largest}'


# 🤖 Agent: the game.

class Game:
    """One build's game, started offscreen in its own game folder and driven through horizon-ctl."""

    def __init__(self, bin_dir, folder, env, args):
        self.ctl_path = bin_dir / f'horizon-ctl{EXE}'
        self.client = bin_dir / f'horizon-client{EXE}'
        self.folder = folder
        self.env = env
        # 🤖 Agent: Sockets need short paths.
        self.socket = Path(tempfile.mkdtemp(prefix='hzfi')) / 'control.sock'
        self.args = args

    def ctl(self, *words, timeout=600, check=True):
        result = subprocess.run([str(self.ctl_path), '--socket', str(self.socket), '--timeout', str(timeout),
                                 *map(str, words)], cwd=self.folder, env=self.env, capture_output=True, text=True,
                                encoding='utf-8', errors='replace')
        if check and result.returncode:
            raise RuntimeError(f'horizon-ctl {" ".join(map(str, words))}: {result.stdout}{result.stderr}'[:2000])
        text = result.stdout.strip()
        return json.loads(text) if text.startswith('{') else text

    def command(self, text):
        """Runs `text`; on chunks not loaded yet (forceloaded ones still generating) waits and tries again for a
        minute. 🤖 Agent: a command on chunks not loaded changes less of the scene, differently each run."""
        for _ in range(60):
            reply = self.ctl('command', text)
            if 'not loaded' not in json.dumps(reply):
                return reply
            self.ctl('wait', 20)
        raise RuntimeError(f'/{text}: {reply}')

    def launch(self):
        # 🤖 Agent: A fixed name: the player's offline UUID, and so the hand's skin, follows it, and a random one is printed.
        self.ctl('launch', '--game', self.client, '--resources', self.args.resources, '--world', 'scenes',
                 '--seed', SEED, '--creative', '--offscreen', '--wait', '--game-dir', self.folder, '--name', 'Frames',
                 '--width', SIZE[0], '--height', SIZE[1], '--render-distance', '6', timeout=900)

    def settle(self, ticks=40):
        """Until no chunk is pending, then `ticks` more for the frame to catch up (chunks fade in on the clock)."""
        self.ctl('wait-ready', 300, timeout=330)
        self.ctl('wait', ticks)

    def quit(self):
        try:
            self.ctl('quit', timeout=120, check=False)
        except (OSError, subprocess.SubprocessError):
            pass


def prepare(game):
    """The same quiet world for every session: nothing that moves, nothing printed over the picture. Each part is
    built once its chunks are loaded around the player."""
    game.ctl('close', check=False)
    for c in ('gamerule send_command_feedback false', 'gamerule advance_time false', 'gamerule advance_weather false',
              'gamerule spawn_mobs false', 'weather clear 1000000', 'effect clear @s',
              f'forceload add {X - 40} {Z - 40} {X + 40} {Z + 40}'):
        game.command(c)
    # 🤖 Agent: The player waits on the first viewpoint's block: left in the air, it falls into the sea below while the
    # scenes are built, which unlocks the boat recipes, and their toast shows over the first shots at a different
    # moment each run.
    where, _, (x, y, z), _, _ = SCENES['day_water']
    game.command(f'execute in minecraft:{where} run setblock {int(x // 1)} {y - 1} {int(z // 1)} minecraft:stone')
    game.command(f'execute in minecraft:{where} run tp @s {x} {y} {z}')
    game.settle()
    # 🤖 Agent: Day over water: a stone platform with a pool, sand around it, the sky and clouds behind.
    game.command(f'fill {X - 8} {Y - 1} {Z - 8} {X + 8} {Y - 1} {Z + 8} minecraft:stone')
    game.command(f'fill {X - 4} {Y - 1} {Z - 4} {X + 4} {Y - 1} {Z + 4} minecraft:sand')
    game.command(f'fill {X - 3} {Y - 1} {Z - 3} {X + 3} {Y - 1} {Z + 3} minecraft:water')
    game.command(f'setblock {X + 6} {Y} {Z - 6} minecraft:oak_log')
    game.command(f'setblock {X + 6} {Y + 1} {Z - 6} minecraft:oak_leaves[persistent=true]')
    # 🤖 Agent: Night with light sources: a closed room east of it, lit by torches, a lantern, glowstone and lava.
    rx = X + 24
    game.command(f'fill {rx - 6} {Y - 1} {Z - 6} {rx + 6} {Y + 6} {Z + 6} minecraft:smooth_stone hollow')
    for x, z, block in ((-4, 4, 'torch'), (4, 4, 'lantern'), (0, 5, 'glowstone'), (-2, -4, 'soul_torch')):
        game.command(f'setblock {rx + x} {Y} {Z + z} minecraft:{block}')
    game.command(f'setblock {rx + 3} {Y - 1} {Z - 3} minecraft:lava')
    # 🤖 Agent: Stained glass: panes and blocks of several colours before a white wall, with ice and water behind them.
    gx = X - 24
    game.command(f'fill {gx - 5} {Y - 1} {Z + 6} {gx + 5} {Y + 5} {Z + 6} minecraft:white_concrete')
    game.command(f'fill {gx - 5} {Y - 1} {Z - 6} {gx + 5} {Y - 1} {Z + 5} minecraft:stone')
    for i, colour in enumerate(('red', 'lime', 'blue', 'yellow', 'magenta')):
        game.command(f'fill {gx - 5 + 2 * i} {Y} {Z + 2} {gx - 4 + 2 * i} {Y + 2} {Z + 2} minecraft:{colour}_stained_glass')
        game.command(f'setblock {gx - 5 + 2 * i} {Y + 3} {Z} minecraft:{colour}_stained_glass_pane')
    game.command(f'fill {gx - 1} {Y} {Z + 4} {gx + 1} {Y} {Z + 4} minecraft:ice')
    game.command(f'fill {gx - 3} {Y} {Z + 5} {gx + 3} {Y} {Z + 5} minecraft:water')
    # 🤖 Agent: A lava pool for the HUD over light, north of the platform, behind every other viewpoint and too far
    # for its light to reach them.
    game.command(f'fill {X - 7} {Y - 1} {Z - 35} {X + 7} {Y - 1} {Z - 29} minecraft:lava')
    # 🤖 Agent: The Nether: a pocket carved out of its rock, lit by glowstone, lava and soul fire.
    # 🤖 Agent: Carved before the player goes there: the Nether's rock and lava would hold it, and set it on fire.
    game.command('execute in minecraft:the_nether run forceload add -16 -16 16 16')
    game.command('execute in minecraft:the_nether run fill -6 60 -6 6 68 6 minecraft:netherrack hollow')
    game.command('execute in minecraft:the_nether run fill -5 61 -5 5 67 5 minecraft:air')
    for c in ('setblock 0 67 4 minecraft:glowstone', 'setblock -3 60 3 minecraft:lava',
              'setblock 3 60 3 minecraft:soul_soil', 'setblock 3 61 3 minecraft:soul_fire',
              'setblock -4 61 -1 minecraft:crimson_fungus', 'setblock 4 60 -2 minecraft:nether_quartz_ore'):
        game.command(f'execute in minecraft:the_nether run {c}')
    # 🤖 Agent: the player still ticks in a frozen world and falls: each viewpoint stands on a block.
    for where, _, (x, y, z), _, _ in SCENES.values():
        game.command(f'execute in minecraft:{where} run setblock {int(x // 1)} {y - 1} {int(z // 1)} minecraft:stone')
    game.command('kill @e[type=!player]')
    # 🤖 Agent: Entities, after the sweep: the world is frozen (HORIZON_STILL), so they stay as summoned. A cow
    # (shoot hurts it), a cutout texture with full-bright eyes (the spider), enchanted armour and a held enchanted
    # item, an arrow and an End crystal (drawn apart from the mobs), block entities (a chest, a bell, a banner) and a
    # framed item. No dropped item: it spins from an angle that differs from run to run.
    game.command(f'fill {EX - 5} {Y - 1} {EZ - 5} {EX + 5} {Y - 1} {EZ + 4} minecraft:stone')
    game.command(f'fill {EX - 5} {Y} {EZ + 4} {EX + 5} {Y + 3} {EZ + 4} minecraft:white_concrete')
    still = 'NoAI:1b,Silent:1b,NoGravity:1b,Rotation:[180f,0f]'
    game.command(f'summon minecraft:cow {EX - 3.5} {Y} {EZ + 0.5} {{{still}}}')
    game.command(f'summon minecraft:spider {EX + 3.5} {Y} {EZ + 0.5} {{{still}}}')
    game.command(f'summon minecraft:armor_stand {EX + 0.5} {Y} {EZ + 0.5} {{ShowArms:1b,NoGravity:1b,Rotation:[180f,0f]}}')
    stand = '@e[type=minecraft:armor_stand,limit=1]'
    game.command(f'item replace entity {stand} armor.chest with '
                 'minecraft:diamond_chestplate[minecraft:enchantments={"minecraft:protection":1}]')
    game.command(f'item replace entity {stand} weapon.mainhand with '
                 'minecraft:diamond_sword[minecraft:enchantments={"minecraft:sharpness":1}]')
    game.command(f'setblock {EX - 2} {Y} {EZ + 3} minecraft:chest[facing=north]')
    game.command(f'setblock {EX + 2} {Y} {EZ + 3} minecraft:bell[attachment=floor]')
    game.command(f'setblock {EX + 5} {Y} {EZ + 3} minecraft:red_banner[rotation=8]')
    game.command(f'summon minecraft:item_frame {EX} {Y + 2} {EZ + 3} '
                 '{Facing:2b,Item:{id:"minecraft:diamond",count:1}}')
    game.command(f'summon minecraft:arrow {EX + 2.0} {Y + 1.5} {EZ - 1.0} {{NoGravity:1b}}')
    game.command(f'summon minecraft:end_crystal {EX - 4.5} {Y} {EZ + 2.5} {{ShowBottom:0b}}')
    # 🤖 Agent: Items, which Horizon lights itself under any pack: two shown three blocks large by displays and one in
    # an unseen frame, at night over a dynamic light, so the light changes across each of their faces. The light is
    # a torch a husk holds in a closed cell under the floor (dynamic lights shine through blocks).
    game.command(f'fill {IX - 5} {Y - 1} {IZ - 5} {IX + 5} {Y - 1} {IZ + 4} minecraft:stone')
    game.command(f'fill {IX - 5} {Y} {IZ + 4} {IX + 5} {Y + 4} {IZ + 4} minecraft:white_concrete')
    game.command(f'fill {IX - 1} {Y - 4} {IZ} {IX + 1} {Y - 2} {IZ + 2} minecraft:stone')
    game.command(f'fill {IX} {Y - 3} {IZ + 1} {IX} {Y - 2} {IZ + 1} minecraft:air')
    game.command(f'summon minecraft:husk {IX + 0.5} {Y - 3} {IZ + 1.5} {{{still},PersistenceRequired:1b,'
                 'equipment:{mainhand:{id:"minecraft:torch",count:1}}}')
    large = ('transformation:{left_rotation:[0f,0f,0f,1f],right_rotation:[0f,0f,0f,1f],translation:[0f,0f,0f],'
             'scale:[3f,3f,3f]}')
    for x, item in ((-2.5, 'white_concrete'), (2.5, 'paper')):
        game.command(f'summon minecraft:item_display {IX + 0.5 + x} {Y + 2} {IZ + 1.5} '
                     f'{{item:{{id:"minecraft:{item}",count:1}},{large}}}')
    game.command(f'summon minecraft:item_frame {IX} {Y + 3} {IZ + 3} '
                 '{Facing:2b,Invisible:1b,Item:{id:"minecraft:diamond",count:1}}')


SCENES = {
    # 🤖 Agent: name: (where, time of day, the player's feet, yaw, pitch)
    'day_water': ('overworld', 6000, (X + 0.5, Y + 3, Z - 9.5), 0, 25),
    'night_lights': ('overworld', 18000, (X + 24.5, Y, Z - 4.5), 0, 20),
    'glass': ('overworld', 6000, (X - 23.5, Y, Z - 3.5), 0, 5),
    'nether': ('the_nether', 18000, (0.5, 61, -4.5), 0, 15),
    'entities': ('overworld', 6000, (EX + 0.5, Y, EZ - 4.5), 0, 5),
    'items': ('overworld', 18000, (IX + 0.5, Y, IZ - 4.5), 0, 5),
    # 🤖 Agent: The HUD over light (HUD below): chat over the sun high in the sky, the hotbar over the sky; and the same
    # view without chat, which shows where the sun's light passes white (hdr_gates).
    'hud_sun': ('overworld', 4000, (X + 0.5, Y + 3, Z - 9.5), -70, -85),
    'sun': ('overworld', 4000, (X + 0.5, Y + 3, Z - 9.5), -70, -85),
    # 🤖 Agent: Chat and the crosshair over the lava pool at night, and the pause menu over both.
    'hud_lava': ('overworld', 18000, (X + 0.5, Y + 3, Z - 27.5), 180, 45),
    'menu': ('overworld', 18000, (X + 0.5, Y + 3, Z - 27.5), 180, 45),
}
WORLD = ['day_water', 'night_lights', 'glass', 'nether', 'entities']
# 🤖 Agent: What each HUD scene shows over the world: chat, and the pause menu (its blur and darkening) over it.
HUD = {'hud_sun': ('chat',), 'hud_lava': ('chat',), 'menu': ('chat', 'pause')}
# 🤖 Agent: The options a scene turns off while it is shot. Over the sun, clouds and the vignette: either, drawn over
# light above white, comes out brighter than over vanilla's clipped sky (Why gamma storage), well beyond where the
# light passes white, so all that covers the sun's light there is the HUD.
OFF = {'hud_sun': ('renderClouds', 'vignette'), 'sun': ('renderClouds', 'vignette')}


def shoot(game, out, scene):
    where, day_time, (x, y, z), yaw, pitch = SCENES[scene]
    shows, off = HUD.get(scene, ()), OFF.get(scene, ())
    game.command(f'execute in minecraft:{where} run time set {day_time}')
    game.command(f'execute in minecraft:{where} run tp @s {x} {y} {z} {yaw} {pitch}')
    game.ctl('look', yaw, pitch)
    if scene == 'entities':
        # 🤖 Agent: The hurt flash is the client's, and gone once the cow is sent again (after the Nether): hurt
        # with the player there, it stays in the frozen world.
        game.command('damage @e[type=minecraft:cow,limit=1] 1')
    for option in off:
        game.ctl('options-set', option, 'false')
    if 'chat' in shows:
        # 🤖 Agent: Chat lines over a see-through background, sent now: they fade on the client's ticks.
        game.ctl('options-set', 'chatVisibility', '0')
        for colour in ('white', 'yellow', 'red', 'aqua', 'white'):
            game.command(f'tellraw @s {{"text":"Chat over the light: {colour} text on its background","color":"{colour}"}}')
    game.settle()
    if 'pause' in shows:
        # 🤖 Agent: The pause menu stops the ticks settle waits on; the screenshot is of a frame drawn after it.
        game.ctl('pause')
        game.ctl('wait-for', 'screen', 'paused', '--timeout', 30)
    game.ctl('screenshot', out / f'{scene}.png')
    if 'pause' in shows:
        game.ctl('close')
    for option in off:
        game.ctl('options-set', option, 'true')
    if 'chat' in shows:
        game.ctl('options-set', 'chatVisibility', '2')


def set_hdr(game, on):
    """Turn HDR output on or off from its settings page, as a player does."""
    game.ctl('options', 'graphics-hdr')
    # 🤖 Agent: Both the base's switch and the three-state choice reach an explicit On or Off.
    for _ in range(3):
        widgets = game.ctl('ui')
        widgets = widgets.get('widgets', widgets) if isinstance(widgets, dict) else widgets
        choice = next((w for w in widgets if w.get('kind') == 'button'
                       and str(w.get('label', '')).startswith('HDR: ')), None)
        if choice is None:
            raise RuntimeError(f'no HDR control on the HDR page: {json.dumps(widgets)[:600]}')
        if choice['label'].endswith(': On' if on else ': Off'):
            break
        game.ctl('click', choice['id'])
    else:
        raise RuntimeError('HDR control did not reach the requested setting')
    game.ctl('close')
    game.ctl('close', check=False)
    game.settle(60)


SESSIONS = {
    # 🤖 Agent: name: (horizon-graphics.txt, extra environment, scenes, HDR turned off after the first shots)
    'vanilla': ('', {}, WORLD + list(HUD) + ['sun'], False),
    # 🤖 Agent: An offered HDR output without a system report must leave Auto in SDR.
    'auto_unknown': ('hdr:auto\n', {'HORIZON_HDR_OUTPUT': 'pq'}, ['day_water', 'night_lights'], False),
    'golden_hour': ('', {'HORIZON_SHADER_PACK': 'golden_hour'}, WORLD, False),
    'golden_hour_hdr': ('hdr:true\n', {'HORIZON_SHADER_PACK': 'golden_hour', 'HORIZON_HDR_OUTPUT': 'pq'},
                        ['day_water', 'night_lights', 'entities'], False),
    'hdr_toggle': ('hdr:true\n', {'HORIZON_HDR_OUTPUT': 'pq'}, ['day_water', 'night_lights', 'entities'], True),
    'oit': ('', {}, ['day_water', 'night_lights'], False),
    # 🤖 Agent: The visibility buffer draws only frames drawn flat, which frames without a pack are by default only on
    # Metal and some Intel drivers.
    'visbuffer': ('', {'HORIZON_VISBUFFER': '1', 'HORIZON_FLAT_DRAWS': '1'}, ['day_water', 'night_lights'], False),
    'float_hdr': ('hdr:true\n', {'HORIZON_HDR_OUTPUT': 'pq'}, WORLD + list(HUD), False),
    'float_oit': ('hdr:true\n', {'HORIZON_HDR_OUTPUT': 'pq'}, WORLD, False),
    # 🤖 Agent: On a display with little headroom (300 nits), the HUD at half brightness: the tone curve and the HUD's
    # composite have the least room there (hdr_gates). An older file's 50% of a 200-nit paper white: it reads as 100
    # nits, so builds from before UI Brightness was in nits draw the same.
    'float_visbuffer': ('hdr:true\nhdrPeak:300\nhdrPaperWhite:200\nhdrUiBrightness:50\n',
                        {'HORIZON_HDR_OUTPUT': 'pq', 'HORIZON_VISBUFFER': '1', 'HORIZON_FLAT_DRAWS': '1'},
                        WORLD + list(HUD), False),
    # 🤖 Agent: The Vanilla look, at that headroom too, where the tone curve's knee is at white: the picture is SDR's
    # (hdr_gates). hdrLighting:false is the same look to a base from before hdrLook, which then draws the same.
    'float_unlit': ('hdr:true\nhdrLighting:false\nhdrLook:vanilla\nhdrPeak:300\n', {'HORIZON_HDR_OUTPUT': 'pq'},
                    WORLD + list(HUD) + ['sun'], False),
    # 🤖 Agent: The entity stage's fixture without its stage and with it (entity_gates): in SDR with the block
    # entities drawn each frame, as by default, and with HDR on with them kept on the GPU (blockEntityCache), which
    # draws them with a pipeline of their own.
    'entity_off': ('', {'HORIZON_SHADER_PACK': 'entity_off'}, ['entities', 'items'], False),
    'entity_stage': ('', {'HORIZON_SHADER_PACK': 'entity_stage'}, ['entities', 'items'], False),
    'entity_off_hdr': ('hdr:true\nblockEntityCache:true\n',
                       {'HORIZON_SHADER_PACK': 'entity_off', 'HORIZON_HDR_OUTPUT': 'pq'}, ['entities', 'items'], False),
    'entity_stage_hdr': ('hdr:true\nblockEntityCache:true\n', {'HORIZON_SHADER_PACK': 'entity_stage',
                                                               'HORIZON_HDR_OUTPUT': 'pq'}, ['entities', 'items'], False),
    # 🤖 Agent: Horizon's HDR look drawn by its pack, at float_hdr's settings (look_gates); and the look and the pack
    # on a display below the scene's peak, where the tone curve, the pack's [tonemap], bends what shines.
    'horizon_hdr': ('hdr:true\n', {'HORIZON_SHADER_PACK': 'horizon_hdr', 'HORIZON_HDR_OUTPUT': 'pq'}, WORLD, False),
    'float_dim': ('hdr:true\nhdrPeak:300\n', {'HORIZON_HDR_OUTPUT': 'pq'}, ['night_lights', 'nether'], False),
    'horizon_hdr_dim': ('hdr:true\nhdrPeak:300\n', {'HORIZON_SHADER_PACK': 'horizon_hdr', 'HORIZON_HDR_OUTPUT': 'pq'},
                        ['night_lights', 'nether'], False),
}
# 🤖 Agent: Auto paper white and peak are 203 and 1000 offscreen; written out so a change of Auto doesn't move the
# pictures. TAA and frame generation build on earlier frames, which HORIZON_STILL doesn't cover: off, as by default.
GRAPHICS = 'hdrPaperWhite:203\nhdrPeak:1000\ntaa:false\nfxaa:false\nframeGeneration:off\n'


def run_session(name, label, bin_dir, out, env_base, args):
    graphics, extra, scenes, toggle = SESSIONS[name]
    folder = RUN / 'games' / f'{name}-{label}'
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir(parents=True)
    (folder / 'horizon-graphics.txt').write_text(GRAPHICS + graphics)
    # 🤖 Agent: No chat, no tutorial toasts, no pause when the offscreen window has no focus.
    options = 'chatVisibility:2\ntutorialStep:none\npauseOnLostFocus:false\n'
    (folder / 'options.txt').write_text(options)
    if name.startswith('entity_'):
        # 🤖 Agent: The items scene's light.
        (folder / 'horizon-extras.txt').write_text('dynamicLights:true\n')
    if 'HORIZON_SHADER_PACK' in extra:
        pack = extra['HORIZON_SHADER_PACK']
        source, cut, unused = PACKS[pack]
        shutil.copytree(source, folder / 'shaderpacks' / pack)
        if cut:
            manifest = folder / 'shaderpacks' / pack / 'horizon-shaders.toml'
            manifest.write_text(manifest.read_text(encoding='utf-8').split(cut)[0], encoding='utf-8')
            (folder / 'shaderpacks' / pack / unused).unlink()
    env = lab.game_env(env_base, HORIZON_STILL='1', **extra)
    out.mkdir(parents=True, exist_ok=True)
    game = Game(bin_dir, folder, env, args)
    started = time.monotonic()
    try:
        game.launch()
        if name in ('oit', 'float_oit'):
            # 🤖 Agent: Old unversioned options run through the graphics-mode migration, which resets this switch.
            game.ctl('options-set', 'improvedTransparency', 'true')
            option = game.ctl('options-get', 'improvedTransparency')
            if option.get('options', {}).get('improvedTransparency', {}).get('value') is not True:
                raise RuntimeError(f'Improved Transparency was not requested: {option}')
            lab.write_json(out / f'{name}.option.json', option)
        prepare(game)
        for scene in scenes:
            shoot_named(game, out, name, scene)
        if toggle:
            set_hdr(game, False)
            for scene in scenes:
                shoot_named(game, out, f'{name}.off', scene)
        (out / f'{name}.log').write_text(str(game.ctl('logs', '--lines', '400', check=False)), encoding='utf-8')
    finally:
        game.quit()
    print(f'{label} {name}: {time.monotonic() - started:.0f} s', flush=True)


def shoot_named(game, out, prefix, scene):
    shoot(game, out, scene)
    for suffix in ('.png', '-hdr.png'):
        shot = out / f'{scene}{suffix}'
        if shot.exists():
            shot.replace(out / f'{prefix}.{scene}{suffix}')


def session_checks(shots, sessions):
    """That each session drew what it is for, so a pack that fails to load, or HDR that doesn't turn on, in both
    builds can't pass as identical: HDR shots have their -hdr.png and only they do, and the pack changes the picture."""
    problems = []
    for name in sessions:
        graphics, extra, scenes, toggle = SESSIONS[name]
        hdr = 'hdr:true' in graphics
        for scene in scenes:
            for prefix, on in ((name, hdr), *(((f'{name}.off', False),) if toggle else ())):
                if not (shots / f'{prefix}.{scene}.png').is_file():
                    problems.append(f'{prefix}.{scene}: screenshot missing')
                if (shots / f'{prefix}.{scene}-hdr.png').exists() != on:
                    problems.append(f'{prefix}.{scene}: HDR output was {"off" if on else "on"}')
        # 🤖 Agent: the pack's pictures against the same scenes without it: vanilla's, or with HDR on hdr_toggle's
        # first shots (the same settings), sidecars included.
        without = 'float_hdr' if name == 'float_oit' else ('hdr_toggle' if hdr else 'vanilla')
        # 🤖 Agent: Not the look's pack, whose pictures are meant to be the ones without it: look_gates reads its log.
        if 'HORIZON_SHADER_PACK' in extra and without in sessions and name not in LOOK_PAIRS:
            same = [s for s in scenes if any(
                compare(shots / f'{without}.{s}{x}', shots / f'{name}.{s}{x}', None) is None
                for x in ('.png', '-hdr.png')
                if (shots / f'{without}.{s}{x}').exists() and (shots / f'{name}.{s}{x}').exists())]
            problems += [f'{name}.{s}: the same as without the pack: it didn\'t run' for s in same]
        # 🤖 Agent: the log says whether the visibility buffer was built; with HORIZON_FLAT_DRAWS=1 it draws every frame.
        log = shots / f'{name}.log'
        built = log.exists() and 'through a visibility buffer' in log.read_text(encoding='utf-8')
        if 'HORIZON_VISBUFFER' in extra and not built:
            problems.append(f'{name}: the visibility buffer didn\'t run')
        # 🤖 Agent: Improved Transparency needs blendable float targets; without them the option changes nothing.
        if name in ('oit', 'float_oit') and without in sessions and all(
                compare(shots / f'{without}.{s}{x}', shots / f'{name}.{s}{x}', None) is None
                for s in scenes for x in ('.png', '-hdr.png') if (shots / f'{name}.{s}{x}').exists()):
            problems.append(f'{name}: the same as without Improved Transparency: it didn\'t run')
    return problems


# 🤖 Agent: The most a channel may differ between the entity stage's fixture and the same pack without the stage.
# Entities: the stage's colours are decoded going in and its result encoded again, which rounds differently in
# places. Items: the stage doesn't draw them, so nothing.
ENTITY_STEPS = {'entities': 1, 'items': 0}


def entity_gates(shots, sessions):
    """The fixture's entity stage is vanilla's own, so its pictures are the pack's without the stage, within the
    scene's ENTITY_STEPS on any channel: the 8-bit screenshots and, with HDR on, the HDR shots' 10-bit codes."""
    problems = []
    for suffix in ('', '_hdr'):
        off, on = f'entity_off{suffix}', f'entity_stage{suffix}'
        if off not in sessions or on not in sessions:
            continue
        for scene in SESSIONS[on][2]:
            for kind in ('.png', '-hdr.png') if suffix else ('.png',):
                _, a = pixels(shots / f'{off}.{scene}{kind}')
                _, b = pixels(shots / f'{on}.{scene}{kind}')
                steps = [max(abs(x - y) for x, y in zip(p, q)) for p, q in zip(a, b)]
                differing, largest = sum(1 for s in steps if s), max(steps)
                print(f'  {on}.{scene}{kind} against {off}: {differing} of {len(steps)} pixels differ, '
                      f'largest step {largest}', flush=True)
                step = ENTITY_STEPS[scene]
                if largest > step:
                    problems.append(f'{on}.{scene}{kind}: {sum(1 for s in steps if s > step)} pixels differ '
                                    f'from {off} by more than {step}, largest step {largest}')
    return problems


# 🤖 Agent: Horizon's HDR look drawn by its pack: each of the pack's sessions, with the session that draws the look
# itself at the same settings.
LOOK_PACK = 'horizon_hdr'
LOOK_PAIRS = {'horizon_hdr': 'float_hdr', 'horizon_hdr_dim': 'float_dim'}
# 🤖 Agent: The most a channel may differ between the two, of the 8-bit screenshot's 255 and of the HDR shot's 1023.
# The pack's route rounds apart from the look's own: its stages take linear light and return it, so each pixel goes
# through more 2.2 powers, the whole picture is decoded into a half-float target for [tonemap] and encoded again,
# and the lift's mark, which has 8 bits, falls on the other side of a step for a few texels.
LOOK_STEPS = {'.png': 1, '-hdr.png': 2}
# 🤖 Agent: What the pack doesn't reach in a scene, as fractions of the picture (x0, y0, x1, y1): the End crystal
# beside the cow. Its glass is drawn from both sides, and Horizon lifts a back face by its front's brightness
# (entity.wgsl `hdr_entity_mark` reads the front's colour), where a stage is handed the shade of the face it draws.
LOOK_APART = {'entities': ((0.73, 0.42, 0.83, 0.63),)}


def look_gates(shots, sessions):
    """Horizon's HDR look drawn by its pack (the look file's functions, called from the shader API's material,
    entity and tonemap stages) is the look the game draws without a pack, within LOOK_STEPS on any channel of the
    screenshot and of the HDR shot, where the pack reaches: blocks, entities and block entities. The scenes hold
    nothing else that shines (no sun, moon or stars in view, no particles, no dropped items, no light source in
    fog), but for what LOOK_APART names. Not held: the pixels under the crosshair and the hotbar, where the HUD's
    8-bit target turns a rounding step of the world under it into one of its own; and in the screenshot the pixels
    either build shows with a channel at white, since a screenshot maps light above white for SDR by Horizon's own
    rule, by the brightest channel with the look and channel by channel under a pack (the HDR shot holds them).
    The pack's log line says it ran: a pack that failed to load would draw the look itself, and pass."""
    problems = []
    for packed, plain in LOOK_PAIRS.items():
        if packed not in sessions or plain not in sessions:
            continue
        log = shots / f'{packed}.log'
        text = log.read_text(encoding='utf-8') if log.exists() else ''
        # 🤖 Agent: "shader pack horizon_hdr 1.0.0: Blocks, Entities, Tone curve in 12 ms", not "... refused: ...".
        if not re.search(rf'shader pack {LOOK_PACK} \d\S*: ', text):
            problems.append(f'{packed}: the pack didn\'t run')
            continue
        for scene in SESSIONS[packed][2]:
            for kind, most in LOOK_STEPS.items():
                width, a = pixels(shots / f'{plain}.{scene}{kind}')
                _, b = pixels(shots / f'{packed}.{scene}{kind}')
                height = len(a) // width
                apart = hud_boxes(width, height) + tuple(
                    (x0 * width, y0 * height, x1 * width, y1 * height) for x0, y0, x1, y1 in LOOK_APART.get(scene, ()))
                steps = [max(abs(x - y) for x, y in zip(p, q)) for i, (p, q) in enumerate(zip(a, b))
                         if not (kind == '.png' and (255 in p or 255 in q))
                         and not any(x0 <= i % width < x1 and y0 <= i // width < y1 for x0, y0, x1, y1 in apart)]
                largest = max(steps, default=0)
                print(f'  {packed}.{scene}{kind} against {plain}: {sum(1 for s in steps if s)} of {len(steps)} '
                      f'pixels held differ, {sum(1 for s in steps if s > 1)} by more than a step, largest step '
                      f'{largest}', flush=True)
                if largest > most:
                    problems.append(f'{packed}.{scene}{kind}: {sum(1 for s in steps if s > most)} pixels differ '
                                    f'from {plain} by more than {most}, largest step {largest}')
    return problems


# 🤖 Agent: HDR's gates (docs/design/hdr-linear-light.md, Tests 3 and 4): what a build's HDR shots must be against
# its SDR ones, whatever the base drew. The HDR shot is HDR10: BT.2020 primaries, SMPTE ST 2084 (PQ), 10 bits a
# channel.

ST2084 = (2610 / 16384, 2523 / 32, 3424 / 4096, 2413 / 128, 2392 / 128)
# 🤖 Agent: Linear BT.709 to BT.2020, ITU-R BT.2087, by rows.
BT2020 = ((0.6274, 0.3293, 0.0433), (0.0691, 0.9195, 0.0114), (0.0164, 0.0880, 0.8956))
# 🤖 Agent: And back, ITU-R BT.2407.
BT709 = ((1.6605, -0.5876, -0.0728), (-0.1246, 1.1329, -0.0083), (-0.0182, -0.1006, 1.1187))


def pq(nits):
    m1, m2, c1, c2, c3 = ST2084
    y = (max(nits, 0.0) / 10000) ** m1
    return ((c1 + c2 * y) / (1 + c3 * y)) ** m2


def hdr10(light, paper, cap=float('inf')):
    """The HDR10 codes of linear BT.709 `light` in units of paper white, each channel at most `cap` of it."""
    light = [min(c, cap) for c in light]
    return [1023 * pq(paper * sum(m * c for m, c in zip(row, light))) for row in BT2020]


def pixels(path):
    """A PNG's width and its pixels as RGB tuples, row by row: 8-bit values, or 10-bit ones from the game's 16-bit
    HDR PNG."""
    width, height, bpp, raw, (depth, _) = read_png(path)
    out = []
    for row in unfilter(width, height, bpp, raw):
        values = [int.from_bytes(row[i:i + 2], 'big') >> 6 for i in range(0, len(row), 2)] if depth == 16 else row
        step = bpp // (depth // 8)
        out += [tuple(values[i:i + 3]) for i in range(0, len(values), step)]
    return width, out


def hud_boxes(width, height):
    """Where the crosshair and the hotbar are, x0, y0, x1, y1, at the GUI scale Auto picks for the shot's size (the
    window's, scaled by the desktop's)."""
    scale = max(1, min(width // 320, height // 240))
    x, y = width // 2, height // 2
    return ((x - 9 * scale, y - 9 * scale, x + 9 * scale, y + 9 * scale),
            (x - 92 * scale, height - 24 * scale, x + 92 * scale, height))


def unclipped(light):
    """The most light the SDR screenshot (hdr.wgsl screenshot_world) shows as `light`: itself below white, and
    anything from white up, which it scales down to white by its brightest channel."""
    return light if max(light) < 1 else [float('inf')] * 3


def shows(sdr, paper, cap=float('inf'), mapped=lambda light: light):
    """The HDR10 codes an 8-bit SDR pixel allows: half a step either way (its rounding), its light through `mapped`
    first, and a code either way (the HDR shot's own rounding)."""
    low, high = ([(max(v + step, 0) / 255) ** 2.2 for v in sdr] for step in (-0.5, 0.5))
    return [c - 1 for c in hdr10(mapped(low), paper, cap)], [c + 1 for c in hdr10(mapped(high), paper, cap)]


def unpq(code):
    """The nits of a 10-bit PQ code: ST 2084's EOTF."""
    m1, m2, c1, c2, c3 = ST2084
    e = (code / 1023) ** (1 / m2)
    return 10000 * (max(e - c1, 0) / (c2 - c3 * e)) ** (1 / m1)


def above_white(path, paper):
    """The pixels, (x, y), of an HDR shot whose light passes paper white on a BT.709 channel."""
    nits = [unpq(code) / paper for code in range(1024)]
    width, hdr = pixels(path)
    return {(i % width, i // width) for i, h in enumerate(hdr)
            if max(sum(m * nits[c] for m, c in zip(row, h)) for row in BT709) > 1}


def clipped(path):
    """The pixels, (x, y), of an 8-bit shot with a channel at 255."""
    width, sdr = pixels(path)
    return {(i % width, i // width) for i, p in enumerate(sdr) if 255 in p}


def setting(name, key, default=None):
    """A session's value of a horizon-graphics.txt key, after GRAPHICS's, else the game's default."""
    graphics = GRAPHICS + SESSIONS[name][0]
    return int(graphics.rsplit(f'{key}:', 1)[1].split()[0]) if f'{key}:' in graphics else default


# 🤖 Agent: The white pixels each HUD scene shows at the HUD's white at least: its text is 3776 or more of them at
# the shots' size.
HUD_WHITE = 200


def hdr_gates(shots, sessions):
    """With the Vanilla look (float_unlit), the picture is SDR's: its SDR screenshot is HDR off's within a step of
    rounding (two in at most 1 pixel in 10 000, where the float frame blends several times), and its HDR shot is
    that screenshot's picture. Where the sun's light passes white (above_white of the sun shot, in sun and hud_sun)
    and HDR off clips it too, the screenshot clips it and the HDR shot keeps it, so there the HDR shot is only never darker; and the later
    blends over that light come out brighter than vanilla's over its clipped sky (Why gamma storage), so there the
    screenshot isn't held to HDR off.
    With Horizon's look, the SDR screenshot is the HDR shot clipped at white (in the world scenes, outside the
    HUD), nothing passes the peak, and the night's lights pass paper white. White in the screenshot under the HUD is
    the HUD's white at UI Brightness times paper white, the world's light from paper white up, or one over the other,
    which lies between them; each HUD scene shows HUD_WHITE pixels of it at the HUD's white itself."""
    problems = []

    def each(name, scene, test, what):
        """`test(sdr, hdr, x, y, boxes)` on each pixel of a scene's two shots; their pixels."""
        width, sdr = pixels(shots / f'{name}.{scene}.png')
        _, hdr = pixels(shots / f'{name}.{scene}-hdr.png')
        boxes = hud_boxes(width, len(sdr) // width)
        bad = [i for i, (s, h) in enumerate(zip(sdr, hdr)) if not test(s, h, i % width, i // width, boxes)]
        if bad:
            i = bad[0]
            problems.append(f'{name}.{scene}: {what}: {len(bad)} pixels, first at {i % width},{i // width}: '
                            f'SDR {sdr[i]}, HDR10 {hdr[i]}')
        return sdr, hdr

    for name in sessions:
        graphics, extra = GRAPHICS + SESSIONS[name][0], SESSIONS[name][1]
        # 🤖 Agent: Without a pack: a pack's [tonemap] maps its own picture for each output.
        if 'HORIZON_SHADER_PACK' in extra or 'hdr:true' not in graphics:
            continue
        paper, peak, ui = setting(name, 'hdrPaperWhite'), setting(name, 'hdrPeak'), setting(name, 'hdrUiBrightness', 100)
        lighting = 'hdrLook:vanilla' not in graphics
        for scene in SESSIONS[name][2]:
            if not (shots / f'{name}.{scene}-hdr.png').exists():
                continue
            ranges = {}
            if not lighting:
                hot = set()
                if scene in OFF:
                    sun, plain_sun = shots / f'{name}.sun-hdr.png', shots / 'vanilla.sun.png'
                    if not sun.exists() or not plain_sun.exists():
                        problems.append(f'{name}.{scene}: no sun shots to find where light passes white')
                        continue
                    # 🤖 Agent: Only where HDR off clipped too: HDR off's shot is the base's, byte for byte, so a
                    # build that wrongly lifts light above white can't excuse it.
                    hot = above_white(sun, paper) & clipped(plain_sun)
                if 'vanilla' in sessions:
                    width, plain = pixels(shots / f'vanilla.{scene}.png')
                    _, unlit = pixels(shots / f'{name}.{scene}.png')
                    steps = [max(abs(a - b) for a, b in zip(p, q)) for i, (p, q) in enumerate(zip(plain, unlit))
                             if (i % width, i // width) not in hot]
                    if max(steps, default=0) > 2 or 10000 * steps.count(2) > len(steps):
                        problems.append(f'{name}.{scene}: the SDR screenshot isn\'t HDR off\'s within rounding: '
                                        f'{len(steps) - steps.count(0) - steps.count(1)} pixels more than a step '
                                        f'apart, by up to {max(steps)}')

                def test(s, h, x, y, _):
                    if s not in ranges:
                        ranges[s] = shows(s, paper)
                    low, high = ranges[s]
                    return all(lo <= c and ((x, y) in hot or c <= hi) for c, lo, hi in zip(h, low, high))
                each(name, scene, test, 'the HDR shot isn\'t its SDR screenshot\'s picture')
                continue
            top, white, world = 1023 * pq(peak) + 1, 1023 * pq(paper * ui / 100), 1023 * pq(paper)

            def test(s, h, x, y, boxes):
                if max(h) > top:
                    return False
                if scene in HUD or any(x0 <= x < x1 and y0 <= y < y1 for x0, y0, x1, y1 in boxes):
                    # 🤖 Agent: White is the HUD's white, the world's light from white up, or one over the other.
                    return s != (255, 255, 255) or min(h) >= min(white, world) - 1
                if s not in ranges:
                    ranges[s] = shows(s, paper, peak / paper, unclipped)
                return all(lo <= c <= hi for c, lo, hi in zip(h, *ranges[s]))
            sdr, hdr = each(name, scene, test, 'the SDR screenshot isn\'t the HDR shot clipped at white, white '
                            'under the HUD is dimmer than the HUD\'s and the world\'s, or the HDR shot passes the peak')
            if scene in HUD:
                # 🤖 Agent: The rule above can't tell the HUD's white from light behind it, so the HUD's opaque white
                # (its text) is counted where it must be.
                found = sum(s == (255, 255, 255) and all(abs(c - white) <= 1 for c in h) for s, h in zip(sdr, hdr))
                if found < HUD_WHITE:
                    problems.append(f'{name}.{scene}: white HUD isn\'t at UI Brightness: {found} white pixels '
                                    f'there, of at least {HUD_WHITE}')
            if scene == 'night_lights' and max(map(max, hdr)) <= 1023 * pq(paper) + 1:
                problems.append(f'{name}.{scene}: no light passes paper white in the HDR shot')
    return problems


# 🤖 Agent: The sessions a gate holds to another session's shots, each with that session: the look's pack to the look
# (look_gates), the Vanilla look to HDR off and its sun shot (hdr_gates). Without it look_gates skips the pair, the
# guard that the pack ran with it, and hdr_gates skips the screenshots and fails only once both builds have drawn.
HELD_TO = {**LOOK_PAIRS, 'float_unlit': 'vanilla'}


def alone(sessions):
    """The chosen sessions held to a session that isn't chosen, as "name needs other": drawn like that one would
    pass unheld, the look's pack even where the game refused it and drew the look itself."""
    return [f'{name} needs {HELD_TO[name]}' for name in sessions if name in HELD_TO and HELD_TO[name] not in sessions]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--base', help='the revision to compare with (default: BASE_SHA, else the merge-base with origin/main)')
    parser.add_argument('--build-base', action='store_true', help='build the base even where it could reuse the change\'s binaries')
    parser.add_argument('--bin-dir', help='the change\'s prebuilt horizon-client and horizon-ctl (default: build them)')
    parser.add_argument('--base-bin-dir', help='the base\'s prebuilt binaries (default: build them, or reuse the change\'s)')
    parser.add_argument('--sessions', default=','.join(SESSIONS), help=f'which of {", ".join(SESSIONS)}')
    parser.add_argument('--resources', default=str(lab.RESOURCES), help='the prepared resources both builds read')
    parser.add_argument('-j', '--jobs', type=int, help='cargo build jobs')
    args = parser.parse_args()
    sessions = [s for s in args.sessions.split(',') if s]
    unknown = [s for s in sessions if s not in SESSIONS]
    if unknown:
        sys.exit(f'unknown sessions: {unknown}')
    if alone(sessions):
        sys.exit(f'--sessions leaves out a session another is held to: {", ".join(alone(sessions))}')
    base = base_revision(args.base)
    head = git('rev-parse', 'HEAD')
    print(f'base {base[:10]}, change {head[:10]}', flush=True)

    timings = {}
    started = time.monotonic()
    head_bin = Path(args.bin_dir).resolve() if args.bin_dir else build(ROOT, lab.cargo_target(), args.jobs)
    timings['change_build_s'] = round(time.monotonic() - started)
    started = time.monotonic()
    own = needs_own_build(base)
    if args.base_bin_dir:
        base_bin = Path(args.base_bin_dir).resolve()
    elif own or args.build_base:
        print(f'the base builds on its own ({len(own)} changed files a binary may use)', flush=True)
        checkout_base(base)
        base_bin = build(BASE_SRC, BASE_TARGET, args.jobs)
    else:
        print('the change touches no file a game binary is built from: the base reuses its binaries', flush=True)
        base_bin = head_bin
    timings['base_build_s'] = round(time.monotonic() - started)

    env = dict(os.environ)
    icd = lavapipe()
    if icd:
        env.update(VK_DRIVER_FILES=str(icd), VK_ICD_FILENAMES=str(icd), WGPU_BACKEND='vulkan')
        print(f'drawing on lavapipe ({icd})', flush=True)
    else:
        print('no lavapipe here: both builds draw on this machine\'s default adapter', flush=True)

    shots = RUN / 'shots'
    shutil.rmtree(shots, ignore_errors=True)
    failures, compared, base_sessions = [], 0, list(sessions)
    for name in sessions:
        builds = [('base', base_bin), ('change', head_bin)]
        if not pack_at(base, name):
            # 🤖 Agent: The sessions' packs are the change's: a base from before one has no picture of its own with it.
            print(f'  {name}: the base has no such pack, so it is not compared', flush=True)
            base_sessions.remove(name)
            del builds[0]
        for label, bin_dir in builds:
            session_started = time.monotonic()
            run_session(name, label, bin_dir, shots / label, env, args)
            timings[f'{name}_{label}_s'] = round(time.monotonic() - session_started)
        if name not in base_sessions:
            continue
        if "[entity] stage isn't drawn yet" in (shots / 'base' / f'{name}.log').read_text(encoding='utf-8'):
            # 🤖 Agent: A base from before the game drew the entity stage refuses the fixture pack and draws without
            # it: nothing of this session's to compare the change with.
            print(f'  {name}: the base refuses the pack, so it is not compared', flush=True)
            base_sessions.remove(name)
            continue
        for path in sorted((shots / 'base').glob(f'{name}.*.png')):
            other = shots / 'change' / path.name
            compared += 1
            if not other.exists():
                failures.append(f'{path.name}: the change wrote no such screenshot')
                continue
            problem = compare(path, other, shots / 'change' / f'diff-{path.name}')
            print(f'  {path.name}: {"identical" if problem is None else problem}', flush=True)
            if problem:
                failures.append(f'{path.name}: {problem}')
        extra = sorted(p.name for p in (shots / 'change').glob(f'{name}.*.png')
                       if not (shots / 'base' / p.name).exists() and not p.name.startswith('diff-'))
        failures += [f'{n}: the base wrote no such screenshot' for n in extra]
    if compared == 0:
        failures.append('no screenshots were taken')
    failures += session_checks(shots / 'base', base_sessions)
    for label, drawn in (('base', base_sessions), ('change', sessions)):
        failures += [f'{label}: {p}' for p in hdr_gates(shots / label, drawn)]
        failures += [f'{label}: {p}' for p in entity_gates(shots / label, drawn)]
        failures += [f'{label}: {p}' for p in look_gates(shots / label, drawn)]
    lab.write_json(RUN / 'report.json', {'base': base, 'change': head, 'lavapipe': str(icd) if icd else None,
                                         'compared': compared, 'failures': failures, 'timings': timings})
    print(json.dumps(timings), flush=True)
    print('frame identity:', 'PASS' if not failures else f'{len(failures)} FAILED', f'({compared} pictures)')
    for f in failures:
        print('  ', f)
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
