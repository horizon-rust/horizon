#!/usr/bin/env python3
"""Horizon's client on a vanilla 26.3 dedicated server, end to end.

A vanilla server (the pinned server jar, offline mode, a flat world) and one Horizon game joined to it,
driven by horizon-ctl. The server's console does what a server or its plugins do; the checks read what
the game shows and what reaches the server (its console and, for custom click actions, its debug log).

    python3 tools/server_e2e.py --bin-dir target/fast             # every check
    python3 tools/server_e2e.py --bin-dir target/fast --check dialog

Checks:
- dialog: `/dialog show` with a multi-action dialog (a message, a text and a boolean input, three
          buttons): the game shows it with its inputs and buttons, the text typed and the box ticked
          go back in a command template (`trigger`, run by the server) and in a custom action's
          payload (the server's "Received custom click action" line), `wait_for_response` leaves the
          waiting screen, and `/dialog clear` closes a dialog (servers used to see nothing: the game
          dropped the packets).
- titles: a chest named with styles (`CustomName`, as a plugin's menu is): its screen's title keeps
          the colours and bold (it used to be flattened to plain grey).
- dialog_text: custom click events inherited by message text and inline sprites send their id and
               optional payload, with the dialog's none/close/wait_for_response screen behaviour.

The game runs with fresh HORIZON_HOME, launcher and XDG folders and without audio. Screenshots and both
logs go to --out.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import test_rust_parity as base  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from lan_e2e import Game  # noqa: E402
from test_dedicated_server_parity import Server  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
PLAYER = 'Steve'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
    'pause-when-empty-seconds': '-1',
}
# MinecraftServer.handleCustomClickAction logs at debug level: the console shows that logger's debug lines.
LOG4J = """<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
  <Appenders><Console name="out" target="SYSTEM_OUT"><PatternLayout pattern="[%level] %msg%n"/></Console></Appenders>
  <Loggers>
    <Logger name="net.minecraft.server.MinecraftServer" level="debug"/>
    <Root level="info"><AppenderRef ref="out"/></Root>
  </Loggers>
