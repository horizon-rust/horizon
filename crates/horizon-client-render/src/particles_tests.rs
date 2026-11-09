use super::*;

#[test]
fn legacy_spell_providers_use_velocity_rgb_and_their_original_alpha_rules() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    let velocity = [0.25, 0.5, 0.75];
    for scoped in [false, true] {
        engine.set_scope_eye(scoped.then_some([0.0; 3]));
        for ambient in [false, true] {
            let mut options =
                ParticleOptions::colored(ParticleType::EntityEffect, [1.0, 0.0, 0.0, 0.8]);
            options.legacy_entity_effect = Some(ambient);
            let particle = engine.make(&options, [0.0; 3], velocity, 7).unwrap();
            let alpha = if ambient {
                0.15
            } else if scoped {
                0.0
            } else {
                1.0
            };
            assert_eq!(particle.rgba, [0.25, 0.5, 0.75, alpha]);
            assert_eq!(
                particle.behaviour,
                Behaviour::Spell {
                    original_alpha: 1.0
                }
            );
        }
        let options = ParticleOptions::colored(ParticleType::EntityEffect, [1.0, 0.0, 0.0, 0.8]);
        let current = engine.make(&options, [0.0; 3], velocity, 7).unwrap();
        assert_eq!(current.rgba, options.color);
        assert_eq!(
            current.behaviour,
            Behaviour::Spell {
                original_alpha: 0.8
            }
        );
    }
}

#[test]
fn particle_light_cache_reuses_exact_blocks_and_fails_open_on_collisions() {
    let reads = std::cell::Cell::new(0);
    let light_at = |block: [i32; 3]| {
        reads.set(reads.get() + 1);
        block[0] as u8
    };
    let mut cache = ParticleLightCache::default();
    let first = [0, 0, 0];
    let colliding = [64, 0, 0];

    assert_eq!(cache.get(first, &light_at), 0);
    assert_eq!(cache.get(first, &light_at), 0);
    assert_eq!(reads.get(), 1);
    assert_eq!(cache.get(colliding, &light_at), 64);
    assert_eq!(cache.get(first, &light_at), 0);
    assert_eq!(reads.get(), 3);
}

#[test]
fn subpixel_culling_uses_the_frame_projection_and_fails_open() {
    assert!(below_half_pixel(0.01, 41.0_f64.powi(2), 1_000.0));
    assert!(!below_half_pixel(0.01, 40.0_f64.powi(2), 1_000.0));
    assert!(!below_half_pixel(0.01, 41.0_f64.powi(2), 0.0));
    assert!(!below_half_pixel(0.01, f64::NAN, 1_000.0));
    assert!(!below_half_pixel(f64::NAN, 41.0_f64.powi(2), 1_000.0));

    for distance in [0.25_f32, 1.0, 8.0, 40.0, 128.0] {
        for radius in [0.01, 0.05, 0.2] {
            let center = FVec3::new(distance, distance * 0.3, -distance * 0.8);
            let old = 2.0 * radius * 900.0 / f64::from(center.length()) < 0.5;
            assert_eq!(
                below_half_pixel(radius, f64::from(center.length_squared()), 900.0),
                old
            );
        }
    }
}

