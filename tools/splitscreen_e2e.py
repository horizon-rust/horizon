#!/usr/bin/env python3
"""Splitscreen end to end: real games, driven like a player would, checked from outside.

    python3 tools/splitscreen_e2e.py                       # every scenario (about 25 minutes)
    python3 tools/splitscreen_e2e.py --list                # the scenarios, numbered, and groups
    python3 tools/splitscreen_e2e.py --only three eight    # some scenarios by name
    python3 tools/splitscreen_e2e.py --only 12 15- @server # by number, range or group
    python3 tools/splitscreen_e2e.py -v --only 9           # -v: every command, reply and screen change

Each scenario prints `[n/total] name` first: give that number to --only to run it again.

How it works
- `Game(players)` launches P1's game through horizon-ctl (`target/fast/horizon-ctl`, built with
  `cargo build --profile fast -p horizon-client`) with `HORIZON_SPLIT=players`: the game adds the
  others itself, as if their controllers had pressed a button. `pick=True` (`HORIZON_SPLIT_PICK`)
  stops them at the profile picker instead of naming them P2, P3...
- Every player has its own control socket (`c.sock`, `c.sock2`...). `g.ctl(n, words...)` sends one
  horizon-ctl command to player n (docs/horizon-ctl-commands.md): `ui` (screen and widgets), `state`,
  `click`/`click-button`, `key`, `type`, `pad` (a virtual controller), `options`, `quit`...
- Checks look from outside: each player's view size (`g.size(n)`, from `status`), its screen's
  buttons (`g.labels(n)`), its `state`. `check(name, got, want)` records a failure and goes on; a
  `RuntimeError` (a `wait` that timed out, a missing button) ends that scenario as failed.
- Helpers: `wait(done, what, seconds)` polls; `settle(players, want)` waits for sizes; `page(n)` /
  `setup(n)` open Horizon Settings > Splitscreen...; `press(n, label)` clicks a button by label
  prefix; `play_on(player, where)` moves a player on the Setup screen and applies; `close_page(n)`
  Escs back to playing; `quit()` closes everything (always in `finally`).
- `dedicated()` starts a Horizon dedicated server for the server scenarios.
- Each game gets fresh scratch folders (`lab.game_env`) and runs inside them: never your real game
  data. Needs a desktop session (real windows) and the vanilla data in .local/.

Adding a scenario: write `scenario_<name>()` that prints `"<name>: what it checks"` first (--list
shows it), uses `Game` in a try/finally with `g.quit()`, and checks with `check`; add it to
`SCENARIOS` (the order is the numbering) and to the `GROUPS` it belongs to.

Pitfalls
- Don't rebuild the game, stash or switch branches while it runs: the next launch takes the new
  binary or times out ("not ready after 300 seconds").
- The windows are real and can take keyboard focus: typing elsewhere can land in a test game.
- Moves through lists (the bumpers' places, the Setup screen's stops) count presses: change an order
  in the game and update those counts here.
- Labels change with the UI ("Splitscreen Settings", "Names: On"...): `press` matches a prefix.

Exit status 0 when every check passed.
"""

import argparse
import inspect
import re
import contextlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
CTL = lab.cargo_target() / 'fast/horizon-ctl'
RESOURCES = ROOT / '.local/horizon-client/26.3'
# Logical; the views are checked as fractions of what P1 gets alone, so any scale works.
WIDTH, HEIGHT = 1000, 600

failures = []


def check(name, got, want):
    ok = got == want
    print(f'  {"ok  " if ok else "FAIL"} {name}: {got}' + ('' if ok else f', expected {want}'))
    if not ok:
        failures.append(name)


