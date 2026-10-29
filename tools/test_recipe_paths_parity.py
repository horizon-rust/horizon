#!/usr/bin/env python3
"""Compare nested recipe IDs with vanilla's PathPackResources and FileToIdConverter."""
import json
import os
from pathlib import Path
import tempfile
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib


def main():
    work = lib.work_dir('recipe-paths-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('VanillaRecipePathsOracle')
    with tempfile.TemporaryDirectory(prefix='recipes-', dir=work) as scratch:
        base = Path(scratch)
        roots = []
        for case in range(12):
            root = base / str(case)
            recipe = root / 'recipe'
            recipe.mkdir(parents=True)
            roots.append(str(root))
            for depth in range(case + 1):
                parent = recipe.joinpath(*(['nested'] * depth))
                parent.mkdir(parents=True, exist_ok=True)
                for name in ('same.json', 'dot.name.json', 'under_score-2.json',
                             'Upper.json', 'space name.json', 'unicode_é.json',
                             'same.JSON', 'same.json.txt'):
                    (parent / name).write_text('{}', encoding='utf-8')
            (recipe / 'folder.json').mkdir()
            (recipe / 'folder.json' / 'child.json').write_text('{}', encoding='utf-8')
            if os.name == 'posix':
                (recipe / 'cycle').symlink_to(recipe, target_is_directory=True)
                (recipe / 'alias.json').symlink_to(recipe / 'same.json')
        roots.append(str(base / 'missing'))
        (base / 'not-directory').mkdir()
        (base / 'not-directory' / 'recipe').write_text('{}', encoding='utf-8')
        roots.append(str(base / 'not-directory'))
        if os.name == 'posix':
            linked = base / 'linked-root'
            linked.mkdir()
            (linked / 'recipe').symlink_to(base / '0' / 'recipe', target_is_directory=True)
            roots.append(str(linked))
        inputs = work / 'cases.json'
        inputs.write_text(json.dumps(roots), encoding='utf-8')
        native_path, rust_path = work / 'native.json', work / 'rust.json'
        native_path.unlink(missing_ok=True)
        rust_path.unlink(missing_ok=True)
        vanilla.run('VanillaRecipePathsOracle', inputs, native_path)
        lib.cargo_example('horizon-client', 'recipe_paths_fixture', inputs, rust_path)
        native = json.loads(native_path.read_text(encoding='utf-8'))
        rust = json.loads(rust_path.read_text(encoding='utf-8'))
        if len(native) != len(roots) or len(rust) != len(roots):
            raise SystemExit('Incomplete recipe path observations')
        if not sum(map(len, native)):
            raise SystemExit('No recipe paths compared')
        bad = [i for i, (a, b) in enumerate(zip(native, rust)) if a != b]
        for i in bad[:3]:
            print(f'case {i}: vanilla={native[i]!r} rust={rust[i]!r}')
        print(f'{len(roots) - len(bad)}/{len(roots)} trees match, {sum(map(len, native))} recipe IDs')
        raise SystemExit(bool(bad))


if __name__ == '__main__':
    main()