</Configuration>
"""


def snbt(value):
    """A JSON value as SNBT (strings quoted, booleans as bytes)."""
    if isinstance(value, bool):
        return '1b' if value else '0b'
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return '[' + ','.join(snbt(v) for v in value) + ']'
    if isinstance(value, dict):
        return '{' + ','.join(f'{k}:{snbt(v)}' for k, v in value.items()) + '}'
    return str(value)


def check_dialog(server, game, out):
    server.console('scoreboard objectives add answer trigger')
    server.console(f'scoreboard players enable {PLAYER} answer')
    dialog = {
        'type': 'minecraft:multi_action',
        'title': {'text': 'Sign in', 'color': 'gold'},
        'body': {'type': 'minecraft:plain_message', 'contents': 'Pick a number.'},
        'inputs': [
            {'type': 'minecraft:text', 'key': 'number', 'label': 'Number'},
            {'type': 'minecraft:boolean', 'key': 'agree', 'label': 'Agree', 'on_true': '1', 'on_false': '0'},
        ],
        'actions': [
            {'label': 'Send', 'action': {'type': 'minecraft:dynamic/run_command',
                                         'template': 'trigger answer set $(number)$(agree)'}},
            {'label': 'Submit', 'action': {'type': 'minecraft:dynamic/custom', 'id': 'horizon:e2e',
                                           'additions': {'tag': 'x'}}},
            {'label': 'Wait', 'action': {'type': 'minecraft:dynamic/custom', 'id': 'horizon:wait'}},
        ],
        'after_action': 'none',
        'pause': False,  # a pausing dialog must close after an action
    }
    shown = {}
    server.console(f'dialog show {PLAYER} {snbt(dialog)}')
    game.raw('wait-for', 'screen', 'dialog', '--timeout', 30)
    state = game('state')
    shown['shown'] = state['dialog']['open']
    assert shown['shown']['title'] == 'Sign in', shown
    assert [i['key'] for i in shown['shown']['inputs']] == ['number', 'agree'], shown
    assert shown['shown']['buttons'] == ['Send', 'Submit', 'Wait'], shown
    game.shot(out, 'dialog')
    # The text, the box, then the command the template makes of them.
    game.raw('type', '4')
    game.raw('click', 'agree')
    game.raw('click', 'send')
    time.sleep(1)
    server.console(f'scoreboard players get {PLAYER} answer')
    server.wait_for(rf'{PLAYER} has 41 \[answer\]', timeout=30)
    shown['trigger'] = 41
    # A custom action: its additions and every input's tag.
    game.raw('click', 'submit')
    server.wait_for(r'Received custom click action horizon:e2e with payload', timeout=30)
    line = next(l for l in server.lines if 'horizon:e2e' in l)
    shown['custom'] = line.split('payload ', 1)[1]
    for part in ('agree:1b', 'number:"4"', 'tag:"x"'):
        assert part in shown['custom'], shown
    # after_action none: still open. Then clear it.
    assert game('state')['dialog']['open'], 'the dialog closed after an action with after_action none'
    server.console(f'dialog clear {PLAYER}')
    game.raw('wait-for', 'screen', 'playing', '--timeout', 30)
    # wait_for_response: the waiting screen until the server answers (here, never).
    server.console(f'dialog show {PLAYER} {snbt(dict(dialog, after_action="wait_for_response"))}')
    game.raw('wait-for', 'screen', 'dialog', '--timeout', 30)
    game.raw('click', 'wait')
    server.wait_for(r'Received custom click action horizon:wait', timeout=30)
    shown['waiting'] = game('state')['dialog']['screen']
    game.shot(out, 'waiting')
    assert shown['waiting'] == 'waiting_for_response', shown
    # Its Back works after 5 s, and so does Escape (clear_dialog leaves it open, as in vanilla).
    time.sleep(5.5)
    game.raw('key', 'esc', 'tap')
    game.raw('wait-for', 'screen', 'playing', '--timeout', 10)
    return shown


def check_titles(server, game, out):
    # AbstractContainerScreen draws the title the server sent, its styles included (#1645). A named
    # chest is what a plugin's menu looks like: its CustomName is the open_screen title.
    x, y, z = (int(v // 1) for v in game('state')['player']['position'])
    chest = (x + 2, y, z)
    name = [{'text': 'Server ', 'color': 'gold', 'bold': True}, {'text': 'Selector', 'color': 'aqua'}]
    server.console(f'setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest{{CustomName:{snbt(name)}}}')
    game.wait(10)
    game.raw('interact', *chest)
    game.raw('wait-for', 'screen', 'container', '--timeout', 30)
    game.wait(5)
    title = game('screen').get('title')
    game.shot(out, 'titles')
    game.raw('key', 'esc', 'tap')
    # A list is its first component with the rest as its children: they inherit its bold.
    want = [{'text': 'Server ', 'color': '#FFAA00', 'bold': True, 'italic': False},
            {'text': 'Selector', 'color': '#55FFFF', 'bold': True, 'italic': False}]
    assert title == want, title
    return {'title': title}


def check_dialog_text(server, game, out):
    """Custom component clicks (including an inherited inline sprite) reach the real server."""
    results = {}
    for after in ('none', 'close', 'wait_for_response'):
        event = {'action': 'custom', 'id': f'horizon:text_{after}'}
        if after != 'none':
            event['payload'] = {'shop': 'potions', 'amount': 3}
        dialog = {
            'type': 'minecraft:multi_action', 'title': 'Text actions', 'pause': False,
            'after_action': after,
            'body': {'type': 'minecraft:plain_message', 'width': 325, 'contents': {
                'text': '', 'click_event': event, 'extra': [
                    {'object': 'atlas', 'atlas': 'minecraft:items', 'sprite': 'minecraft:item/potion'},
                    ' Shop',
                ],
            }},
            'actions': [{'label': 'Unused'}],
        }
        server.console(f'dialog show {PLAYER} {snbt(dialog)}')
        game.raw('wait-for', 'screen', 'dialog', '--timeout', 30)
        # Read the rendered layout without depending on simulation ticks: dialogs are UI.
        width = game.raw('ui')['gui_size'][0]
        # The single message starts at y=63 with four pixels of padding. Blank space
        # within its 325-wide widget must not execute the component's click event.
        game.raw('click-at', width / 2 + 100, 70)
        time.sleep(0.25)
        assert not any(f'Received custom click action {event["id"]} ' in line for line in server.lines)
        # The icon and the letters inherit the same event. Exercise both paths.
        game.raw('click-at', width / 2 - 15 if after == 'none' else width / 2 + 3, 70)
        server.wait_for(rf'Received custom click action {event["id"]} with payload', timeout=10)
        line = next(line for line in server.lines if f'Received custom click action {event["id"]} ' in line)
        if 'payload' in event:
            assert 'shop:"potions"' in line and 'amount:3' in line, line
        else:
            assert 'with payload null' in line, line
        state = game('state')
        if after == 'none':
            assert state['dialog']['open'], state['dialog']
            server.console(f'dialog clear {PLAYER}')
        elif after == 'wait_for_response':
            assert state['dialog']['screen'] == 'waiting_for_response', state['dialog']
            game.shot(out, 'text-waiting')
            time.sleep(5.5)
            game.raw('key', 'esc', 'tap')
        game.raw('wait-for', 'screen', 'playing', '--timeout', 10)
        results[after] = line
    return results


CHECKS = {'dialog': check_dialog, 'dialog_text': check_dialog_text, 'titles': check_titles}


def run(bins, out, checks):
    lock, jars, libraries = base.verify_artifacts()
    work = Path(tempfile.mkdtemp(prefix='hz-server-e2e-'))
    port = lab.free_port()
    directory = work / 'server'
    directory.mkdir()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    (work / 'log4j2.xml').write_text(LOG4J)
    classpath = os.pathsep.join(map(str, [jars['server'], *libraries]))
    server = Server(['java', '-Xmx2g', f'-Dlog4j.configurationFile={work / "log4j2.xml"}', '-cp', classpath,
                     'net.minecraft.server.Main', 'nogui'], directory, 'vanilla')
    game = Game('client', bins / 'horizon-ctl', lab.RESOURCES.resolve())
    for key in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'HORIZON_LAUNCHER_HOME'):
        game.d.env[key] = str(game.work / key.lower())
    game.d.env['HORIZON_LAUNCHER_NO_SELF_UPDATE'] = '1'
    results = {}
    try:
        server.wait_for(r'Done \(')
        server.console('gamerule spawn_mobs false')
        game('launch', wait=False, offscreen=sys.platform == 'darwin')
        game.raw('connect', f'127.0.0.1:{port}', PLAYER)
        game.raw('wait-for', 'loaded', '--timeout', 300)
        server.wait_for(rf'{PLAYER} joined the game')
        server.console(f'op {PLAYER}')
        game.wait(20)
        for name in checks:
            try:
                results[name] = {'pass': True, 'detail': CHECKS[name](server, game, out)}
            except (Failed, AssertionError, StopIteration, KeyError) as e:
                results[name] = {'pass': False, 'detail': repr(e)}
            print(f'[server] {name}: {"PASS" if results[name]["pass"] else "FAIL"} {results[name]["detail"]}')
    finally:
        (out / 'client.log').write_text(game.close())
        server.stop()
        shutil.copy(server.log_path, out / 'server.log')
        shutil.rmtree(game.work, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build with the fast profile)')
    parser.add_argument('--check', action='append', choices=list(CHECKS), help='run only these checks (repeatable)')
    parser.add_argument('--out', default=str(ROOT / '.local/server-e2e'), help='screenshots and logs go here')
    args = parser.parse_args()
    if args.bin_dir:
        bins = Path(args.bin_dir).resolve()
    else:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib', '--bins'],
                       cwd=ROOT, check=True)
        bins = ROOT / 'target/fast'
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    results = run(bins, out, args.check or list(CHECKS))
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)


if __name__ == '__main__':
    main()
