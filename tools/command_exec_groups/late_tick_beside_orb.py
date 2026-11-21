"""`late tick beside orb`: fresh primed TNT and falling blocks tick when an orb joined after them (#17269)."""

GROUP = 'late tick beside orb'


def cases(cmd, setup):
    """`ServerLevel.tick` ticks every entity in its tick list once a tick, in join order. Three primed TNT
    and a falling block are summoned, then an experience orb; after one tick each fuse is down by one and
    the falling block's `Time` up by one. A control area holds the same without the orb."""
    x0, y0, z0 = 6240, 250, 6240
    # Entities tick once every chunk within 2 columns is loaded (ChunkMap.prepareEntityTickingChunk).
    chunk = [[(x0 >> 4) + dx, (z0 >> 4) + dz] for dx in range(-2, 3) for dz in range(-2, 3)]
    ops = [cmd(f'/forceload add {x0} {z0}', False), {'op': 'wait_chunks', 'chunks': chunk},
           cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} stone', expect_result=256)]
    uuids = []
    for area, orb in ((0, True), (8, False)):
        for n in range(1, 4):
            uuids.append((f'{17269:08x}-0000-{area:04x}-0000-{n:04x}00000001', 'fuse'))
            ops.append(cmd(f'/summon tnt {x0 + 3 * n}.5 {y0} {z0 + area + 2}.5 '
                           f'{{UUID:[I;17269,{area},{n},1],fuse:400}}', expect_result=1))
        uuids.append((f'{17269:08x}-0000-{area:04x}-0000-000400000001', 'Time'))
        ops.append(cmd(f'/summon falling_block {x0 + 12}.5 {y0} {z0 + area + 2}.5 '
                       f'{{UUID:[I;17269,{area},4,1],BlockState:{{Name:"minecraft:sand"}},Time:1,NoGravity:1b}}',
                       expect_result=1))
        if orb:
            ops.append(cmd(f'/summon experience_orb {x0 + 12}.5 {y0} {z0 + area + 5}.5 {{Value:3}}',
                           expect_result=1))

    def reads():
        for uuid, key in uuids:
            ops.append(cmd(f'/data get entity {uuid} {key}'))

    reads()
    ops.append({'op': 'tick', 'n': 1})
    reads()
    ops.append({'op': 'tick', 'n': 1})
    reads()

    # A primed TNT that joined before a wind charge has ticked when the charge bursts, so the burst measures it
    # where its own move left it. `WindCharge.onHitBlock` bursts 0.25 off the wall, here at x0 + 1.25, and reaches
    # 2.4 blocks. (TNT east of the burst, its motion): leaves the reach, stays inside it, comes into it.
    y1 = y0 + 8
    lanes = [(2.3, 0.4), (1.0, 0.2), (2.7, -0.4)]
    ops.append(cmd('/tick freeze', False))
    tnt = []
    for n, (east, motion) in enumerate(lanes, 1):
        z1 = z0 + 1 + (n - 1) * 6
        tnt.append(f'{17269:08x}-0000-0010-0000-{n:04x}00000001')
        ops.append(cmd(f'/setblock {x0} {y1} {z1} stone', expect_result=1))
        ops.append(cmd(f'/summon tnt {x0 + 1.25 + east} {y1}.5 {z1}.5 '
                       f'{{UUID:[I;17269,16,{n},1],fuse:400,NoGravity:1b,Motion:[{motion}d,0d,0d]}}', expect_result=1))
        ops.append(cmd(f'/summon wind_charge {x0 + 1.5} {y1}.5 {z1}.5 {{Motion:[-0.5d,0d,0d]}}', expect_result=1))
    for _ in range(2):
        ops += [cmd('/tick unfreeze', False), {'op': 'tick', 'n': 1}, cmd('/tick freeze', False)]
        for uuid in tnt:
            for key in ('Pos', 'Motion'):
                ops.append(cmd(f'/data get entity {uuid} {key}'))
    ops.append(cmd('/tick unfreeze', False))
    return ops + setup(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=24,dz=16]',
                       f'/fill {x0} {y1} {z0} {x0} {y1} {z0 + 15} air',
                       f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} air',
                       f'/forceload remove {x0} {z0}')
