#!/usr/bin/env python3
"""Compare both ModelPart draw paths with vanilla's rendered boat floor.

The floor and a bottom trapdoor have the same height. Even a tiny unintended
tilt makes one end fall below the trapdoor (#5950). All position components must
match exactly, rather than using the entity animation harness's checksum tolerance.
The boat root transform and ModelPart.translateAndRotate both follow vanilla.
"""
import json
import os
import struct
import subprocess

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking


def main():
    lock, jars, _ = base.verify_artifacts()
    work = lab.LOCAL / 'boat-floor-parity'
    work.mkdir(parents=True, exist_ok=True)
    report = work / 'report.json'
    report.unlink(missing_ok=True)
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBoatFloorOracle.java'

    def run(command, name):
        with (work / f'{name}.log').open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=1800)
        if result.returncode:
            raise RuntimeError(f'{name}: {(work / f"{name}.log").read_text()[-4000:]}')

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies,
         '-d', classes, oracle], 'compile-java')
    run(['java', '-cp', str(classes) + os.pathsep + dependencies,
         'dev.horizonlab.parity.VanillaBoatFloorOracle', work / 'vanilla.json'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example',
         'client_probe', '--', 'boat_floor_fixture', lab.RESOURCES, work / 'vanilla.json',
         work / 'rust.json'], 'rust')
    vanilla = json.loads((work / 'vanilla.json').read_text())
    rust = json.loads((work / 'rust.json').read_text())
    assert len(vanilla) == len(rust) == 36
    failures = []
    for case, actual in zip(vanilla, rust):
        for path in ('animated', 'direct'):
            assert len(case['vertices']) == len(actual[path]) == 24
            for index, (expected, got) in enumerate(zip(case['vertices'], actual[path])):
                # Round JSON decimals back to their original float32 before comparing.
                if any(struct.pack('f', a) != struct.pack('f', b) for a, b in zip(expected, got)):
                    failures.append((case['yaw'], case['camera_y'], path, index, expected, got))
                assert all(abs(a - b) < 5e-7 for a, b in zip(expected, got)), (case, path, index, got)
    assert not failures, f'{len(failures)} floor vertices differ from vanilla: {failures[:3]}'
    lab.write_json(report, dict(minecraft=lock['minecraft']['version'], cases=len(vanilla),
                               vertices=36 * 24 * 2, scope=__doc__))
    print('boat floor: 36 transforms, 1728 vertices; both draw paths match vanilla positions exactly')


if __name__ == '__main__':
    main()