class Game:
    """One splitscreen session: P1 launched with `players` (HORIZON_SPLIT), each on its socket."""

    def __init__(self, players, pick=False, seed=None, args=(), ready=True, home=None):
        """`ready=False`: return once P1's game answers, before it is in a world (it may be asked
        something first); the caller waits for the players."""
        self.home = Path(home) if home is not None else Path(tempfile.mkdtemp(prefix='horizon-split-e2e-'))
        self.home.mkdir(parents=True, exist_ok=True)
        if seed:
            seed(self.home)
        self.sockets = Path(tempfile.mkdtemp(prefix='hzse-'))  # short: Unix socket paths
        extra = {'HORIZON_SPLIT': str(players)}
        if pick:
            extra['HORIZON_SPLIT_PICK'] = '1'
        self.env = lab.game_env(HORIZON_HOME=self.home, **extra)
        self.players = players
        try:
            self.ctl(1, 'launch', *(['--wait'] if ready else []), '--world', 'Split', '--creative',
                     '--resources', str(RESOURCES), '--width', str(WIDTH), '--height', str(HEIGHT),
                     '--render-distance', '4', '--name', 'P1',
                     # Another scenario's windows closing take the focus: no pause on losing it.
                     '--no-pause', *args)
            if not ready:
                self.wait(lambda: self.alive(1), "P1's game")
                return
            self.wait(lambda: all(self.alive(n) for n in range(1, players + 1)), 'every player in')
            # The automatic GUI scale keeps menus about 400 x 240: room for every row of the page.
            self.ctl(1, 'options-set', 'guiScale', '2')
        except BaseException:
            # A launch that never got ready leaves no game behind.
            self.quit()
            raise

    screens = {}

    def ctl(self, n, *words):
        socket = self.sockets / ('c.sock' if n == 1 else f'c.sock{n}')
        out = subprocess.run([str(CTL), '--socket', str(socket), *words], env=self.env,
                             cwd=self.home, capture_output=True, text=True)
        reply = json.loads(out.stdout or '{}')
        if VERBOSE:
            # Every command and its reply, and each player's screen as it changes: enough to tell
            # afterwards what put a game where it is.
            at = f'{time.monotonic() - STARTED:8.2f}'
            print(f'{at} P{n} {" ".join(words)} -> {json.dumps(reply)[:400]}', file=sys.stderr, flush=True)
            screen = reply.get('screen')
            if screen is not None and self.screens.get(n) != screen:
                print(f'{at} P{n} screen: {self.screens.get(n)} -> {screen}', file=sys.stderr, flush=True)
                self.screens[n] = screen
        if 'error' in reply:
            raise RuntimeError(f'P{n} {" ".join(words)}: {reply["error"]}')
        return reply

    def alive(self, n):
        try:
            self.ctl(n, 'status')
            return True
        except (RuntimeError, json.JSONDecodeError):
            return False

    def size(self, n):
        return tuple(self.ctl(n, 'status')['renderer']['size'])

    def sizes(self, players):
        return [self.size(n) for n in players]

    @staticmethod
    def wait(done, what, seconds=30):
        end = time.time() + seconds
        while time.time() < end:
            if done():
                return
            time.sleep(0.3)
        raise RuntimeError(f'timed out waiting for {what}')

    def settle(self, players, want):
        """Wait until the players' views have the sizes `want` (a moment for windows to open)."""
        try:
            self.wait(lambda: self.sizes(players) == want, 'the layout', seconds=8)
        except RuntimeError:
            pass
        return self.sizes(players)

    def labels(self, n=1):
        return [w.get('label', '') for w in self.ctl(n, 'ui')['widgets']]

    def page(self, n=1):
        """Open player n's Splitscreen Setup screen (Horizon Settings > Splitscreen...), on or
        off; its widgets."""
        def shown():
            labels = self.labels(n)
            return 'Done' in labels and ('Identify' in labels or 'Splitscreen: Off' in labels)
        if shown():
            return self.ctl(n, 'ui')['widgets']  # still open (Esc taken during a rebuild)
        self.ctl(n, 'options', 'horizon')
        try:
            self.wait(lambda: 'Splitscreen...' in self.labels(n), 'Horizon Settings', seconds=15)
        except RuntimeError:
            ui = self.ctl(n, 'ui')
            raise RuntimeError(f'no Horizon Settings: screen {ui.get("screen")}, '
                               f'buttons {[w["label"] for w in ui["widgets"]]}') from None
        self.ctl(n, 'click-button', 'Splitscreen...')
        self.wait(shown, 'the Setup screen', seconds=5)
        return self.ctl(n, 'ui')['widgets']

    def setup(self, n=1):
        """Open player n's Splitscreen Setup screen, splitscreen on."""
        self.page(n)
        self.wait(lambda: 'Apply' in self.labels(n), 'the Setup screen', seconds=5)

    def press(self, n, label):
        """Click player n's button `label` (the first whose label starts with it)."""
        ids = [w['id'] for w in self.ctl(n, 'ui')['widgets'] if w.get('label', '').startswith(label)]
        if not ids:
            raise RuntimeError(f'no {label!r} among {self.labels(n)}')
        self.ctl(n, 'click', ids[0])
        time.sleep(0.4)

    def play_on(self, player, where, by=1):
        """Move player `player` to `where` ('Window 2', 'New window', 'Fill Monitor 2') on player
        `by`'s Setup screen and Apply; a window not open yet is a new one."""
        self.setup(by)
        self.press(by, f'P{player}')
        if where.startswith('Window '):
            # One window button cycles through the windows: on until it names this one (or a new
            # window when it never does: the window isn't open yet).
            for _ in range(16):
                windows = [l for l in self.labels(by) if l.startswith('Window ')]
                if not windows or where in windows:
                    break
                self.press(by, windows[0])
        self.press(by, where if where in self.labels(by) else 'New window')
        self.press(by, 'Apply')
        self.close_page(by)

    def close_page(self, n=1):
        """Back to playing: Esc through the options and the pause menu."""
        for _ in range(6):
            if self.ctl(n, 'ui').get('screen') not in ('options', 'paused'):
                return
            self.ctl(n, 'key', 'esc', 'tap')
            time.sleep(0.5)

    def quit(self):
        """Save and quit, and wait until the game is gone (the next scenario starts clean)."""
        try:
            self.ctl(1, 'quit')
            self.wait(lambda: not self.alive(1), 'the game to quit')
        except (RuntimeError, json.JSONDecodeError):
            # Not answering (still starting, or stuck): stop it by its socket (`--control-socket`).
            subprocess.run(['pkill', '-f', str(self.sockets)], capture_output=True)
        shutil.rmtree(self.sockets, ignore_errors=True)


def scenario_join():
    print('join: four players, then one leaves')
    g = Game(4)
    try:
        quarters = g.sizes([1, 2, 3, 4])
        check('four take equal quarters', len(set(quarters)), 1)
        w, h = quarters[0]
        g.ctl(4, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(4), "P4's pause menu", seconds=5)
        check("P4's pause menu says Leave", 'Leave' in g.labels(4), True)
        g.ctl(4, 'disconnect')
        three = g.settle([1, 2, 3], [(2 * w, h), (w, h), (w, h)])
        check('three: one wide on top, two below', three, [(2 * w, h), (w, h), (w, h)])
        # A player's quit leaves splitscreen; only P1's ends the game.
        g.ctl(3, 'quit')
        g.wait(lambda: not g.alive(3), 'P3 gone after its quit', seconds=15)
        check("P3's quit: P3 left, P1 plays on", g.ctl(1, 'state').get('player') is not None, True)
    finally:
        g.quit()


def scenario_windows():
    print('windows: New window, share it, leave it')
    g = Game(4)
    try:
        w, h = g.size(1)
        g.play_on(4, 'Window 2')  # New window: the first free number
        main = g.settle([1, 2, 3], [(2 * w, h), (w, h), (w, h)])
        check('Window 1: one wide on top, two below', main, [(2 * w, h), (w, h), (w, h)])
        alone = g.size(4)
        g.play_on(3, 'Window 2')  # P4's
        halves = g.settle([1, 2], [(w, 2 * h), (w, 2 * h)])
        check('Window 1: P1 and P2 side by side', halves, [(w, 2 * h), (w, 2 * h)])
        shared = g.sizes([3, 4])
        check('Window 2: P3 and P4 share it', shared[0] == shared[1] and shared[0][0] < alone[0], True)
        g.play_on(3, 'Window 1')
        back = g.settle([1, 2, 3], [(2 * w, h), (w, h), (w, h)])
        check('P3 back in Window 1', back, [(2 * w, h), (w, h), (w, h)])
        check('P4 alone in Window 2 again', g.size(4), alone)
    finally:
        g.quit()