#[test]
fn disabled_culled_render_matches_the_regular_particle_path() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    let sprite = "test:smoke".to_owned();
    engine.atlas.insert(
        sprite.clone(),
        (
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16,
            },
            false,
        ),
    );
    let mut random = LegacyRandom::new(7);
    let mut particle =
        Particle::moving(&mut random, ParticleType::Smoke, [0.0, 0.0, -3.0], [0.0; 3]);
    particle.sprite = Sprite::Named(sprite);
    engine.particles.push(particle);

    let light_at = |_: [i32; 3]| 0xf0;
    let mut regular = Vec::new();
    engine.render(DVec3::ZERO, 0.0, 0.0, 0.5, &light_at, &mut regular);
    let mut disabled = Vec::new();
    engine.render_culled(
        DVec3::ZERO,
        0.0,
        0.0,
        0.5,
        900.0,
        false,
        None,
        |_, _| panic!("disabled culling must not query particle visibility"),
        &light_at,
        &mut disabled,
    );

    let attributes = |vertices: &[Vertex]| {
        vertices
            .iter()
            .map(|vertex| {
                (
                    vertex.pos,
                    vertex.uv,
                    vertex.layer,
                    vertex.color,
                    vertex.light,
                    vertex.normal,
                    vertex.glint_uv,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(attributes(&disabled), attributes(&regular));
}

#[test]
fn a_mod_particles_json_becomes_its_sprite_set() {
    let dir = std::env::temp_dir().join(format!("horizon-mod-particles-{}", std::process::id()));
    let particles = dir.join("assets/mymod/particles");
    std::fs::create_dir_all(&particles).unwrap();
    std::fs::write(
        particles.join("spark.json"),
        br#"{"textures":["mymod:spark/1","mymod:spark/2"]}"#,
    )
    .unwrap();
    assert!(particles.join("spark.json").exists());
    let mut engine = ParticleEngine::new(&dir, &HashMap::new(), &HashSet::new());
    let (set, _) = *engine
        .mod_sets
        .get("mymod:spark")
        .expect("the set is loaded");
    assert_eq!(engine.set_sprites[set as usize].len(), 2);
    // The named option spawns a billboard from that set, not a vanilla type.
    let made = engine
        .make(
            &ParticleOptions::named("mymod:spark"),
            [0.0; 3],
            [0.0; 3],
            7,
        )
        .expect("a mod particle is made");
    assert_eq!(made.set, Some(set));
    // A name no pack knows draws nothing, like a missing vanilla sprite set.
    assert!(
        engine
            .make(
                &ParticleOptions::named("mymod:missing"),
                [0.0; 3],
                [0.0; 3],
                7
            )
            .is_none()
    );
    std::fs::write(
        particles.join("spark.json"),
        br#"{"textures":["mymod:spark/1"],"horizon:behaviour":{
                "velocity":[0,0.5,0],"gravity":-1,"friction":0.5,
                "lifetime":4,"size":[0.25,0.5],"alpha":[0.75,0]
            }}"#,
    )
    .unwrap();
    engine.reload(&dir, &HashMap::new(), &HashSet::new());
    let custom = engine
        .make(
            &ParticleOptions::named("mymod:spark"),
            [0.0; 3],
            [0.0; 3],
            7,
        )
        .unwrap();
    assert_eq!(custom.velocity, [0.0, 0.5, 0.0]);
    assert_eq!(custom.gravity, -1.0);
    assert_eq!(custom.friction, 0.5);
    assert_eq!(custom.lifetime, 4);
    assert_eq!(custom.size, 0.25);
    assert_eq!(custom.rgba[3], 0.75);
    for suffix in [r#", "horizon:behaviour": {"lifetime":0}"#, ""] {
        std::fs::write(
            particles.join("spark.json"),
            format!(r#"{{"textures":["mymod:spark/1","mymod:spark/2"]{suffix}}}"#,),
        )
        .unwrap();
        engine.reload(&dir, &HashMap::new(), &HashSet::new());
        let fallback = engine
            .make(
                &ParticleOptions::named("mymod:spark"),
                [0.0; 3],
                [0.0; 3],
                7,
            )
            .unwrap();
        assert_eq!(fallback.velocity, made.velocity);
        assert_eq!(fallback.gravity, made.gravity);
        assert_eq!(fallback.friction, made.friction);
        assert_eq!(fallback.size, made.size);
        assert_eq!(fallback.rgba, made.rgba);
        assert_eq!(fallback.lifetime, made.lifetime);
        assert_eq!(fallback.behaviour, made.behaviour);
    }
    assert_eq!(custom.lifetime, 4);
    assert_eq!(custom.size, 0.25);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mod_behaviour_validates_declarations() {
    use serde_json::json;
    for invalid in [
        json!({"velocity": [0, 0]}),
        json!({"velocity": [17, 0, 0]}),
        json!({"gravity": 101}),
        json!({"gravity": 1e100}),
        json!({"friction": -0.1}),
        json!({"friction": 1.1}),
        json!({"lifetime": 0}),
        json!({"lifetime": 12001}),
        json!({"lifetime": 2.5}),
        json!({"size": [-1, 1]}),
        json!({"size": [0, 17]}),
        json!({"alpha": [0, 1.1]}),
        json!({"alpha": [-0.1, 1]}),
        json!({"fricton": 0.9}),
        json!(null),
    ] {
        assert!(ModBehaviour::parse(&invalid).is_none(), "{invalid}");
    }
    assert!(
        ModBehaviour::parse(&json!({
            "velocity": [-16, 0, 16], "gravity": -100, "friction": 0,
            "lifetime": 12000, "size": [0, 16], "alpha": [0, 1]
        }))
        .is_some()
    );
    let mut particle = Particle::moving(
        &mut LegacyRandom::new(7),
        ParticleType::Smoke,
        [0.0; 3],
        [0.0; 3],
    );
    let original = particle.clone();
    ModBehaviour::parse(&json!({})).unwrap().apply(
        &mut particle,
        [0.0; 3],
        &mut LegacyRandom::new(1),
    );
    assert_eq!(particle.velocity, original.velocity);
    assert_eq!(particle.lifetime, original.lifetime);
    assert_eq!(particle.gravity, original.gravity);
    assert_eq!(particle.friction, original.friction);
    assert_eq!(particle.size, original.size);
    assert_eq!(particle.rgba, original.rgba);
    assert_eq!(particle.behaviour, original.behaviour);
}

#[test]
fn mod_velocity_adds_the_spawn_velocity_and_jitters_deterministically() {
    use serde_json::json;
    let mut base = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    ModBehaviour::parse(&json!({"velocity": [0.5, 0.0, 0.0]}))
        .unwrap()
        .apply(&mut base, [0.25, 1.0, -1.0], &mut LegacyRandom::new(1));
    assert_eq!(base.velocity, [0.75, 1.0, -1.0]);
    let jitter = ModBehaviour::parse(&json!({"velocity_jitter": [0.5, 0.0, 0.25]})).unwrap();
    let mut first = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    let mut second = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    jitter.apply(&mut first, [1.0, 2.0, 3.0], &mut LegacyRandom::new(9));
    jitter.apply(&mut second, [1.0, 2.0, 3.0], &mut LegacyRandom::new(9));
    assert_eq!(first.velocity, second.velocity);
    assert!((0.5..=1.5).contains(&first.velocity[0]));
    assert_eq!(first.velocity[1], 2.0);
    assert!((2.75..=3.25).contains(&first.velocity[2]));
    for invalid in [
        json!({"velocity_jitter": [-0.1, 0, 0]}),
        json!({"velocity_jitter": [17, 0, 0]}),
        json!({"frame_ticks": 2}),
        json!({"animation": "age", "frame_ticks": 2}),
        json!({"animation": "loop", "frame_ticks": 0}),
        json!({"animation": "loop", "frame_ticks": 201}),
        json!({"animation": "spin"}),
    ] {
        assert!(ModBehaviour::parse(&invalid).is_none(), "{invalid}");
    }
    assert!(ModBehaviour::parse(&json!({"animation": "loop", "frame_ticks": 3})).is_some());
}

#[test]
fn mod_animation_modes_pick_their_sprites() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.set_sprites.push(vec![10, 11, 12]);
    let mut looping = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    looping.set = Some(0);
    ModBehaviour::parse(&serde_json::json!({"animation": "loop", "frame_ticks": 2}))
        .unwrap()
        .apply(&mut looping, [0.0; 3], &mut LegacyRandom::new(1));
    let mut seen = Vec::new();
    for age in 0..8 {
        looping.age = age;
        engine.sprite_from_age(&mut looping);
        seen.push(match looping.sprite {
            Sprite::Index(i) => i,
            Sprite::Named(_) => u32::MAX,
        });
    }
    assert_eq!(seen, [10, 10, 11, 11, 12, 12, 10, 10]);
    let mut kept = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    kept.set = Some(0);
    ModBehaviour::parse(&serde_json::json!({"animation": "first"}))
        .unwrap()
        .apply(&mut kept, [0.0; 3], &mut LegacyRandom::new(1));
    assert_eq!(kept.set, None);
}

#[test]
fn mod_curves_clamp_and_interpolate_over_lifetime() {
    assert_eq!(mod_curve([0.0, 2.0], -1, 4), 0.0);
    assert_eq!(mod_curve([0.0, 2.0], 0, 4), 0.0);
    assert_eq!(mod_curve([0.0, 2.0], 1, 4), 0.5);
    assert_eq!(mod_curve([0.0, 2.0], 4, 4), 2.0);
    assert_eq!(mod_curve([0.0, 2.0], 5, 4), 2.0);
    assert_eq!(mod_curve([1.0, 0.0], 1, 4), 0.75);
    assert_eq!(mod_curve([0.5, 0.5], 2, 4), 0.5);
    assert_eq!(mod_curve([0.0, 1.0], 1, 1), 1.0);
}

#[test]
fn mod_animation_advances_in_the_tick_with_and_without_curves() {
    use horizon_world::{
        DATA_VERSION,
        collision::CachedCollisionRegistry,
        nbt::Tag,
        registry::{BlockRegistry, BlockStateId},
    };
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![Tag::String("minecraft:air".into())]),
        ),
        ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
    ]))
    .unwrap();
    let cached = CachedCollisionRegistry::from_nbt(
        &Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "states",
                Tag::List(vec![Tag::String("minecraft:air".into())]),
            ),
            ("shapes", Tag::List(vec![])),
            ("cached_shapes", Tag::IntArray(vec![-1])),
        ]),
        &registry,
    )
    .unwrap();
    let collision = BlockCollisionRegistry::new(std::sync::Arc::new(cached)).unwrap();
    let facts = BlockFacts {
        states: vec![],
        air: BlockStateId(0),
        woods: HashMap::new(),
    };
    let world = World::new(BlockStateId(0), 0);
    let level = ParticleLevel {
        world: &world,
        facts: &facts,
        collision: &collision,
        player: None,
    };
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.set_sprites.push(vec![10, 11, 12]);
    let sprite = |p: &Particle| match p.sprite {
        Sprite::Index(i) => i,
        Sprite::Named(_) => u32::MAX,
    };
    for declaration in [
        serde_json::json!({"animation": "loop", "frame_ticks": 2, "alpha": [1, 0], "lifetime": 100}),
        serde_json::json!({"animation": "loop", "frame_ticks": 2, "lifetime": 100}),
    ] {
        let mut particle = Particle::blank(ParticleType::Smoke, [0.0; 3]);
        particle.has_physics = false;
        particle.set = Some(0);
        ModBehaviour::parse(&declaration).unwrap().apply(
            &mut particle,
            [0.0; 3],
            &mut LegacyRandom::new(1),
        );
        let mut seen = Vec::new();
        for _ in 0..6 {
            engine.tick_particle(&mut particle, &level);
            seen.push(sprite(&particle));
        }
        assert_eq!(seen, [10, 11, 11, 12, 12, 10], "{declaration}");
    }
    let mut aged = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    aged.has_physics = false;
    aged.set = Some(0);
    ModBehaviour::parse(&serde_json::json!({"animation": "age", "lifetime": 4}))
        .unwrap()
        .apply(&mut aged, [0.0; 3], &mut LegacyRandom::new(1));
    for _ in 0..4 {
        engine.tick_particle(&mut aged, &level);
    }
    assert_eq!(sprite(&aged), 12);
    let mut kept = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    kept.has_physics = false;
    kept.set = Some(0);
    kept.sprite = Sprite::Index(11);
    ModBehaviour::parse(&serde_json::json!({"alpha": [1, 0], "lifetime": 4}))
        .unwrap()
        .apply(&mut kept, [0.0; 3], &mut LegacyRandom::new(1));
    for _ in 0..4 {
        engine.tick_particle(&mut kept, &level);
    }
    assert_eq!(sprite(&kept), 11, "no animation keeps the spawn sprite");
}

