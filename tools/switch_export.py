#!/usr/bin/env python3
"""Copies Horizon's prepared resources and its NRO onto a Switch SD card (docs/switch.md).

    python tools/switch_export.py E:\\
    python tools/switch_export.py /media/sd --resources .local/horizon-client/26.3 --nro horizon.nro
    python tools/switch_export.py E:\\ --check        # what it would copy, nothing written

Run `horizon-prepare` (or the launcher) on the PC first: it builds the resources from your own
Minecraft install, which needs a JDK and cannot run on the Switch. The SD card's layout:

    switch/horizon/horizon.nro        start it from the Homebrew Menu, holding R (full memory)
    switch/horizon/resources/         the prepared resources (this script)
    switch/horizon/                   also where the game keeps worlds, options.txt, logs, log.txt
    .mesa/                            NVK's shader cache (made on the first launch)

Saves and settings already on the card are never touched, and running the script again copies only
what changed. Python 3.8 or later, nothing else.
"""
import argparse
import json
import os
import shutil
import sys
from pathlib import Path

TARGET = Path('switch', 'horizon')
RESOURCES = 'resources'
MANIFEST = 'horizon-resources.json'
NRO_NAME = 'horizon.nro'
DEFAULT_RESOURCES = Path('.local', 'horizon-client', '26.3')
DEFAULT_NRO = Path('target', 'aarch64-nintendo-horizon', 'release', NRO_NAME)
# The manifest's `schema` the Switch build reads (horizon-prepare SCHEMA).
SCHEMA = 7
# A FAT32 file holds at most 4 GiB - 1 byte.
FAT32_LIMIT = 4 * 1024 ** 3 - 1
# Free space to leave on the card for worlds, shader cache and logs.
HEADROOM = 512 * 1024 ** 2


def fail(message):
    sys.exit(f'error: {message}')


def read_manifest(resources):
    try:
        return json.loads((resources / MANIFEST).read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return None


def plan(resources, nro, game):
    """[(source, destination)] for every file, the NRO first."""
    files = []
    if nro is not None:
        files.append((nro, game / NRO_NAME))
    for directory, _, names in os.walk(resources):
        for name in sorted(names):
            source = Path(directory, name)
            files.append((source, game / RESOURCES / source.relative_to(resources)))
    return files


def unchanged(source, destination):
    try:
        a, b = source.stat(), destination.stat()
    except OSError:
        return False
    # FAT stores modification times to two seconds.
    return a.st_size == b.st_size and abs(a.st_mtime - b.st_mtime) <= 2


def too_big(files, limit=FAT32_LIMIT):
    return [source for source, _ in files if source.stat().st_size > limit]


def case_clashes(files):
    """Pairs of destinations that differ only by case: FAT is case-insensitive."""
    seen, clashes = {}, []
    for _, destination in files:
        name = str(destination)
        key = name.lower()
        if key in seen and seen[key] != name:
            clashes.append((seen[key], name))
        seen[key] = name
    return clashes


def export(resources, sd, nro, check_only=False, out=print):
    resources, sd = Path(resources), Path(sd)
    manifest = read_manifest(resources)
    if manifest is None:
        fail(f'{resources / MANIFEST} is missing or unreadable. Prepare the resources on the PC '
             'first (the launcher, or `cargo run -p horizon-prepare`).')
    if manifest.get('schema') != SCHEMA:
        out(f'warning: the resources have schema {manifest.get("schema")}, this Switch build reads '
            f'{SCHEMA}; prepare them again with the same version of Horizon as the NRO.')
    if not sd.is_dir():
        fail(f'{sd} is not a folder (the SD card, e.g. E:\\ on Windows).')
    if nro is not None and not Path(nro).is_file():
        fail(f'{nro} is missing: build it (packaging/switch/build.sh nro) or pass --nro.')
    game = sd / TARGET
    files = plan(resources, Path(nro) if nro else None, game)
    big = too_big(files)
    if big:
        fail('FAT32 cannot hold a file of 4 GiB or more: ' + ', '.join(str(p) for p in big[:5]) +
             '. Format the card as exFAT (Atmosphere reads it), or split the file.')
    clashes = case_clashes(files)
    if clashes:
        fail('FAT is case-insensitive, and these differ only by case: ' +
             ', '.join(f'{a} / {b}' for a, b in clashes[:5]))
    todo = [(s, d) for s, d in files if not unchanged(s, d)]
    size = sum(s.stat().st_size for s, _ in todo)
    out(f'{len(files)} files, {len(todo)} to copy ({size / 1024 ** 2:.0f} MiB) to {game}')
    free = shutil.disk_usage(sd).free
    if size + HEADROOM > free:
        fail(f'not enough room on {sd}: {size / 1024 ** 2:.0f} MiB to copy, '
             f'{free / 1024 ** 2:.0f} MiB free (and {HEADROOM // 1024 ** 2} MiB kept for worlds).')
    if check_only:
        return todo
    for i, (source, destination) in enumerate(todo, 1):
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        if i % 500 == 0:
            out(f'  {i}/{len(todo)}')
    out('done. Start the Homebrew Menu with full memory: hold R while starting an installed game.')
    return todo


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('sd', help="the SD card's root (or any folder standing for it)")
    parser.add_argument('--resources', type=Path, default=DEFAULT_RESOURCES,
                        help=f'the prepared resources (default {DEFAULT_RESOURCES})')
    parser.add_argument('--nro', type=Path, default=None,
                        help=f'the NRO (default {DEFAULT_NRO} if it exists; none: resources only)')
    parser.add_argument('--check', action='store_true', help='say what would be copied; write nothing')
    args = parser.parse_args()
    nro = args.nro or (DEFAULT_NRO if DEFAULT_NRO.is_file() else None)
    if nro is None:
        print(f'note: no NRO ({DEFAULT_NRO}); copying the resources only')
    export(args.resources, args.sd, nro, args.check)


if __name__ == '__main__':
    main()