def scenario_three():
    print('three: the three layouts')
    g = Game(3)
    try:
        w2, h = g.size(2)  # one wide on top by default: P2 has a quarter
        layouts = [
            ('one tall on the left', [(w2, 2 * h), (w2, h), (w2, h)]),
            ('side by side', None),
            ('one wide on top', [(2 * w2, h), (w2, h), (w2, h)]),
        ]
        for name, want in layouts:
            g.setup()
            g.press(1, '3 sharing:')
            g.press(1, 'Apply')
            g.close_page()
            got = g.settle([1, 2, 3], want) if want else g.settle([1, 2, 3], [g.size(1)] * 3)
            if want:
                check(name, got, want)
            else:
                widths = [x for x, _ in got]
                check(f'{name}: three columns, full height',
                      max(widths) - min(widths) <= 1 and all(y == 2 * h for _, y in got), True)
    finally:
        g.quit()


def scenario_live():
    print('live: splitscreen off and on in a world')
    g = Game(2)
    try:
        half = g.size(1)
        # With another player in, the switch is greyed: it turns off once P1 plays alone.
        enabled = [w.get('enabled') for w in g.page() if w.get('label') == 'Splitscreen: On']
        check('others in: the switch is greyed', enabled, [False])
        g.close_page()
        g.ctl(2, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(2), "P2's pause menu", seconds=5)
        g.press(2, 'Leave')
        g.wait(lambda: not g.alive(2), 'P2 gone', seconds=10)
        g.page()
        g.ctl(1, 'click-button', 'Splitscreen: On')
        g.close_page()
        full = g.settle([1], [(2 * half[0], half[1])])
        check('off: P1 has the whole window', full, [(2 * half[0], half[1])])
        g.page()
        g.ctl(1, 'click-button', 'Splitscreen: Off')
        g.close_page()
        time.sleep(1)
        status = g.ctl(1, 'status')
        check('on again: P1 still renders the whole window', tuple(status['renderer']['size']),
              (2 * half[0], half[1]))
        check('on again: frames keep coming', status['fps'] > 10, True)
    finally:
        g.quit()


def scenario_picker():
    print('picker: RB to a new window, then a new profile')
    g = Game(2, pick=True)
    try:
        g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles',
               'the picker')
        half = g.size(1)  # P2 picks in Window 1 beside P1
        g.ctl(2, 'pad', 'rb', 'tap')  # Window 1 -> New window
        g.settle([1], [(2 * half[0], half[1])])
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Sam')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Sam in the world')
        check('P1 has Window 1 to itself', g.settle([1], [(2 * half[0], half[1])]), [(2 * half[0], half[1])])
        check('Sam plays in its own window', g.size(2) != g.size(1), True)
    finally:
        g.quit()


def scenario_profile_availability():
    print('profile_availability: long profile lists and a profile freed while another picker stays open')
    def seed(home):
        for name in ['Ada', 'Bea', 'Cal', 'Dee', 'Eve', 'Fay', 'Gia', 'Hal', 'Ivy']:
            (home / 'splitscreen' / 'profiles' / name).mkdir(parents=True)
    g = Game(3, pick=True, seed=seed)
    try:
        menu = lambda n: (g.ctl(n, 'state').get('menu') or {})
        g.wait(lambda: all(menu(n).get('mode') == 'splitscreen-profiles' for n in [2, 3]),
               'both pickers')
        g.press(2, 'Next')
        g.press(2, 'Next')
        g.press(2, 'Ivy')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Ivy playing')
        g.wait(lambda: 'Ivy' in menu(3).get('taken', []), 'Ivy disabled in the open picker')
        check('the ninth profile joins', True, True)
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'Ivy leaving')
        g.wait(lambda: 'Ivy' not in menu(3).get('taken', []), 'Ivy available without reopening')
        g.press(3, 'Next')
        g.press(3, 'Next')
        g.press(3, 'Ivy')
        g.wait(lambda: g.ctl(3, 'state').get('player') is not None, 'Ivy joining again')
        check('the open picker can select the freed profile', True, True)
    finally:
        g.quit()


def scenario_gui_scales():
    print('gui_scales: each player keeps its own GUI scale across turns and settings changes')
    g = Game(2)
    try:
        g.ctl(1, 'options-set', 'guiScale', '1')
        g.ctl(2, 'options-set', 'guiScale', '2')
        for n in [1, 2]:
            g.ctl(n, 'key', 'esc', 'tap')
        def sizes():
            return [g.ctl(n, 'ui')['gui_size'] for n in [1, 2]]
        g.wait(lambda: sizes()[0][0] == 2 * sizes()[1][0], 'independent GUI scales')
        check('P1 GUI twice as wide at scale 1', sizes()[0][0], 2 * sizes()[1][0])
        g.ctl(2, 'options-set', 'guiScale', '1')
        g.wait(lambda: sizes()[0] == sizes()[1], 'P2 scale changed independently')
        check('equal scales give equal GUI sizes', sizes()[0], sizes()[1])
    finally:
        g.quit()