#[test]
fn mod_particle_ticks_motion_curves_and_removal() {
    use horizon_world::{
        DATA_VERSION,
        collision::CachedCollisionRegistry,
        nbt::Tag,
        registry::{BlockRegistry, BlockStateId},
    };
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![Tag::String("minecraft:air".into())]),
        ),
        ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
    ]))
    .unwrap();
    let cached = CachedCollisionRegistry::from_nbt(
        &Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "states",
                Tag::List(vec![Tag::String("minecraft:air".into())]),
            ),
            ("shapes", Tag::List(vec![])),
            ("cached_shapes", Tag::IntArray(vec![-1])),
        ]),
        &registry,
    )
    .unwrap();
    let collision = BlockCollisionRegistry::new(std::sync::Arc::new(cached)).unwrap();
    let facts = BlockFacts {
        states: vec![],
        air: BlockStateId(0),
        woods: HashMap::new(),
    };
    let world = World::new(BlockStateId(0), 0);
    let level = ParticleLevel {
        world: &world,
        facts: &facts,
        collision: &collision,
        player: None,
    };
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    let mut particle = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    particle.has_physics = false;
    ModBehaviour::parse(&serde_json::json!({
        "velocity": [1, 2, -1], "gravity": 1, "friction": 0.5,
        "lifetime": 2, "size": [0.25, 0.75], "alpha": [1, 0]
    }))
    .unwrap()
    .apply(&mut particle, [0.0; 3], &mut LegacyRandom::new(1));
    assert_eq!(particle.size, 0.25);
    assert_eq!(particle.rgba[3], 1.0);
    engine.tick_particle(&mut particle, &level);
    assert_eq!(particle.age, 1);
    assert_eq!(particle.previous, [0.0; 3]);
    assert_eq!(particle.position, [1.0, 1.96, -1.0]);
    assert_eq!(particle.velocity, [0.5, 0.98, -0.5]);
    assert_eq!(particle.size, 0.5);
    assert_eq!(particle.rgba[3], 0.5);
    engine.tick_particle(&mut particle, &level);
    assert_eq!(particle.position, [1.5, 2.9, -1.5]);
    assert_eq!(particle.velocity, [0.25, 0.47, -0.25]);
    assert_eq!(particle.size, 0.75);
    assert_eq!(particle.rgba[3], 0.0);
    assert!(!particle.removed);
    engine.tick_particle(&mut particle, &level);
    assert!(particle.removed);
    assert_eq!(particle.position, [1.5, 2.9, -1.5]);
}

