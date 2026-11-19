//! Water left alone by a mod that took over its tick, and water that spreads as vanilla.
use super::*;

#[test]
fn water_spreads_on_a_floor() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..60 {
        w.tick();
    }
    for d in 1..=7 {
        let pos = BlockPos::new(d, 1, 0);
        assert_eq!(
            w.prop(pos, "level"),
            d.to_string(),
            "flow at distance {d}: {}",
            w.name(pos)
        );
    }
    assert_eq!(w.get(BlockPos::new(8, 1, 0)), w.b().air());
    // Diagonals follow Manhattan distance.
    assert_eq!(w.prop(BlockPos::new(2, 1, 2), "level"), "4");
}

#[test]
fn refused_fluid_tick_leaves_the_water_alone() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let source = BlockPos::new(0, 1, 0);
    w.level.guarded.insert(source);
    let water = w.id("water", &[]);
    w.set(source, water);
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(w.get(source), water);
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    // Allowed again, its tick is gone: nothing was left scheduled to run it.
    w.level.guarded.clear();
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
}

#[test]
fn refused_non_source_tick_keeps_exact_state_and_schedules_nothing() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let flowing = w.id("water", &[("level", "3")]);
    w.put(pos, flowing);
    w.level.guarded.insert(pos);
    assert_eq!(w.ticks.fluids.count(), 0);
    w.b().scheduled_fluid_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.get(pos), flowing);
    assert_eq!(w.ticks.fluids.count(), 0);
    assert_eq!(w.ticks.blocks.count(), 0);
}

/// An off mod's water (#18519): its fluid tick hook is still asked, and the water keeps still.
#[test]
fn a_suppressed_fluid_tick_still_asks_its_hook() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let flowing = w.id("water", &[("level", "3")]);
    w.put(pos, flowing);
    w.level.suppressed.insert(flowing);
    w.b().scheduled_fluid_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.level.fluid_asks.get(), 1, "the hook is asked");
    assert_eq!(w.get(pos), flowing, "a lone flowing water would dry");
    assert_eq!(w.ticks.fluids.count(), 0);
}

/// A due tick an off mod's block owes goes back in the queue 20 ticks on with its own priority;
/// a fluid tick its hook cancels is gone, as for any block (#18519).
#[test]
fn a_suppressed_due_tick_waits_with_its_own_priority() {
    use horizon_core::ticks::TickPriority;
    let mut w = world!();
    let at = BlockPos::new(0, 5, 0);
    let sand = w.id("sand", &[]);
    w.put(at, sand);
    w.level.suppressed.insert(sand);
    let block = w.b().block(sand);
    w.ticks
        .schedule_block(0, at, block, 1, TickPriority::VeryHigh);
    w.tick();
    assert_eq!(w.get(at), sand, "it doesn't fall");
    let queued = w.ticks.blocks.chunk(ChunkPos::new(0, 0)).unwrap().queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        (queued[0].trigger_tick, queued[0].priority),
        (21, TickPriority::VeryHigh)
    );
    assert_eq!(w.level.requeued, [at]);

    let pool = BlockPos::new(4, 1, 0);
    let flowing = w.id("water", &[("level", "3")]);
    let kind = w.b().fluid(flowing).kind();
    w.put(pool, flowing);
    w.level.suppressed.insert(flowing);
    w.level.guarded.insert(pool);
    let time = w.level.time;
    w.ticks.schedule_fluid(time, pool, kind, 1);
    w.tick();
    assert_eq!(w.level.fluid_asks.get(), 1);
    assert_eq!(w.ticks.fluids.count(), 0, "the hook's cancel consumes it");
    w.level.guarded.clear();
    let time = w.level.time;
    w.ticks.schedule_fluid(time, pool, kind, 1);
    w.tick();
    assert_eq!(w.get(pool), flowing);
    assert_eq!(w.ticks.fluids.count(), 1, "owed, it waits");
    assert_eq!(w.level.requeued, [at, pool]);
}