def scenario_fullscreen():
    print('fullscreen: a profile that was fullscreen joins')

    def seed(home):
        profile = home / 'splitscreen/profiles/P2'
        profile.mkdir(parents=True)
        (profile / 'options.txt').write_text('fullscreen:true\n')

    g = Game(2, seed=seed)
    try:
        time.sleep(3)
        value = g.ctl(1, 'options-get', 'fullscreen')['options']['fullscreen']['value']
        check("P1's window stays windowed (P1's fullscreen option)", value, False)
        # P2's F11 in a window of its own: that window, and only it, goes fullscreen and back.
        g.play_on(2, 'Window 2')
        windowed = g.size(2)
        g.ctl(2, 'key', 'f11', 'tap')
        try:
            g.wait(lambda: g.size(2)[0] > windowed[0], "P2's window fullscreen", seconds=10)
        except RuntimeError:
            # macOS ignores a fullscreen request while another window's transition runs: again.
            g.ctl(2, 'key', 'f11', 'tap')
            g.wait(lambda: g.size(2)[0] > windowed[0], "P2's window fullscreen", seconds=20)
        check("P2's F11: its window fills the screen", g.size(2)[0] > windowed[0], True)
        value = g.ctl(1, 'options-get', 'fullscreen')['options']['fullscreen']['value']
        check("P1's window stays windowed", value, False)
        full = g.size(2)
        time.sleep(2)  # macOS ignores a change while its fullscreen animation runs
        g.ctl(2, 'key', 'f11', 'tap')
        try:
            g.wait(lambda: g.size(2) == windowed, "P2's window back", seconds=10)
        except RuntimeError:
            print(f'    (windowed {windowed}, fullscreen {full}, now {g.size(2)})')
        check("P2's F11 again: back to its window", g.size(2), windowed)
    finally:
        g.quit()


def scenario_bumpers():
    print('bumpers: a joining player moves its view with LB and RB, then picks a profile')
    g = Game(2, pick=True)
    try:
        g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles',
               'the picker')
        half = g.size(1)
        whole = (2 * half[0], half[1])
        g.ctl(2, 'pad', 'rb', 'tap')
        check('RB: P2 opens Window 2, P1 has Window 1 to itself', g.settle([1], [whole]), [whole])
        own = g.size(2)
        g.ctl(2, 'pad', 'lb', 'tap')
        check('LB: back beside P1 in Window 1', g.settle([1, 2], [half, half]), [half, half])
        g.ctl(2, 'pad', 'rb', 'tap')
        g.settle([2], [own])
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Ann')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Ann in the world')
        check('Ann joins where the bumpers put her', g.settle([1, 2], [whole, own]), [whole, own])
    finally:
        g.quit()
    print('bumpers: LB puts a joining player before P1 in Window 1, and it stays there')
    g = Game(2, pick=True)
    try:
        g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles',
               'the picker')
        half = g.size(1)
        g.ctl(2, 'pad', 'lb', 'tap')  # from "Window 1 (2 of 2)" to "(1 of 2)"
        time.sleep(0.5)
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Bea')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Bea in the world')
        check('still halves of Window 1', g.settle([1, 2], [half, half]), [half, half])
        # Who is first shows on the Setup screen's map.
        g.setup(1)
        tiles = {w['label']: w['bounds'] for w in g.ctl(1, 'ui')['widgets']}
        check('Bea before P1: left of it', tiles['Bea'][0] < tiles['P1'][0], True)
        g.press(1, 'Done')
    finally:
        g.quit()


def scenario_leave():
    print("leave: from the pause menu, or switch profile from Splitscreen Settings")
    g = Game(3)
    try:
        def settings(n):
            """Player n's pause menu, then its Splitscreen Settings."""
            g.ctl(n, 'key', 'esc', 'tap')
            g.wait(lambda: 'Splitscreen Settings' in g.labels(n), f"P{n}'s pause menu", seconds=5)
            g.press(n, 'Splitscreen Settings')

        def menu(n):
            return (g.ctl(n, 'state').get('menu') or {}).get('mode')
        g.ctl(3, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(3), "P3's pause menu", seconds=5)
        g.press(3, 'Leave')
        g.wait(lambda: not g.alive(3), 'P3 gone', seconds=10)
        w, h = g.size(1)
        check('Leave: P3 left, P1 and P2 share the window', g.settle([1, 2], [(w, h), (w, h)]), [(w, h), (w, h)])
        # P2's Splitscreen Settings: its profiles, still playing beneath.
        settings(2)
        g.wait(lambda: menu(2) == 'splitscreen-profiles', "P2's profiles", seconds=10)
        check('P2 still plays while it picks', g.ctl(2, 'state').get('player') is not None, True)
        check('its own profile marked, not pickable',
              [x.get('enabled') for x in g.ctl(2, 'ui')['widgets'] if x.get('label') == 'P2 - you'], [False])
        # Cancel: nothing changed.
        g.press(2, 'Cancel')
        time.sleep(1)
        check('Cancel: P2 plays on as P2', (g.alive(2), menu(2), g.size(2)), (True, None, (w, h)))
        g.close_page(2)
        # Again, RB to a new window, then a new profile: P2 comes back as Cam, there.
        settings(2)
        g.wait(lambda: menu(2) == 'splitscreen-profiles', "P2's profiles", seconds=10)
        g.ctl(2, 'pad', 'rb', 'tap')
        whole = (2 * w, h)
        check('RB on the page: P2 to a new window, P1 alone', g.settle([1], [whole]), [whole])
        g.press(2, 'New Profile...')
        g.ctl(2, 'type', 'Cam')
        g.press(2, 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None and menu(2) is None, 'Cam in the world',
               seconds=20)
        check('Cam plays in the new window', (g.size(1), g.size(2) != (w, h)), (whole, True))
        # P1's Splitscreen Settings is the Setup screen.
        settings(1)
        g.wait(lambda: 'Apply' in g.labels(1), "P1's Setup screen", seconds=5)
        g.press(1, 'Done')
        g.close_page(1)
    finally:
        g.quit()