#[test]
fn fading_mod_particles_use_the_blended_layer() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.rects.push(Some(Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    }));
    let mut particle = Particle::blank(ParticleType::Smoke, [0.0; 3]);
    ModBehaviour::parse(&serde_json::json!({"alpha": [0.5, 0]}))
        .unwrap()
        .apply(&mut particle, [0.0; 3], &mut LegacyRandom::new(1));
    particle.sprite = Sprite::Index(0);
    engine.particles.push(particle);
    let mut vertices = Vec::new();
    engine.render(DVec3::ZERO, 0.0, 0.0, 0.0, &|_| 0, &mut vertices);
    assert_eq!(vertices.len(), 4);
    assert!(
        vertices.iter().all(
            |v| v.color >> 24 == 127 && v.layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0
        )
    );
}

/// `LivingEntity.handleEntityEvent(46)`: 128 samples, the first on the old
/// box and the last on the new one, with speeds inside ±0.1.
#[test]
fn living_teleport_particles_span_the_old_and_new_boxes() {
    let mut random = LegacyRandom::new(1);
    let mut samples = Vec::new();
    living_teleport_particles(
        &mut random,
        [0.0, 1.0, 0.0],
        [8.0, 1.0, 0.0],
        0.6,
        1.8,
        |p, v| samples.push((p, v)),
    );
    assert_eq!(samples.len(), 128);
    let first = samples[0].0;
    let last = samples[127].0;
    assert!(first[0].abs() <= 0.6, "{first:?}");
    assert!((1.0..2.8).contains(&first[1]), "{first:?}");
    assert!(first[2].abs() <= 0.6, "{first:?}");
    assert!((7.4..8.6).contains(&last[0]), "{last:?}");
    assert!((1.0..2.8).contains(&last[1]), "{last:?}");
    assert!(last[2].abs() <= 0.6, "{last:?}");
    // `(nextDouble - 0.5) * width * 2` reaches ±width. A spread of only
    // `width` (the factor of 2 dropped) stays inside 0.3 of the line.
    let edge = samples
        .iter()
        .enumerate()
        .map(|(i, (p, _))| {
            let along = (i as f64 / 127.0) * 8.0;
            (p[0] - along).abs()
        })
        .fold(0.0_f64, f64::max);
    assert!(edge > 0.45, "box spread reached only {edge}");
    for (_, v) in &samples {
        for c in v {
            assert!((-0.1..0.1).contains(c), "{v:?}");
        }
    }
}

