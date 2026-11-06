#!/usr/bin/env python3
"""Compare real crystal/cart explosion causes, death messages and server kill credit."""
import argparse
import difflib
import json
import os
from pathlib import Path
import subprocess
import tempfile

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

FIXTURE = 'game::minecarts::cause_tests::explosion_causing_entities_parity_fixture'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vanilla-home', type=Path, help='prepared .local directory')
    parser.add_argument('--resources', type=Path)
    parser.add_argument('--test-binary', type=Path, help='already built horizon-client library test executable')
    args = parser.parse_args()
    if args.vanilla_home:
        lab.LOCAL = args.vanilla_home.resolve()
    build = lab.ROOT / '.local' / 'rust-explosion-causes-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    source = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaExplosionCausesOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    expected, actual = build / 'java.txt', build / 'rust.txt'
    actual.unlink(missing_ok=True)
    def run(command, name, **kwargs):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            subprocess.run(list(map(str, command)), check=True, stdout=log, stderr=subprocess.STDOUT, **kwargs)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath',
         lab.ROOT / 'mods/horizon/src/test/java', '-cp', cp, '-d', classes, source], 'compile-java')
    with tempfile.TemporaryDirectory(prefix='horizon-explosion-causes-') as scratch:
        env = dict(os.environ)
        for key in ['HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME']:
            env[key] = str(Path(scratch) / key.lower())
        env.update(HORIZON_NO_AUDIO='1', HORIZON_LAUNCHER_NO_SELF_UPDATE='1',
                   HORIZON_TEST_RESOURCES=str((args.resources or lab.LOCAL / 'horizon-client' / lab.VERSION).resolve()),
                   HORIZON_EXPLOSION_CAUSES_OUTPUT=str(actual))
        run(['java', '-Xmx512M', '-cp', os.pathsep.join([str(classes), cp]),
             'dev.horizonlab.parity.VanillaExplosionCausesOracle', expected], 'java', cwd=scratch, env=env)
        if args.test_binary:
            command = [args.test_binary.resolve(), FIXTURE, '--exact', '--ignored', '--nocapture']
            cwd = scratch
        else:
            command = ['cargo', 'test', '-p', 'horizon-client', '--lib', FIXTURE,
                       '--', '--exact', '--ignored', '--nocapture']
            cwd = lab.ROOT
        run(command, 'rust', cwd=cwd, env=env)
    want, got = expected.read_text().splitlines(), actual.read_text().splitlines()
    if want != got:
        raise AssertionError('\n'.join(difflib.unified_diff(want, got, fromfile='vanilla', tofile='horizon')))
    assert len(got) == 32, 'all player/mob/ownerless and persistence cases must run'
    report.write_text(json.dumps({'passed': True, 'cases': len(got)}, indent=2) + '\n')
    print('explosion causes: 32 actual crystal/cart cases match vanilla; server kill credit checked')


if __name__ == '__main__':
    main()