def scenario_signedin():
    print('signedin: P1 signed into an account, another player joins under its own profile')
    session = Path(tempfile.mkdtemp(prefix='horizon-session-')) / 'session.json'
    session.write_text(json.dumps({'name': 'Signed', 'uuid': '069a79f444e94726a5befca90e38aaf5',
                                   'access_token': 'not-a-real-token', 'user_type': 'msa'}))
    session.chmod(0o600)
    g = Game(2, args=('--session-file', str(session)))
    try:
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'P2 in the world', seconds=20)
        time.sleep(2)
        check('P2 plays', g.ctl(2, 'state').get('player') is not None, True)
        check('P1 still plays (not replaced by its own name joining)',
              g.ctl(1, 'state').get('player') is not None and g.size(1) == g.size(2), True)
    finally:
        g.quit()


def scenario_monitor():
    print('monitor: a player fills another monitor, another joins it (skipped with one monitor)')
    g = Game(2)
    try:
        half = g.size(1)
        g.setup(1)
        g.press(1, 'P2')
        fill = [l for l in g.labels(1) if l.startswith('Fill Monitor')]
        if not fill:
            print('  skip one monitor only')
            return
        g.press(1, fill[0])
        g.press(1, 'Apply')
        g.close_page(1)
        g.wait(lambda: g.size(2) != half, "P2's window to fill the monitor", seconds=15)
        time.sleep(1)
        size = g.size(2)
        check('P2 fills the monitor: a window of its own', size != g.size(1), True)
        g.play_on(1, 'Window 2')
        half = (size[0] // 2, size[1])
        g.wait(lambda: g.size(1) == half, 'P1 beside P2 on the monitor', seconds=15)
        check('P1 joins it: halves of the monitor', (g.size(1), g.size(2)), (half, half))
    finally:
        g.quit()


def scenario_setup():
    print('setup: the Setup screen moves a player only on Apply; Revert undoes a draft')
    g = Game(2)
    try:
        w, h = g.size(1)
        g.setup(1)
        g.press(1, 'P2')
        g.press(1, 'New window')
        time.sleep(1)
        check('a draft changes nothing yet', (g.size(1), g.size(2)), ((w, h), (w, h)))
        g.press(1, 'Revert')
        g.press(1, 'P2')
        g.press(1, 'New window')
        g.press(1, 'Apply')
        g.wait(lambda: g.size(1) == (2 * w, h), 'P1 alone in Window 1 after Apply', seconds=15)
        check('Apply: P1 has Window 1 to itself', g.size(1), (2 * w, h))
        check('Apply: P2 in a window of its own', g.size(2) != (w, h), True)
        # Back beside P1 from the screen: its window closes.
        g.press(1, 'P2')
        g.press(1, 'Window 1')
        g.press(1, 'Apply')
        g.wait(lambda: g.size(2) == (w, h), 'P2 back beside P1', seconds=15)
        check('P2 back beside P1', (g.size(1), g.size(2)), ((w, h), (w, h)))
        g.press(1, 'Done')
        check('Done closes the screen', 'Apply' in g.labels(1), False)
        g.close_page(1)
    finally:
        g.quit()
    print("setup: another player's Splitscreen... (Horizon Settings) is its profiles, not Setup")
    g = Game(2)
    try:
        g.ctl(2, 'options', 'horizon')
        g.wait(lambda: 'Splitscreen...' in g.labels(2), "P2's Horizon Settings", seconds=15)
        g.ctl(2, 'click-button', 'Splitscreen...')
        g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles',
               "P2's profiles", seconds=10)
        check('no Setup screen for P2', 'Apply' in g.labels(2), False)
        g.press(2, 'Cancel')
        g.close_page(2)
        check('P2 plays on', g.ctl(2, 'state').get('player') is not None, True)
    finally:
        g.quit()
    print('setup: with the controller, A picks a player up, the stick moves it, A drops it')
    g = Game(2)
    try:
        w, h = g.size(1)
        g.setup(1)

        def tap(button):
            g.ctl(1, 'pad', button, 'tap')
            time.sleep(0.35)

        def apply_on():
            return [x.get('enabled') for x in g.ctl(1, 'ui')['widgets'] if x.get('label') == 'Apply']
        # The map's tiles come with the game's first sync of the screen.
        g.wait(lambda: any(l == 'P2' for l in g.labels(1)), "P2's tile", seconds=5)
        tap('right')  # the focus starts on P1's tile
        tap('right')  # P2's
        tap('south')  # picked up
        # The stick to the right three times: its two places in Window 1, then a new window (the
        # D-pad and LB/RB cycle the same way).
        for _ in range(3):
            g.ctl(1, 'pad', 'stick', 'left', '1', '0')
            time.sleep(0.25)  # a poll or two, under the menu's repeat (0.35 s)
            g.ctl(1, 'pad', 'stick', 'left', '0', '0')
            time.sleep(0.35)
        check('held: nothing in the draft yet', apply_on(), [False])
        tap('south')  # dropped there
        check('dropped: in the draft', apply_on(), [True])
        g.press(1, 'Apply')
        g.wait(lambda: g.size(1) == (2 * w, h), 'P1 alone in Window 1', seconds=15)
        check('P2 in a new window, P1 alone', (g.size(1), g.size(2) != (w, h)), ((2 * w, h), True))
    finally:
        g.quit()


@contextlib.contextmanager
def dedicated(directory=None, **extra):
    """A Horizon dedicated server, offline mode, started and ready: (server, port)."""
    from test_dedicated_server_parity import Server
    port = lab.free_port()
    folder = Path(tempfile.mkdtemp(prefix='hz-split-server-'))
    directory = Path(directory) if directory is not None else folder / 'server'
    directory.mkdir(parents=True, exist_ok=True)
    properties = {'online-mode': 'false', 'white-list': 'false', 'server-port': str(port), 'level-type': 'minecraft\\:flat',
                  'generate-structures': 'false', 'view-distance': '3', 'simulation-distance': '3',
                  'pause-when-empty-seconds': '-1', 'spawn-protection': '0', **extra}
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in properties.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server([lab.cargo_target() / 'fast/horizon-client', '--resources', RESOURCES, '--dedicated',
                     '--nogui'], directory, 'server')
    try:
        server.wait_for(r'Done \(')
        yield server, port
    finally:
        server.stop()
        shutil.rmtree(folder, ignore_errors=True)