#[test]
fn egg_teleport_unpacks_offsets_and_preserves_seeded_draw_order() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.seed_randoms(0, 0, 0);
    // Destination offset (-15, 7, 15); the unused top byte is ignored.
    engine.block_teleportation([4, 12, -5], 0xff01_0f1fu32 as i32, [16, 8, 16]);
    assert_eq!(engine.pending.len(), 128);
    assert!(engine.pending.iter().all(|p| p.ty == ParticleType::Portal));
    let first = &engine.pending[0];
    assert_eq!(
        first.position.map(f64::to_bits),
        [0x3fd18220c05a530a, 0x402b001e6be383c1, 0xbfc76bf5c46dbe5c]
    );
    assert_eq!(
        first.velocity.map(f64::to_bits),
        [0xbfaa91aee0000000, 0x3f95c78ce0000000, 0x3f9c24a0e0000000]
    );
    assert_eq!(
        engine.pending[127].position.map(f64::to_bits),
        [0xbfd728b135cb9ef6, 0x402ddd72dc6d0019, 0x3fe0b5775a29f186]
    );
    assert_eq!(engine.level_random.state(), 99_413_408_083_437);
    assert!(engine.sounds.is_empty());
}

#[test]
fn consume_teleport_unpacks_offsets_within_127() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.seed_randoms(0, 0, 0);
    // TeleportRandomlyConsumeEffect: BlockUtil.clampedPackDifferenceInPosition
    // of the move (-7, -3, 9) with radii 127, from (4, 12, -5) to (-3, 9, 4).
    let data = (-7 + 127) << 16 | (-3 + 127) << 8 | (9 + 127);
    engine.block_teleportation([4, 12, -5], data, [127, 127, 127]);
    assert_eq!(engine.pending.len(), 128);
    for p in &engine.pending {
        let [x, y, z] = p.position;
        assert!((-3.0..5.0).contains(&x), "{x}");
        assert!((8.5..12.5).contains(&y), "{y}");
        assert!((-5.0..5.0).contains(&z), "{z}");
    }
    // The same draws as the egg's: only the unpacking differs.
    assert_eq!(engine.level_random.state(), 99_413_408_083_437);
}

