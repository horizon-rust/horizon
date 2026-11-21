#!/usr/bin/env python3
"""Compare native text_from_nbt with the checksum-pinned 26.3 client codec.

Example: python tools/test_native_text_nbt_parity.py --client-jar PATH --libraries PATH
The libraries directory is the launcher's Maven-style libraries tree. Mojang's
files stay outside the repository; the harness never downloads them.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ORACLE = ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaNativeTextNbtOracle.java'
CASES = [
    "{text:'plain'}", "{text:1}", "{unknown:'x'}", "{text:'x',bold:1b}",
    "{text:'x',bold:0b}", "{text:'x',color:'gold'}", "{text:'x',color:'not-a-color'}",
    "{text:'x',click_event:{action:'change_page',page:2}}",
    "{text:'x',click_event:{action:'change_page',page:2b}}",
    "{text:'x',click_event:{action:'change_page',page:2s}}",
    "{text:'x',click_event:{action:'change_page',page:2L}}",
    "{text:'x',click_event:{action:'change_page',page:2.0d}}",
    "{text:'x',click_event:{action:'change_page',page:2.0f}}",
    "{text:'x',click_event:{action:'change_page',page:2.5d}}",
    "{text:'x',click_event:{action:'change_page',page:2147483647.5d}}",
    "{text:'x',click_event:{action:'change_page',page:2.9f}}",
    "{text:'x',click_event:{action:'change_page',page:0.5d}}",
    "{text:'x',click_event:{action:'change_page',page:-1.5d}}",
    "{text:'x',click_event:{action:'change_page',page:0}}",
    "{text:'x',click_event:{action:'change_page',page:'two'}}",
    "{text:'x',extra:[{text:'child',click_event:{action:'change_page',page:2.0d}}]}",
    "{text:'x',hover_event:{action:'show_text',value:{text:'child',click_event:{action:'change_page',page:2}}}}",
    "{text:'x',hover_event:{action:'show_text',value:{text:1}}}",
    "{text:'x',extra:[{text:'child',click_event:{action:'change_page',page:0}}]}",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client-jar', type=Path, default=ROOT / '.local/minecraft/26.3/jars/client.jar')
    parser.add_argument('--libraries', type=Path, default=ROOT / '.local/minecraft/26.3/libraries/client')
    args = parser.parse_args()
    expected = json.loads((ROOT / 'research-lock.json').read_text())['minecraft']['client_sha1']
    if hashlib.sha1(args.client_jar.read_bytes()).hexdigest() != expected:
        raise SystemExit('Client JAR does not match the pinned 26.3 checksum')
    libraries = sorted(args.libraries.rglob('*.jar'))
    if not libraries:
        raise SystemExit('No Java libraries found')
    cp = os.pathsep.join(map(str, [args.client_jar.resolve(), *libraries]))
    with tempfile.TemporaryDirectory(prefix='horizon-text-nbt-') as tmp:
        tmp = Path(tmp)
        cases, output = tmp / 'cases.txt', tmp / 'vanilla.txt'
        cases.write_text('\n'.join(CASES) + '\n', encoding='utf-8')
        subprocess.run(['javac', '--release', '25', '-cp', cp, '-d', str(tmp), str(ORACLE)], check=True, cwd=ROOT)
        subprocess.run(['java', '-cp', str(tmp) + os.pathsep + cp,
                        'dev.horizonlab.parity.VanillaNativeTextNbtOracle', str(cases), str(output)],
                       check=True, cwd=ROOT)
        env = dict(os.environ, HORIZON_TEXT_NBT_CASES=str(cases), HORIZON_TEXT_NBT_ORACLE=str(output))
        subprocess.run(['cargo', 'test', '--locked', '-p', 'horizon-client', '--lib',
                        'game::native_text_nbt::tests::vanilla_nbt_oracle', '--', '--ignored', '--exact'], check=True, cwd=ROOT, env=env)
    print(f'{len(CASES)} NBT component cases matched vanilla 26.3')


if __name__ == '__main__':
    main()