def scenario_packs():
    print("packs: the server's resource pack: P1 says Yes, the others follow without being asked")
    from server_pack_e2e import Files, build_pack
    folder = Path(tempfile.mkdtemp(prefix='hz-split-pack-'))
    sha1 = build_pack(folder / 'pack.zip')
    files = Files({'/pack.zip': (folder / 'pack.zip').read_bytes()})
    try:
        with dedicated(**{'resource-pack': files.url('/pack.zip'), 'resource-pack-sha1': sha1,
                          'require-resource-pack': 'false'}) as (server, port):
            g = Game(2, args=('--server', f'127.0.0.1:{port}'), ready=False)
            try:
                g.wait(lambda: 'Yes' in g.labels(1), "P1 asked about the server's pack", seconds=60)
                g.ctl(1, 'click-button', 'Yes')
                server.wait_for(r'P2 joined the game', timeout=90)
                check("P2 joins on P1's Yes, never asked itself", True, True)
                # P2 finds P1's copy in their shared downloads folder (same SHA-1).
                downloads = [p for p, _ in files.requests if p == '/pack.zip']
                check('the pack downloaded once, for both', len(downloads), 1)
            finally:
                g.quit()
    finally:
        files.shutdown()
        shutil.rmtree(folder, ignore_errors=True)


def scenario_server():
    print('server: P1 on a dedicated server; the others join it, a kicked one drops out, all leave with P1')
    with dedicated() as (server, port):
        g = Game(3, args=('--server', f'127.0.0.1:{port}'))
        try:
            server.wait_for(r'P2 joined the game', timeout=60)
            server.wait_for(r'P3 joined the game', timeout=60)
            check('P2 and P3 join the server P1 is on', True, True)
            server.console('kick P3 Testing')
            g.wait(lambda: not g.alive(3), 'P3 gone after its kick', seconds=20)
            check('kicked: P3 drops out, P1 plays on', g.ctl(1, 'state').get('player') is not None, True)
            g.ctl(1, 'disconnect')
            server.wait_for(r'P2 left the game', timeout=30)
            check('P1 leaves: P2 leaves with it', True, True)
        finally:
            g.quit()


def scenario_eight():
    print('eight: eight players in one window')
    g = Game(8)
    try:
        sizes = g.sizes(range(1, 9))
        near = lambda a, b: abs(a - b) <= 1  # thirds of an odd width differ by a pixel
        # Rows of three, three and two: six equal views, then two wider ones; all the same height.
        check(f'rows of 3, 3 and 2: {sizes}',
              all(near(x[0], sizes[0][0]) and near(x[1], sizes[0][1]) for x in sizes[:6])
              and near(sizes[6][0], sizes[7][0]) and sizes[6][0] > sizes[0][0] + 100
              and all(near(x[1], sizes[0][1]) for x in sizes), True)
        drawing = lambda: all(g.ctl(n, 'status')['fps'] > 10 for n in range(1, 9))
        try:
            g.wait(drawing, 'every view drawing', seconds=20)
        except RuntimeError:
            pass
        check('every view draws', drawing(), True)
    finally:
        g.quit()



def scenario_sixteen():
    print('sixteen: sixteen players in a 4x4 grid, P16 moves to its own window and leaves')
    g = Game(16)
    try:
        players = range(1, 17)
        g.wait(lambda: all(g.ctl(n, 'state').get('player') is not None for n in players),
               'all sixteen players admitted to the world', seconds=60)
        check('all sixteen players enter the world', True, True)
        sizes = g.sizes(players)
        near = lambda a, b: abs(a - b) <= 1
        check('all sixteen views have equal dimensions',
              all(near(w, sizes[0][0]) and near(h, sizes[0][1]) for w, h in sizes), True)
        drawing = lambda: all(g.ctl(n, 'status')['fps'] > 0 for n in players)
        g.wait(drawing, 'all sixteen views drawing', seconds=30)
        check('every player renders', drawing(), True)
        print('  FPS per player:', [round(g.ctl(n, 'status')['fps'], 1) for n in players])
        g.play_on(16, 'New window')
        check('P16 gets its own larger view', g.size(16)[0] > sizes[15][0], True)
        g.ctl(16, 'disconnect')
        g.wait(lambda: not g.alive(16), 'P16 leaving')
        check('P15 still plays after P16 leaves', g.ctl(15, 'state').get('player') is not None, True)
    finally:
        g.quit()


def scenario_pointer():
    print('pointer: only the player with the keyboard and mouse grabs the pointer')
    g = Game(2)
    try:
        time.sleep(3)
        grabbed = lambda n: g.ctl(n, 'state').get('pointer_grabbed')  # noqa: E731
        g.ctl(1, 'mouse', 'left', 'click')
        time.sleep(0.5)
        check('P1 plays with the mouse, P2 never grabs it', (grabbed(1), grabbed(2)), (True, False))
        # P2's menus (its controller) leave P1's pointer alone.
        g.ctl(2, 'key', 'e', 'tap')
        time.sleep(0.6)
        g.ctl(2, 'key', 'esc', 'tap')
        time.sleep(0.6)
        check("after P2's inventory: still P1's", (grabbed(1), grabbed(2)), (True, False))
    finally:
        g.quit()