#[test]
fn egg_teleport_options_roll_before_distance_rejection() {
    for (option, count, state) in [
        (0, 128, 99_413_408_083_437),
        (1, 89, 244_221_347_771_501),
        (2, 0, 99_413_408_083_437),
    ] {
        for distant in [false, true] {
            let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
            engine.seed_randoms(0, 0, 0);
            if distant {
                engine.camera = [1000.0; 3];
            }
            engine.particle_level = option;
            engine.block_teleportation([4, 12, -5], 0x010f1f, [16, 8, 16]);
            assert_eq!(engine.pending.len(), if distant { 0 } else { count });
            assert_eq!(engine.level_random.state(), state);
        }
    }
}

#[test]
fn mining_particles_follow_slab_and_trapdoor_height() {
    for height in [0.5, 3.0 / 16.0] {
        let boxes = [[0.0, 0.0, 0.0, 1.0, height, 1.0]];
        let p =
            breaking_position(&mut LegacyRandom::new(1), [2, 97, 3], [0, 1, 0], &boxes).unwrap();
        assert_eq!(p[1], 97.0 + height + f64::from(0.1f32));
        for seed in 0..128 {
            let p = breaking_position(&mut LegacyRandom::new(seed), [0; 3], [0, 0, -1], &boxes)
                .unwrap();
            assert!(p[1] <= height, "crumb above shape: {p:?}");
        }
    }
}
#[test]
fn destroyed_slab_uses_its_shape_and_particle_density() {
    let mut burst = Vec::new();
    destroy_positions([0; 3], &[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]], |p, v| {
        burst.push((p, v))
    });
    assert_eq!(burst.len(), 32);
    assert!(burst.iter().all(|(p, _)| p[1] < 0.5));
    assert_eq!(burst[0], ([0.125, 0.125, 0.125], [-0.375, -0.25, -0.375]));
}
#[test]
fn empty_outline_emits_no_terrain_particles_or_random_draws() {
    let mut r = LegacyRandom::new(1);
    assert_eq!(breaking_position(&mut r, [0; 3], [0, 1, 0], &[]), None);
    assert_eq!(r.next_long(), LegacyRandom::new(1).next_long());
    destroy_positions([0; 3], &[], |_, _| panic!("empty shape emitted a particle"));
}
#[test]
fn every_hit_face_consumes_three_position_draws() {
    let boxes = [[0.25, 0.0, 0.25, 0.75, 0.5, 0.75]];
    for face in [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ] {
        let mut random = LegacyRandom::new(42);
        let p = breaking_position(&mut random, [0; 3], face, &boxes).unwrap();
        let i = face.iter().position(|v| *v != 0).unwrap();
        let expected = if face[i] < 0 {
            boxes[0][i] - f64::from(0.1f32)
        } else {
            boxes[0][i + 3] + f64::from(0.1f32)
        };
        assert_eq!(p[i], expected);
        let mut reference = LegacyRandom::new(42);
        for _ in 0..3 {
            reference.next_double();
        }
        assert_eq!(random.next_long(), reference.next_long());
    }
}
#[test]
fn multipart_destroy_burst_uses_each_box_and_caps_widths() {
    let boxes = [
        [0.0, 0.0, 0.0, 1.0, 0.5, 1.0],
        [0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
    ];
    let mut burst = Vec::new();
    destroy_positions([0; 3], &boxes, |p, v| burst.push((p, v)));
    assert_eq!(burst.len(), 48);
    assert_eq!(burst[32].0, [0.125, 0.625, 0.625]);
    let mut tall = Vec::new();
    destroy_positions([0; 3], &[[0.0, 0.0, 0.0, 1.0, 1.5, 1.0]], |p, _| {
        tall.push(p)
    });
    assert_eq!(tall.len(), 64);
    assert!(tall.iter().all(|p| p[1] < 1.0));
}
#[test]
fn spore_blossom_air_stops_at_its_particle_limit() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    let spore = ParticleOptions::simple(ParticleType::SporeBlossomAir);
    for i in 0..1001 {
        engine.create(&spore, [f64::from(i % 16), 70.0, 0.0], [0.0; 3]);
    }
    assert_eq!(engine.pending.len(), 1000);
    assert!(
        engine
            .pending
            .iter()
            .all(|p| p.ty == ParticleType::SporeBlossomAir)
    );
    // A type without a `ParticleLimit` is not capped.
    let smoke = ParticleOptions::simple(ParticleType::Smoke);
    for _ in 0..10 {
        engine.create(&smoke, [0.0; 3], [0.0; 3]);
    }
    assert_eq!(engine.pending.len(), 1010);
}
/// `VibrationSignalParticle.Provider.createParticle`: the provider path
/// the client's packet handler reaches (`add_with`) makes the signal from
/// the option's destination and arrival time, where it used to refuse it.
#[test]
fn a_vibration_option_makes_the_signal_its_provider_draws() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    let options =
        ParticleOptions::vibration(PositionSource::Block([10, 70, -4]), [10.5, 70.5, -3.5], 20);
    // Within the camera's 32 blocks, so only the provider decides.
    engine.add_with(&options, [0.5, 1.5, 0.5], [0.0; 3], false, false);
    assert_eq!(engine.made().get("vibration"), Some(&1));
    let q = engine.pending.first().expect("the signal is made");
    assert_eq!(q.ty, ParticleType::Vibration);
    assert_eq!(q.lifetime, 20);
    assert!((q.size - 0.3).abs() < 1e-6);
    assert!(!q.has_physics);
    match &q.behaviour {
        Behaviour::Vibration {
            dest, rot, pitch, ..
        } => {
            assert_eq!(*dest, [10.5, 70.5, -3.5]);
            // The constructor aims it from where it starts at the listener.
            let d: [f64; 3] = [0.5 - 10.5, 1.5 - 70.5, 0.5 + 3.5];
            assert!((*rot - d[0].atan2(d[2]) as f32).abs() < 1e-6);
            let flat = (d[0] * d[0] + d[2] * d[2]).sqrt();
            assert!((*pitch - d[1].atan2(flat) as f32).abs() < 1e-6);
        }
        _ => panic!("a vibration signal"),
    }
    // Only `VibrationParticleOption` carries a listener; a bare type has none.
    let bare = ParticleOptions::simple(ParticleType::Vibration);
    assert!(
        engine.make(&bare, [0.0; 3], [0.0; 3], 1).is_none(),
        "no destination, no signal"
    );
}
#[test]
fn particle_names_round_trip() {
    for &ty in ParticleType::ALL {
        assert_eq!(
            ParticleType::from_name(&format!("minecraft:{}", ty.name())),
            Some(ty)
        );
    }
    assert_eq!(ParticleType::from_name("minecraft:not_a_particle"), None);
}
#[test]
fn terrain_crumb_samples_a_mirrored_quarter() {
    let mut r = LegacyRandom::new(1);
    let p = Particle::terrain(&mut r, [0.5; 3], [0.0; 3], "block/stone".into(), 0xFFFFFF);
    assert!((p.uv[0] - p.uv[2] - 0.25).abs() < 1e-6);
    assert!((p.uv[3] - p.uv[1] - 0.25).abs() < 1e-6);
    assert!((p.rgba[0] - 0.6).abs() < 1e-6);
    // The initial speed is normalized to 0.15..0.45 * 0.4 plus 0.1 upwards.
    let s = (p.velocity[0].powi(2) + (p.velocity[1] - 0.1).powi(2) + p.velocity[2].powi(2)).sqrt();
    assert!((0.15 * 0.4 - 1e-6..=0.45 * 0.4 + 1e-6).contains(&s));
}
#[test]
fn legacy_trails_choose_a_lifetime_for_each_particle_from_the_level_random() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.seed_randoms(1, 42, 2);
    let legacy = ParticleOptions {
        legacy_trail_lifetime: true,
        ..ParticleOptions::trail([1.0, 2.0, 3.0], 0x7f3f1f, -1)
    };
    for _ in 0..5 {
        engine.create(&legacy, [0.0; 3], [0.0; 3]);
    }
    // java.util.Random(42).nextInt(40) + 10, as vanilla 1.21.3 TrailParticle.Provider does.
    assert_eq!(
        engine
            .pending
            .iter()
            .map(|p| p.lifetime)
            .collect::<Vec<_>>(),
        [20, 13, 18, 14, 20]
    );
    engine.create(
        &ParticleOptions::trail([1.0, 2.0, 3.0], 0x7f3f1f, 37),
        [0.0; 3],
        [0.0; 3],
    );
    assert_eq!(engine.pending.last().unwrap().lifetime, 37);
    // A current server's -1 is a lifetime like any other, not the old marker.
    engine.create(
        &ParticleOptions::trail([1.0, 2.0, 3.0], 0x7f3f1f, -1),
        [0.0; 3],
        [0.0; 3],
    );
    assert_eq!(engine.pending.last().unwrap().lifetime, -1);
}
#[test]
fn sizes_follow_vanilla_curves() {
    let mut r = LegacyRandom::new(2);
    let mut p = Particle::at(&mut r, ParticleType::Flame, [0.0; 3]);
    p.size = 1.0;
    p.lifetime = 10;
    p.age = 5;
    p.size_curve = Size::Flame;
    assert!((p.quad_size(0.0) - (1.0 - 0.25 * 0.5)).abs() < 1e-6);
    p.size_curve = Size::GrowIn;
    p.age = 0;
    assert!((p.quad_size(0.1) - (0.1f32 / 10.0 * 32.0)).abs() < 1e-6);
}
#[test]
fn lifetime_alpha_interpolates() {
    let a = [0.0, 0.6, 0.25, 1.0];
    assert_eq!(lifetime_alpha(a, 0, 40, 0.0), 0.0);
    assert!((lifetime_alpha(a, 40, 40, 0.0) - 0.6).abs() < 1e-6);
}
#[test]
fn explosion_tracker_tracks_and_ignores_zero_blocks() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    assert!(engine.tracked_explosions.is_empty());
    engine.track_explosion([1.0, 2.0, 3.0], 4.0, 100);
    engine.track_explosion([5.0, 6.0, 7.0], 2.0, 50);
    assert_eq!(engine.tracked_explosions.len(), 2);
    assert_eq!(engine.tracked_explosions[0].block_count, 100);
    assert_eq!(engine.tracked_explosions[1].block_count, 50);

    // Zero block count explosions are not queued.
    engine.track_explosion([0.0; 3], 1.0, 0);
    assert_eq!(engine.tracked_explosions.len(), 2);
}
#[test]
fn explosion_tracker_caps_debris_at_512_across_multiple_explosions() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    // Queue two explosions near camera: 400 blocks and 300 blocks (total 700).
    engine.track_explosion([0.0, 0.0, 0.0], 4.0, 400);
    engine.track_explosion([2.0, 0.0, 2.0], 5.0, 300);
    assert_eq!(engine.tracked_explosions.len(), 2);

    // In pure air, total particles spawned must be capped at 512.
    engine.tick_explosions_with(&|_| true);
    assert_eq!(engine.pending.len(), 512);
    assert!(engine.tracked_explosions.is_empty());
}
#[test]
fn explosion_tracker_emits_debris_only_in_air() {
    let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
    engine.track_explosion([0.0, 64.0, 0.0], 4.0, 100);

    // When no blocks are air, zero debris particles are emitted.
    engine.tick_explosions_with(&|_| false);
    assert_eq!(engine.pending.len(), 0);
    assert!(engine.tracked_explosions.is_empty());
}
