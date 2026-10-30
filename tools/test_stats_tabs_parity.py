#!/usr/bin/env python3
"""Compare statistics tab availability and tooltips with the real vanilla method.

No window or prepared resources are needed. With --minecraft-home, use and
authenticate the pinned version and libraries from an official launcher install.
Otherwise use the normal parity lab's authenticated artifacts.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

try:
    from . import interaction_lab as lib, lab, test_model_baking_parity as baking, test_rust_parity as base
except ImportError:
    import interaction_lab as lib
    import lab
    import test_model_baking_parity as baking
    import test_rust_parity as base


def client_dependencies(minecraft_home):
    if minecraft_home is None:
        lock, jars, _ = base.verify_artifacts()
        return lock, [jars['client'], *baking.client_libraries()]
    lock = json.loads(lab.LOCK.read_text())
    version = lab.VERSION
    directory = minecraft_home / 'versions' / version
    manifest_path = directory / (version + '.json')
    if not lab.manifest_accepted(lock, lab.digest(manifest_path, 'sha1')):
        raise ValueError('Installed manifest does not match research-lock.json')
    manifest = json.loads(manifest_path.read_text())
    jar = directory / (version + '.jar')
    if lab.digest(jar, 'sha1') != manifest['downloads']['client']['sha1']:
        raise ValueError('Installed client checksum changed')
    dependencies = [jar]
    for artifact in lab.client_library_artifacts(manifest):
        relative = Path(artifact['path'])
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('Unsafe library path')
        library = minecraft_home / 'libraries' / relative
        if lab.digest(library, 'sha1') != artifact['sha1']:
            raise ValueError('Installed library checksum changed: ' + str(relative))
        dependencies.append(library)
    return lock, dependencies


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--minecraft-home', type=Path)
    args = parser.parse_args()
    lock, dependencies = client_dependencies(args.minecraft_home)
    work = lib.work_dir('stats-tabs-parity')
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    # Reuse each Java widget: empty -> populated must clear an old tooltip,
    # and populated -> empty must disable it again, independently for both tabs.
    cases = [{'screen': 'stats', 'index': index, 'entries': entries}
             for index in (1, 2) for entries in (0, 1, 0, 2, 100, 0)]
    inputs = work / 'cases.jsonl'
    native_path, rust_path = work / 'native.jsonl', work / 'rust.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in cases))
    deps = os.pathsep.join(map(str, dependencies))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn',
                    '-cp', deps, '-d', str(classes), str(lib.PARITY / 'VanillaStatsTabsOracle.java')], check=True)
    subprocess.run(['java', '-cp', str(classes) + os.pathsep + deps,
                    'dev.horizonlab.parity.VanillaStatsTabsOracle', str(inputs), str(native_path)],
                   check=True, cwd=work)
    lib.cargo_example('horizon-client', 'screen_layout_fixture', lib.RESOURCES, inputs, rust_path)
    native = [json.loads(line) for line in native_path.read_text().splitlines()]
    rust = [json.loads(line) for line in rust_path.read_text().splitlines()]
    assert len(native) == len(rust) == len(cases)
    mismatches = [(case, a, b) for case, a, b in zip(cases, native, rust) if a != b]
    for case, a, b in mismatches:
        print(case, 'vanilla:', a, 'Rust:', b)
    print(f'{len(cases) - len(mismatches)}/{len(cases)} statistics tab states match')
    raise SystemExit(bool(mismatches))


if __name__ == '__main__':
    main()