def scenario_launcher():
    print("launcher: its other accounts are profiles; offline ones stay in this instance")
    folder = Path(tempfile.mkdtemp(prefix='hz-launcher-'))
    # Another signed-in account's session, as the launcher writes it (a token nobody checks:
    # P1's own world is offline-mode).
    session = folder / 'session-ann.json'
    session.write_text(json.dumps({'name': 'Ann', 'uuid': '069a79f444e94726a5befca90e38aaf5',
                                   'access_token': 'not-a-real-token', 'user_type': 'msa'}))
    session.chmod(0o600)
    os.environ['HORIZON_ACCOUNT_SESSIONS'] = json.dumps(
        [{'name': 'Ann', 'uuid': '069a79f444e94726a5befca90e38aaf5', 'file': str(session)}])
    g = Game(3, pick=True, seed=lambda home: (home / 'splitscreen/profiles/Bea').mkdir(parents=True))
    try:
        menu = lambda n: g.ctl(n, 'state').get('menu') or {}  # noqa: E731
        g.wait(lambda: menu(2).get('mode') == 'splitscreen-profiles', 'the picker')
        check('the accounts, then the offline profiles', menu(2).get('profiles'), ['Ann', 'Bea'])
        check('the account marked', 'Ann (Microsoft)' in g.labels(2), True)
        g.ctl(2, 'click-button', 'Ann (Microsoft)')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Ann in the world')
        g.wait(lambda: menu(3).get('mode') == 'splitscreen-profiles', "P3's picker")
        g.ctl(3, 'click-button', 'New Profile...')
        g.ctl(3, 'type', 'Dan')
        g.ctl(3, 'click-button', 'Create')
        g.wait(lambda: g.ctl(3, 'state').get('player') is not None, 'Dan in the world')
        check('a new offline profile is kept in the instance',
              (g.home / 'splitscreen/profiles/Dan').is_dir(), True)
    finally:
        g.quit()
        os.environ.pop('HORIZON_ACCOUNT_SESSIONS', None)



def scenario_pausemenu():
    print("pausemenu: Splitscreen Settings in the pause menu only while another player has joined")
    g = Game(2, pick=True)
    try:
        def paused(n):
            """Player n's pause menu: its buttons, then closed again."""
            g.ctl(n, 'key', 'esc', 'tap')
            g.wait(lambda: 'Options...' in g.labels(n), f"P{n}'s pause menu", seconds=5)
            labels = g.labels(n)
            g.close_page(n)
            return labels
        menu = lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode')  # noqa: E731
        g.wait(lambda: menu() == 'splitscreen-profiles', "P2's picker")
        # P2 still choosing: P1 plays alone, so no Splitscreen Settings; Horizon Settings has it.
        check('alone: none in the pause menu', 'Splitscreen Settings' in paused(1), False)
        g.page(1)
        check('alone: Horizon Settings > Splitscreen... still there', 'Done' in g.labels(1), True)
        g.close_page(1)
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Dee')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Dee in the world')
        time.sleep(1)
        check('joined: in P1\'s pause menu', 'Splitscreen Settings' in paused(1), True)
        check("joined: in Dee's", 'Splitscreen Settings' in paused(2), True)
        # Dee leaves: P1 alone again, gone.
        g.ctl(2, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(2), "Dee's pause menu", seconds=5)
        g.press(2, 'Leave')
        g.wait(lambda: not g.alive(2), 'Dee gone', seconds=10)
        time.sleep(1)
        check('alone again: none in the pause menu', 'Splitscreen Settings' in paused(1), False)
    finally:
        g.quit()


