#!/usr/bin/env python3
"""Compare the order natural spawning walks its chunks in with vanilla's: a real DistanceManager's
naturalSpawnChunkCounter (FixedPlayerDistanceChunkTracker over its Long2ByteOpenHashMap) fed the
addPlayer/removePlayer calls ChunkMap makes as players join, move and leave, against Horizon's
SpawnChunkTracker fed the same players. Every spawning pass compares the counter's size, its
key set in iteration order, that list after Util.shuffle and the random's next draw."""
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab
    import oracle_harness as harness


def join(id, x, z, y=4):
    return ['join', id, x, y, z]


def move(id, x, z, y=4):
    return ['move', id, x, y, z]


def leave(id):
    return ['leave', id]


class Seeds:
    def __init__(self):
        self.next = 0x5DEECE66D

    def __call__(self):
        self.next = (self.next * 6364136223846793005 + 1442695040888963407) % (1 << 64)
        return self.next - (1 << 64) if self.next >= 1 << 63 else self.next


PASS_SEEDS = Seeds()


def pass_():
    return ['pass', PASS_SEEDS()]


def walk(id, path, every=1):
    """Moves along `path`, a spawning pass after every `every` of them."""
    ops = []
    for i, (x, z) in enumerate(path, 1):
        ops.append(move(id, x, z))
        if i % every == 0:
            ops.append(pass_())
    return ops


def scripted():
    line = [(x, 0) for x in range(1, 21)]
    yield 10, [join(1, 0, 0), pass_()]  # the null key, chunk (0, 0)
    yield 10, [join(1, 5, -3), pass_(), move(1, 6, -3), pass_()]
    yield 10, [join(1, -1875000, 1874990), pass_(), move(1, -1874999, 1874991), pass_()]
    yield 10, [join(1, 0, 0), pass_(), *walk(1, line)]
    yield 2, [join(1, 0, 0), *walk(1, line, every=5)]
    yield 32, [join(1, -3, -3), pass_(), *walk(1, [(-3 - i, -3 - i) for i in range(1, 13)])]
    # Back and forth over one border: the edge columns leave and come back.
    yield 10, [join(1, 7, 7), pass_(), *walk(1, [(8, 7), (7, 7)] * 6)]
    # Moves between sections of one chunk (ChunkMap.move removes and re-adds it).
    yield 10, [join(1, 2, 2), pass_(), move(1, 2, 2, 5), pass_(), move(1, 2, 2, -4), move(1, 3, 2, -4), pass_()]
    # A section move inside one chunk while the queue still holds that chunk (joined or moved this
    # tick) sends it to the back of its queue, behind the chunks queued after it.
    yield 10, [join(1, 0, 0), join(2, 1, 0), move(1, 0, 0, 5), pass_(), move(2, 2, 0), move(1, 1, 1),
               move(2, 2, 0, 3), pass_(), move(1, 5, 5), move(2, 6, 5), move(1, 5, 5, 0), pass_()]
    # Far teleports: the table empties down to its minimum and grows back.
    yield 10, [join(1, 0, 0), pass_(), move(1, 1000, -1000), pass_(), move(1, 0, 0), pass_(),
               move(1, 40, 0), pass_(), move(1, 9, 9), pass_()]
    # Two players in one chunk, then each leaving.
    yield 10, [join(1, 4, 4), join(2, 4, 4), pass_(), leave(1), pass_(), leave(2), pass_(),
               pass_(), join(3, 0, 0), pass_()]
    # Overlapping squares moving apart and together.
    yield 10, [join(1, 0, 0), join(2, 10, 0), pass_(), *walk(2, [(10 + i, 0) for i in range(1, 9)], 2),
               *walk(1, [(i, 0) for i in range(1, 17)], 4), leave(1), pass_()]
    # Everyone leaves, the level stays empty, someone joins far away.
    yield 10, [join(1, 0, 0), join(2, 3, -5), pass_(), leave(2), leave(1), pass_(), pass_(),
               join(4, 300, 300), pass_()]
    # Several players joining in one tick, then moving in one tick.
    yield 10, [join(1, 0, 0), join(2, 1, 1), join(3, -16, 4), join(4, 30, 30), pass_(),
               move(1, 1, 0), move(2, 2, 2), move(3, -17, 4), move(4, 30, 31), pass_()]


def random_cases(rng):
    """Random walks: players join, step over chunk borders (sometimes several between two
    passes), climb between sections, teleport and leave."""
    for players in (2, 3, 5, 8):
        for spread, steps in ((6, 70), (20, 70), (60, 40)):
            ops, at, next_id = [], {}, 1
            for _ in range(steps):
                roll = rng.random()
                if len(at) < players and (not at or roll < 0.15):
                    x, z = rng.randint(-spread, spread), rng.randint(-spread, spread)
                    at[next_id] = (x, z, 4)
                    ops.append(join(next_id, x, z))
                    next_id += 1
                elif at and roll < 0.2:
                    ops.append(leave(gone := rng.choice(list(at))))
                    del at[gone]
                elif at:
                    id = rng.choice(list(at))
                    x, z, y = at[id]
                    kind = rng.random()
                    if kind < 0.1:
                        y += rng.choice((-1, 1))
                    elif kind < 0.15:
                        x, z = x + rng.randint(-400, 400), z + rng.randint(-400, 400)
                    else:
                        x, z = x + rng.randint(-2, 2), z + rng.randint(-2, 2)
                    at[id] = (x, z, y)
                    ops.append(move(id, x, z, y))
                if rng.random() < 0.45:
                    ops.append(pass_())
            ops.append(pass_())
            yield rng.choice((2, 10, 32)), ops


CASES = [*scripted(), *random_cases(random.Random(0x5350434F))]
# A "== n" line per case, two lines per pass.
LINES = len(CASES) + 2 * sum(op[0] == 'pass' for _, ops in CASES for op in ops)


def make_input(build):
    path = build / 'inputs.jsonl'
    path.write_text(''.join(json.dumps({'sim': sim, 'ops': ops}) + '\n' for sim, ops in CASES),
                    encoding='utf-8')
    return path, len(CASES)


def main():
    harness.differential(
        'spawn-chunk-order-parity', 'VanillaSpawnChunkOrderOracle', 'spawn_chunk_order_fixture', make_input,
        'A real DistanceManager (TicketStorage, its trackers and executors run inline, no ChunkMap or '
        'level) given ChunkMap.updatePlayerStatus/move\'s addPlayer and removePlayer calls for allocated '
        'ServerPlayers, at several simulation distances; each spawning pass reads getNaturalSpawnChunkCount '
        'and getSpawnCandidateChunks (keySet order) and runs Util.shuffle with a LegacyRandomSource. '
        'Excludes the 128-block and ticking-chunk filters of collectSpawningChunks and spectators '
        'with spectatorsGenerateChunks off.',
        sources=[lab.ROOT / 'crates/horizon-client/src/spawn_chunk_order.rs',
                 lab.ROOT / 'crates/horizon-client-core/src/fastutil_order.rs'],
        expected_lines=LINES)


if __name__ == '__main__':
    main()