def pick_profile(g, n, name):
    """Choose or create an instance-local offline profile, through the actual picker."""
    g.wait(lambda: (g.ctl(n, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles',
           f"P{n}'s picker")
    if name in (g.ctl(n, 'state').get('menu') or {}).get('profiles', []):
        for _ in range(8):
            if name in g.labels(n):
                g.press(n, name)
                break
            g.press(n, 'Next')
        else:
            raise RuntimeError(f'profile {name} never appeared')
    else:
        g.press(n, 'New Profile...')
        g.ctl(n, 'type', name)
        g.press(n, 'Create')
    g.wait(lambda: g.ctl(n, 'state').get('player') is not None, f'{name} playing')


def progress(g, n):
    p = g.ctl(n, 'state').get('player') or {}
    return {'diamonds': g.ctl(n, 'inventory')['counts'].get('minecraft:diamond', 0),
            'position': [round(v, 2) for v in p.get('position', [])],
            'health': p.get('health'), 'xp': p.get('xp')}


def prepare_progress(run, name, count, levels, x):
    for command in ['gamerule natural_health_regeneration false',
                    'fill 0 99 0 31 99 31 minecraft:stone',
                    'fill 0 100 0 31 105 31 minecraft:air',
                    f'gamemode survival {name}', f'clear {name}',
                    f'give {name} minecraft:diamond {count}',
                    f'xp set {name} {levels} levels', f'xp set {name} 0 points',
                    f'tp {name} {x} 100 8.5', f'damage {name} 4 minecraft:generic']:
        run(command)


def await_progress(g, n, count, levels, x):
    def ready():
        p = progress(g, n)
        return (p['diamonds'] == count and p['health'] == 16
                and p['position'] == [x, 100.0, 8.5] and (p['xp'] or {}).get('level') == levels)
    try:
        g.wait(ready, f'P{n} inventory, position, health and XP', seconds=20)
    except RuntimeError as error:
        raise RuntimeError(f'{error}; actual: {progress(g, n)}') from error
    return progress(g, n)


def scenario_dedicated_profiles():
    print('dedicated_profiles: guest becomes independent P1; identities survive server restart')
    with tempfile.TemporaryDirectory(prefix='hz-profile-server-') as folder:
        directory = Path(folder) / 'server'
        with dedicated(directory=directory) as (server, port):
            g = Game(2, pick=True, args=('--server', f'127.0.0.1:{port}'))
            try:
                pick_profile(g, 2, 'jordank')
                prepare_progress(server.console, 'jordank', 7, 12, 8.5)
                expected = await_progress(g, 2, 7, 12, 8.5)
                g.ctl(2, 'disconnect')
                g.wait(lambda: not g.alive(2), 'Jordan leaving splitscreen')
                remote = Game(1, args=('--server', f'127.0.0.1:{port}', '--name', 'jordank'))
                try:
                    check('independent P1 uses the guest save', await_progress(remote, 1, 7, 12, 8.5), expected)
                finally:
                    remote.quit()
            finally:
                g.quit()
        with dedicated(directory=directory) as (server, port):
            for name in ['someone_else', 'jordank']:
                g = Game(1, args=('--server', f'127.0.0.1:{port}', '--name', name))
                try:
                    if name == 'jordank':
                        check('server restart keeps Jordan progress', await_progress(g, 1, 7, 12, 8.5), expected)
                    else:
                        check('a different offline name has separate inventory',
                              progress(g, 1)['diamonds'], 0)
                finally:
                    g.quit()


def scenario_integrated_profiles():
    print('integrated_profiles: P1 rename keeps solo save; Jordan moves from splitscreen to LAN')
    g = Game(2, pick=True)
    home = g.home
    try:
        pick_profile(g, 2, 'jordank')
        run = lambda command: g.ctl(1, 'command', command)
        prepare_progress(run, 'P1', 3, 5, 20.5)
        owner = await_progress(g, 1, 3, 5, 20.5)
        prepare_progress(run, 'jordank', 7, 12, 8.5)
        jordan = await_progress(g, 2, 7, 12, 8.5)
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'Jordan leaving')
        port = g.ctl(1, 'lan', 'open', '0', 'commands')['port']
        remote = Game(1, args=('--server', f'127.0.0.1:{port}', '--name', 'jordank'))
        try:
            check('LAN Jordan uses splitscreen progress', await_progress(remote, 1, 7, 12, 8.5), jordan)
        finally:
            remote.quit()
    finally:
        g.quit()
    g = Game(1, home=home, args=('--name', 'AnotherName'))
    try:
        check('changing P1 launcher name keeps solo progress', await_progress(g, 1, 3, 5, 20.5), owner)
    finally:
        g.quit()


SCENARIOS = {
    'join': scenario_join,
    'windows': scenario_windows,
    'three': scenario_three,
    'live': scenario_live,
    'picker': scenario_picker,
    'fullscreen': scenario_fullscreen,
    'eight': scenario_eight,
    'bumpers': scenario_bumpers,
    'leave': scenario_leave,
    'signedin': scenario_signedin,
    'monitor': scenario_monitor,
    'setup': scenario_setup,
    'server': scenario_server,
    'packs': scenario_packs,
    'pointer': scenario_pointer,
    'launcher': scenario_launcher,
    'pausemenu': scenario_pausemenu,
    'profile_availability': scenario_profile_availability,
    'gui_scales': scenario_gui_scales,
    'sixteen': scenario_sixteen,
    'dedicated_profiles': scenario_dedicated_profiles,
    'integrated_profiles': scenario_integrated_profiles,
}

# Related scenarios, for `--only @name`.
GROUPS = {
    'layout': ['windows', 'three', 'eight', 'sixteen', 'monitor', 'setup'],
    'joining': ['join', 'picker', 'fullscreen', 'bumpers', 'leave', 'pausemenu'],
    'menus': ['leave', 'setup', 'pausemenu'],
    'controller': ['bumpers', 'setup', 'pointer'],
    'accounts': ['signedin', 'launcher'],
    'server': ['server', 'packs', 'dedicated_profiles', 'integrated_profiles'],
}


def select(words):
    """The scenarios `--only` names, in the suite's order: names, numbers from --list (`12`),
    ranges (`10-12`, `15-`), groups (`@layout`)."""
    names = list(SCENARIOS)
    chosen = set()
    for word in words:
        if word.startswith('@'):
            if word[1:] not in GROUPS:
                sys.exit(f'no group {word}: {", ".join("@" + g for g in GROUPS)}')
            chosen.update(GROUPS[word[1:]])
        elif word[0].isdigit():
            first, dash, last = word.partition('-')
            first, last = int(first), int(last) if last else (len(names) if dash else int(first))
            if not 1 <= first <= last <= len(names):
                sys.exit(f'{word}: scenarios are 1-{len(names)} (see --list)')
            chosen.update(names[first - 1:last])
        elif word in SCENARIOS:
            chosen.add(word)
        else:
            sys.exit(f'no scenario {word!r} (see --list)')
    return [n for n in names if n in chosen]


VERBOSE = False
STARTED = time.monotonic()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--only', nargs='+', metavar='WHICH',
                    help='run only these: names, numbers (12), ranges (10-12, 15-) or groups (@layout)')
    ap.add_argument('--list', action='store_true', help='list the scenarios, numbered, and the groups')
    ap.add_argument('--verbose', '-v', action='store_true',
                    help='print every horizon-ctl command, its reply and each screen change (stderr)')
    args = ap.parse_args()
    global VERBOSE
    VERBOSE = args.verbose
    sys.stdout.reconfigure(line_buffering=True)
    if args.list:
        for i, (name, run) in enumerate(SCENARIOS.items(), 1):
            # What it checks: the line it prints first ("name: what").
            said = re.search(r"print\(f?(['\"])(.*?)\1", inspect.getsource(run))
            what = said.group(2).partition(': ')[2] if said else ''
            print(f'{i:3} {name:11} {what}')
        print('groups: ' + '  '.join(f'@{g} ({", ".join(m)})' for g, m in GROUPS.items()))
        return
    which = select(args.only) if args.only else list(SCENARIOS)
    if not CTL.exists():
        sys.exit(f'{CTL} is missing: cargo build --profile fast -p horizon-client')
    if not (RESOURCES / 'assets').exists():
        sys.exit(f'no prepared resources in {RESOURCES}')
    numbers = {name: i for i, name in enumerate(SCENARIOS, 1)}
    for name in which:
        # The number to give --only, to run this one again.
        print(f'[{numbers[name]}/{len(SCENARIOS)}] {name}')
        try:
            SCENARIOS[name]()
        except RuntimeError as e:
            print(f'  FAIL {name}: {e}')
            failures.append(name)
    if failures:
        sys.exit(f'{len(failures)} check(s) failed: {", ".join(failures)}')
    print('splitscreen: every check passed')


if __name__ == '__main__':
    main()
