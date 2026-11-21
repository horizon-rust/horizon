use super::*;

/// The centre of a section, in a world whose section 0 starts at y 0.
fn at(key: SectionKey) -> DVec3 {
    DVec3::new(
        f64::from(key.0) * 16.0 + 8.0,
        f64::from(key.1) * 16.0 + 8.0,
        f64::from(key.2) * 16.0 + 8.0,
    )
}

#[test]
fn constants_are_vanillas() {
    assert_eq!(SECTION_DIAGONAL, (3f64.sqrt() * 16.0).ceil());
    assert_eq!(DISTANT_SECTIONS, 3);
}

#[test]
fn floor_is_std_floor() {
    let mut x = -40_000.0_f64;
    while x < 40_000.0 {
        for v in [x, x + 0.5, x - 1e-9, x + 1e-9, -x] {
            assert_eq!(floor(v), v.floor() as i32, "{v}");
        }
        x += 0.37;
    }
}

#[test]
fn open_space_reaches_the_whole_box() {
    let mut o = Occlusion::new(0, 4);
    o.settle(at((0, 1, 0)), 2);
    for x in -2..=2 {
        for z in -2..=2 {
            for y in -1..=4 {
                assert!(o.contains((x, y, z)), "{x} {y} {z}");
            }
        }
    }
    assert!(!o.contains((3, 1, 0)));
}

#[test]
fn closed_neighbours_hide_what_is_behind_them() {
    let mut o = Occlusion::new(0, 4);
    for (dx, dy, dz) in STEPS {
        o.set((dx, 1 + dy, dz), 0);
    }
    o.settle(at((0, 1, 0)), 3);
    assert!(o.contains((1, 1, 0)), "a neighbour is seen");
    assert!(!o.contains((2, 1, 0)), "nothing shows through it");
    o.set((1, 1, 0), ALL_VISIBLE);
    o.settle(at((0, 1, 0)), 3);
    assert!(o.contains((2, 1, 0)));
}

/// `visible_for_camera` answers only for the box of the graph it holds, and
/// only for the camera that search started from. Everywhere else it says
/// unknown, so the caller keeps the geometry.
#[test]
fn visible_for_camera_is_unknown_outside_its_box_and_its_camera() {
    // No graph yet: nothing to answer with.
    assert_eq!(
        Occlusion::new(0, 4).visible_for_camera((0, 1, 0), (0, 1, 0)),
        None
    );
    let mut o = Occlusion::new(0, 4);
    o.settle(at((0, 1, 0)), 2);
    let from = o.columns.section_of(at((0, 1, 0)));
    assert_eq!(from, (0, 1, 0), "the camera section searched from");
    // Inside the searched box the answer is `contains`, its edges too.
    for x in -2..=2 {
        for z in -2..=2 {
            for y in -1..=4 {
                assert_eq!(
                    o.visible_for_camera((x, y, z), from),
                    Some(o.contains((x, y, z))),
                    "{x} {y} {z}"
                );
            }
        }
    }
    assert_eq!(
        o.visible_for_camera((2, 1, 0), from),
        Some(true),
        "the box's edge"
    );
    // Off the box's sides, below it, or above the level: unknown.
    for key in [
        (3, 1, 0),
        (-3, 1, 0),
        (0, 1, 3),
        (0, 1, -3),
        (0, -2, 0),
        (0, 5, 0),
    ] {
        assert_eq!(o.visible_for_camera(key, from), None, "{key:?}");
    }
    // Another camera section: this graph is not about it.
    assert_eq!(o.visible_for_camera((0, 1, 0), (1, 1, 0)), None);
}

/// The answer follows the graph: a closed wall hides the section behind it,
/// and opening the wall shows it again.
#[test]
fn visible_for_camera_reports_what_the_graph_hides() {
    let mut o = Occlusion::new(0, 4);
    for (dx, dy, dz) in STEPS {
        o.set((dx, 1 + dy, dz), 0);
    }
    o.settle(at((0, 1, 0)), 3);
    let from = (0, 1, 0);
    assert_eq!(
        o.visible_for_camera((1, 1, 0), from),
        Some(true),
        "the wall itself"
    );
    assert_eq!(
        o.visible_for_camera((2, 1, 0), from),
        Some(false),
        "hidden behind it"
    );
    o.set((1, 1, 0), ALL_VISIBLE);
    o.settle(at((0, 1, 0)), 3);
    assert_eq!(o.visible_for_camera((2, 1, 0), from), Some(true));
}

/// Faces `a` and `b` of a section see each other, nothing else does.
fn connects(a: usize, b: usize) -> u64 {
    1 << (a * 6 + b) | 1 << (b * 6 + a)
}

/// #629 / #782 rule 1: a section first entered through a face that leads
/// nowhere still leads on through the face it is entered through later.
#[test]
fn a_section_leads_on_through_every_face_it_is_entered_through() {
    let mut o = Occlusion::new(0, 4);
    // (1, 1, 1) is reached through its west face first (from (0, 1, 1),
    // queued before (1, 1, 0)), then through its north face. Only north
    // leads on, to the east.
    o.set((1, 1, 1), connects(2, 5));
    // Every other way into (2, 1, 1) is closed.
    for key in [(2, 1, 0), (2, 0, 1), (2, 2, 1)] {
        o.set(key, 0);
    }
    o.settle(at((0, 1, 0)), 3);
    let graph = o.graph.as_ref().expect("searched");
    let i = graph.index((1, 1, 1)).unwrap();
    assert_eq!(
        graph.entered[i],
        1 << 2 | 1 << 4,
        "entered from north and west"
    );
    assert!(o.contains((2, 1, 1)), "seen through the north face");
}

#[test]
fn frames_never_wait_and_partial_updates_open_at_once() {
    let mut o = Occlusion::new(0, 4);
    for (dx, dy, dz) in STEPS {
        o.set((dx, 1 + dy, dz), 0);
    }
    let camera = at((0, 1, 0));
    o.settle(camera, 3);
    assert!(!o.contains((2, 1, 0)));
    // Opening the wall shows what is behind it in the same frame, from the
    // graph the frame has, whether or not the full search has finished.
    o.set((1, 1, 0), ALL_VISIBLE);
    o.update(camera, 3);
    assert!(o.contains((2, 1, 0)));
    assert!(o.contains((3, 1, 0)));
    // Closing it again lasts until the next full search lands.
    o.set((1, 1, 0), 0);
    o.settle(camera, 3);
    o.settle(camera, 3);
    assert!(!o.contains((2, 1, 0)));
}

#[test]
fn a_graph_that_lost_the_camera_culls_nothing() {
    let mut o = Occlusion::new(0, 4);
    for (dx, dy, dz) in STEPS {
        o.set((dx, 1 + dy, dz), 0);
    }
    o.settle(at((0, 1, 0)), 3);
    assert!(!o.contains((3, 1, 0)));
    // The camera jumped past the wall; the next search is still running.
    let o = &mut o;
    o.update(at((3, 1, 0)), 3);
    if o.running.is_some() {
        assert!(o.contains((3, 1, 0)));
    }
    o.settle(at((3, 1, 0)), 3);
    assert!(o.contains((3, 1, 0)));
    assert!(
        !o.contains((0, 1, 0)),
        "the old camera section is walled in"
    );
}

/// #1037: from far above or below the world, the search starts from the
/// world's top or bottom layer (`initializeQueueForFullUpdate`), so the
/// ground stays drawn at any height.
#[test]
fn a_camera_outside_the_world_sees_it_from_its_top_or_bottom() {
    for from in [(0, 6, 0), (0, 9, 0), (2, 40, -1), (0, -1, 0), (1, -7, 0)] {
        let mut o = Occlusion::new(0, 6);
        o.settle(at(from), 3);
        for x in -1..=5 {
            for z in -4..=2 {
                for y in 0..6 {
                    let inside = (x - from.0).abs() <= 3 && (z - from.2).abs() <= 3;
                    assert_eq!(o.contains((x, y, z)), inside, "{from:?}: {x} {y} {z}");
                }
            }
        }
        assert!(
            !o.contains(from),
            "{from:?}: the camera's own section is not in the world"
        );
    }
}

/// Seeds enter through the outer face and never turn back up (down)
/// or towards the camera's column.
#[test]
fn a_closed_top_layer_hides_the_world_from_above() {
    let mut o = Occlusion::new(0, 6);
    for x in -3..=3 {
        for z in -3..=3 {
            o.set((x, 5, z), 0);
        }
    }
    o.settle(at((0, 30, 0)), 3);
    assert!(
        o.contains((0, 5, 0)) && o.contains((3, 5, -3)),
        "the top is seen"
    );
    assert!(
        !o.contains((0, 4, 0)) && !o.contains((2, 1, 2)),
        "nothing under it"
    );
    assert!(!o.contains((0, 6, 0)), "seeds don't step back up");
    // From below, the open bottom layer shows everything up to the lid.
    o.settle(at((0, -9, 0)), 3);
    assert!(o.contains((0, 0, 0)) && o.contains((1, 4, 1)) && o.contains((1, 5, 1)));
    assert!(!o.contains((0, -1, 0)), "seeds don't step back down");
}

/// Vanilla's `runUpdates` as written (a map of nodes, a FIFO queue,
/// `Vector3d` maths), over the same box: whether each cell is reached.
/// `rays: false` leaves out the distant step test.
fn reference(columns: &Columns, camera: DVec3, radius: i32, rays: bool) -> Vec<bool> {
    struct Node {
        directions: u8,
        sources: u8,
    }
    let from = columns.section_of(camera);
    let y0 = -1;
    let height = columns.sections + 2;
    let side = 2 * radius + 1;
    let index = |k: SectionKey| {
        let (dx, dz, dy) = (k.0 - from.0, k.2 - from.2, k.1 - y0);
        (dx.abs() <= radius && dz.abs() <= radius && (0..height).contains(&dy))
            .then(|| (((dz + radius) * side + dx + radius) * height + dy) as usize)
    };
    let block = |k: SectionKey| [k.0 * 16, k.1 * 16 + columns.min_y, k.2 * 16];
    // Direction.getAxis in x, y, z order.
    let axis = |d: usize| [1, 2, 0][d / 2];
    let centre = block(from).map(|c| c + 8);
    let mut nodes: HashMap<SectionKey, Node> = HashMap::new();
    let mut queue = VecDeque::new();
    if (0..columns.sections).contains(&from.1) {
        nodes.insert(
            from,
            Node {
                directions: 0,
                sources: 0,
            },
        );
        queue.push_back(from);
    } else {
        // initializeQueueForFullUpdate from outside the world.
        let above = from.1 >= columns.sections;
        let (y, travel) = if above {
            (columns.sections - 1, 0)
        } else {
            (0, 1)
        };
        let camera_block = [camera.x.floor() as i64, camera.z.floor() as i64];
        let mut seeds = Vec::new();
        for x in -radius..=radius {
            for z in -radius..=radius {
                let key = (from.0 + x, y, from.2 + z);
                let mut directions = 1 << travel;
                if x > 0 {
                    directions |= 1 << 5;
                } else if x < 0 {
                    directions |= 1 << 4;
                }
                if z > 0 {
                    directions |= 1 << 3;
                } else if z < 0 {
                    directions |= 1 << 2;
                }
                let origin = block(key);
                let dx = camera_block[0] - i64::from(origin[0] + 8);
                let dz = camera_block[1] - i64::from(origin[2] + 8);
                seeds.push((dx * dx + dz * dz, key, directions));
            }
        }
        seeds.sort_by_key(|seed| seed.0);
        for (_, key, directions) in seeds {
            nodes.insert(
                key,
                Node {
                    directions,
                    sources: 1 << travel,
                },
            );
            queue.push_back(key);
        }
    }
    while let Some(key) = queue.pop_front() {
        let node = &nodes[&key];
        let (directions, sources) = (node.directions, node.sources);
        let vis = columns.get(key);
        let distant =
            (key.0 - from.0).abs() > 3 || (key.1 - from.1).abs() > 3 || (key.2 - from.2).abs() > 3;
        for (d, step) in STEPS.iter().enumerate() {
            let next = (key.0 + step.0, key.1 + step.1, key.2 + step.2);
            if next.1 < -1 || next.1 > columns.sections || index(next).is_none() {
                continue;
            }
            if directions & (1 << (d ^ 1)) != 0 {
                continue;
            }
            // facesCanSeeEachother(source.getOpposite(), direction).
            if sources != 0
                && !(0..6).any(|s| sources & (1 << s) != 0 && vis & (1 << ((s ^ 1) * 6 + d)) != 0)
            {
                continue;
            }
            if rays && distant {
                let origin = block(key);
                let max = |c: usize| {
                    if c == axis(d) {
                        centre[c] > origin[c]
                    } else {
                        centre[c] < origin[c]
                    }
                };
                let mut check = DVec3::new(
                    f64::from(origin[0] + if max(0) { 16 } else { 0 }),
                    f64::from(origin[1] + if max(1) { 16 } else { 0 }),
                    f64::from(origin[2] + if max(2) { 16 } else { 0 }),
                );
                let length = |v: DVec3| (v.x * v.x + (v.y * v.y + v.z * v.z)).sqrt();
                let to = camera - check;
                let step = to * (1.0 / length(to)) * 28.0;
                let mut visible = true;
                while length(check - camera).powi(2) > 3600.0 {
                    check += step;
                    let max_y = columns.min_y + columns.sections * 16 - 1;
                    if check.y > f64::from(max_y) || check.y < f64::from(columns.min_y) {
                        break;
                    }
                    let section = columns.section_of(check);
                    if !nodes.contains_key(&section) {
                        visible = false;
                        break;
                    }
                }
                if !visible {
                    continue;
                }
            }
            match nodes.get_mut(&next) {
                Some(existing) => existing.sources |= 1 << d,
                None => {
                    nodes.insert(
                        next,
                        Node {
                            directions: directions | 1 << d,
                            sources: 1 << d,
                        },
                    );
                    queue.push_back(next);
                }
            }
        }
    }
    let mut reached = vec![false; (side * side * height) as usize];
    for key in nodes.keys() {
        reached[index(*key).unwrap()] = true;
    }
    reached
}

fn random_world(o: &mut Occlusion, seed: u64, count: usize, spread: i32) {
    let mut seed = seed;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let span = (2 * spread + 1) as u64;
    for _ in 0..count {
        let r = next();
        let key = (
            (r % span) as i32 - spread,
            ((r >> 8) % 6) as i32,
            ((r >> 16) % span) as i32 - spread,
        );
        // Open, half open or nearly closed sections.
        let vis = match r >> 40 & 3 {
            0 => next() & next() & next(),
            1 => next(),
            _ => next() | next(),
        };
        o.set(key, vis & ALL_VISIBLE);
    }
}

fn reached(o: &Occlusion) -> Vec<bool> {
    let graph = o.graph.as_ref().expect("searched");
    graph.directions.iter().map(|&d| d & REACHED != 0).collect()
}

/// The graph is vanilla's, cell for cell, near the camera and beyond the
/// distant step test's 60 blocks.
#[test]
#[ignore = "daily"]
fn same_sections_as_vanilla() {
    let (mut hidden, mut rayed) = (0, 0);
    for seed in [0x9e37_79b9, 7, 0xdead_beef] {
        for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
            let mut o = Occlusion::new(0, 6);
            random_world(&mut o, seed, count, spread);
            for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4), (1, -1, 1), (0, 7, 0)] {
                // Off-centre, so the rays are not axis-aligned.
                let camera = at(from) + DVec3::new(3.3, -5.7, 1.9);
                o.settle(camera, radius);
                o.settle(camera, radius);
                let reached = reached(&o);
                assert_eq!(
                    reached,
                    reference(&o.columns, camera, radius, true),
                    "{seed} {from:?} {radius}"
                );
                let no_rays = reference(&o.columns, camera, radius, false);
                rayed += no_rays
                    .iter()
                    .zip(&reached)
                    .filter(|(a, b)| **a && !**b)
                    .count();
                hidden += reached.iter().filter(|&&r| !r).count();
            }
        }
    }
    assert!(hidden > 1000, "the worlds hide sections ({hidden})");
    assert!(
        rayed > 100,
        "the distant step test culls sections ({rayed})"
    );
}

/// A clamped search may conservatively show sections above its searched
/// box, but it must never hide one that the full-height search reaches.
#[test]
#[ignore = "daily"]
fn vertical_clamping_only_adds_overdraw() {
    let mut compared = 0;
    let mut overdraw = 0;
    let mut clamped = 0;
    for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
        for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
            let mut o = Occlusion::new(0, 24);
            random_world(&mut o, seed, count, spread);
            let camera = at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9);
            o.settle(camera, radius);
            let graph = o.graph.as_ref().expect("searched");
            clamped += usize::from(graph.height < o.columns.sections + 2);
            let full = reference(&o.columns, camera, radius, true);
            let from = o.columns.section_of(camera);
            let side = 2 * radius + 1;
            let height = o.columns.sections + 2;
            let full_contains = |key: SectionKey| {
                let (dx, dz, dy) = (key.0 - from.0, key.2 - from.2, key.1 + 1);
                if dx.abs() > radius || dz.abs() > radius || !(0..height).contains(&dy) {
                    return false;
                }
                full[(((dz + radius) * side + dx + radius) * height + dy) as usize]
            };
            for x in from.0 - radius..=from.0 + radius {
                for z in from.2 - radius..=from.2 + radius {
                    for y in -1..=o.columns.sections {
                        let key = (x, y, z);
                        let expected = full_contains(key);
                        let actual = o.contains(key);
                        compared += 1;
                        assert!(
                            !expected || actual,
                            "clamped search hid {key:?} for seed {seed:#x}, spread {spread}, radius {radius}; from={from:?}, y0={}, height={}, expected={expected}, actual={actual}",
                            graph.y0,
                            graph.height
                        );
                        overdraw += usize::from(actual && !expected);
                        if actual && !expected {
                            assert!(
                                y >= graph.y0 + graph.height,
                                "overdraw must be above the clamped box: {key:?}, y0={}, height={}",
                                graph.y0,
                                graph.height
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(clamped > 0, "the comparison exercised a clamped search");
    assert!(compared > 50_000, "compared {compared} sections");
    assert!(overdraw > 0, "the conservative top-cell rule was exercised");
}

/// A line checked again from the sample it failed at, after more
/// sections were reached, ends where checking it from its corner does.
#[test]
fn a_line_checked_again_resumes_where_it_failed() {
    let mut seed = 0x2545_f491_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let columns = Columns::new(-64, 24);
    let mut graph = Graph::default();
    let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
    let (mut resumed, mut seen) = (0, 0);
    for _ in 0..40 {
        let camera = DVec3::new(
            (next() % 3000) as f64 / 100.0 - 15.0,
            (next() % 30000) as f64 / 100.0 - 60.0,
            (next() % 3000) as f64 / 100.0 - 15.0,
        );
        // Open air: every section reached; then holes along the lines.
        graph.search(&columns, camera, 12, &mut vis, &mut queue);
        for cell in graph.directions.iter_mut() {
            if next() % 4 == 0 {
                *cell = 0;
            }
        }
        let from = graph.from;
        for _ in 0..25 {
            let corner = (
                from.0 + (next() % 25) as i32 - 12,
                graph.y0 + (next() % graph.height as u64) as i32,
                from.2 + (next() % 25) as i32 - 12,
            );
            let mut known = 0;
            while let Some(failed) = graph.walk(corner, camera, known) {
                for earlier in 0..=failed {
                    assert_eq!(graph.walk(corner, camera, earlier), Some(failed));
                }
                known = failed;
                resumed += 1;
                // More sections reached, as the search goes on.
                for cell in graph.directions.iter_mut() {
                    if next() % 3 == 0 {
                        *cell |= REACHED;
                    }
                }
                assert_eq!(
                    graph.walk(corner, camera, known),
                    graph.walk(corner, camera, 0)
                );
            }
            seen += usize::from(known > 1);
        }
    }
    assert!(resumed > 200 && seen > 50, "{resumed} {seen}");
}

/// #782 rule 2: behind a wall, a section the search reaches around its
/// edge is culled when the line to the camera crosses the wall's shadow.
#[test]
fn distant_sections_behind_a_wall_are_culled() {
    let mut o = Occlusion::new(0, 4);
    // A closed wall at x 2, z -2..=2, the whole height.
    for z in -2..=2 {
        for y in -1..=4 {
            o.set((2, y, z), 0);
        }
    }
    let camera = at((0, 1, 0));
    o.settle(camera, 12);
    let no_rays = reference(&o.columns, camera, 12, false);
    let graph = o.graph.as_ref().expect("searched");
    let behind = graph.index((9, 1, 3)).unwrap();
    assert!(no_rays[behind], "reached around the wall's edge");
    assert!(
        !o.contains((9, 1, 3)),
        "but its line to the camera is hidden"
    );
    assert!(o.contains((9, 1, 9)), "a section off to the side is seen");
}

/// `cargo test --profile fast -p horizon-client --lib occlusion::tests::search_cost -- --ignored --nocapture`:
/// full searches at render distance 32 over a world with open sky above
/// y 64 and caves below it.
#[test]
#[ignore]
fn search_cost() {
    let mut o = Occlusion::new(-64, 24);
    let mut seed = 0x9e37_79b9_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for x in -40..=40 {
        for z in -40..=40 {
            for y in 0..24 {
                let vis = if y >= 8 {
                    ALL_VISIBLE
                } else if next() % 5 == 0 {
                    next() & ALL_VISIBLE
                } else {
                    0
                };
                o.set((x, y, z), vis);
            }
        }
    }
    let camera = DVec3::new(3.3, 70.2, -5.6);
    let (mut vis, mut queue) = (Vec::new(), VecDeque::new());
    let mut graph = Graph::default();
    let mut best = f64::MAX;
    for _ in 0..12 {
        let started = std::time::Instant::now();
        graph.search(&o.columns, camera, 34, &mut vis, &mut queue);
        best = best.min(started.elapsed().as_secs_f64() * 1000.0);
    }
    let reached = graph
        .directions
        .iter()
        .filter(|&&d| d & REACHED != 0)
        .count();
    println!("search_cost: best {best:.2} ms over 12, {reached} reached");
}

/// Removed columns see through every face: the graph is the one of a world that
/// never had them, whether some columns go or all of them (a teleport).
#[test]
fn removed_columns_leave_the_graph_of_a_world_without_them() {
    fn without(o: &Occlusion, gone: &FxHashSet<(i32, i32)>) -> Occlusion {
        let mut fresh = Occlusion::new(0, 6);
        for (&(x, z), column) in &o.columns.map {
            if !gone.contains(&(x, z)) {
                for (y, &vis) in column.iter().enumerate() {
                    fresh.set((x, y as i32, z), vis);
                }
            }
        }
        fresh
    }
    let camera = at((0, 2, 0));
    let mut o = Occlusion::new(0, 6);
    random_world(&mut o, 0x51ed_2701, 900, 6);
    o.settle(camera, 6);
    let some: FxHashSet<(i32, i32)> = (-6..=6)
        .flat_map(|x| (-1..=1).map(move |z| (x, z)))
        .collect();
    let mut expected = without(&o, &some);
    o.remove_columns(&some);
    o.settle(camera, 6);
    expected.settle(camera, 6);
    let graph = |o: &Occlusion| o.graph.as_ref().expect("searched").directions.clone();
    assert_eq!(graph(&o), graph(&expected));

    let all: FxHashSet<(i32, i32)> = o.columns.map.keys().copied().collect();
    o.remove_columns(&all);
    assert!(o.contains((40, 3, 40)), "no graph from the old view culls");
    random_world(&mut o, 0x2545_f491, 900, 6);
    let mut arrived = Occlusion::new(0, 6);
    random_world(&mut arrived, 0x2545_f491, 900, 6);
    o.settle(camera, 6);
    arrived.settle(camera, 6);
    assert_eq!(graph(&o), graph(&arrived));
}

/// The worker's graph is the one the render thread would compute.
#[test]
fn the_worker_matches_a_search_on_the_render_thread() {
    let mut o = Occlusion::new(0, 6);
    random_world(&mut o, 0x9e37_79b9, 800, 6);
    for from in [(0, 2, 0), (3, 0, -2), (-5, 5, 4)] {
        let camera = at(from);
        o.settle(camera, 6);
        o.settle(camera, 6);
        let mut local = Graph::default();
        local.search(&o.columns, camera, 6, &mut Vec::new(), &mut VecDeque::new());
        let graph = o.graph.as_ref().expect("searched");
        assert_eq!(graph.directions, local.directions);
        assert_eq!(graph.entered, local.entered);
    }
}

#[test]
fn vertical_clamping_bounds_height_and_reaches_open_sky() {
    let mut o = Occlusion::new(0, 24);
    o.set((0, 5, 0), 0); // Wall at section 5
    o.settle(at((0, 2, 0)), 4);
    let graph = o.graph.as_ref().expect("searched");
    assert_eq!(graph.height, 5 + 3);
    assert!(o.contains((0, 2, 0)), "camera section is reached");
    assert!(o.contains((1, 20, 1)), "open air column is reached");

    // An enclosed cave cannot reach the sky
    let mut cave = Occlusion::new(0, 24);
    for (dx, dy, dz) in STEPS {
        cave.set((dx, 1 + dy, dz), 0);
    }
    cave.settle(at((0, 1, 0)), 4);
    assert!(
        !cave.contains((0, 10, 0)),
        "enclosed cave does not see the sky"
    );
}

/// `contains` over the columns `radius` + 3 around `from`, from two
/// sections below the level to two above it, checked against the graph
/// `reach` exports: `None` exactly without a graph, the same answer for
/// every key, nothing past its box, its bits its cells' `REACHED`, and its
/// sky band reached exactly when a bit of its top layer is.
fn answers(o: &Occlusion, from: SectionKey, radius: i32) -> Vec<bool> {
    let reach = o.reach();
    assert_eq!(reach.is_some(), o.graph.is_some(), "a graph exported");
    assert!(o.reach_is_exact(), "the bits are the cells'");
    if let Some(g) = &o.graph {
        let height = g.height as usize;
        let mut top = (height - 1..g.directions.len()).step_by(height);
        let sky = top.any(|i| (g.bits[i / 32] >> (i % 32)) & 1 != 0);
        assert_eq!(g.sky_reached, sky, "the sky band");
    }
    let past = |key: SectionKey| {
        o.graph.as_ref().is_some_and(|g| {
            (key.0 - g.from.0).abs() > g.radius
                || (key.2 - g.from.2).abs() > g.radius
                || !(g.y0..=g.sections).contains(&key.1)
        })
    };
    let mut answers = Vec::new();
    for x in from.0 - radius - 3..=from.0 + radius + 3 {
        for z in from.2 - radius - 3..=from.2 + radius + 3 {
            for y in -2..=o.columns.sections + 1 {
                let key = (x, y, z);
                let contains = o.contains(key);
                assert_eq!(reach.is_none_or(|r| r.contains(key)), contains, "{key:?}");
                assert!(!(contains && past(key)), "{key:?} past the box");
                answers.push(contains);
            }
        }
    }
    answers
}

/// The graph `reach` exports answers as `contains` (`answers`): walled in,
/// opened by a partial update, cleared; over clamped boxes, the sky band
/// above them reached or not (a closed cave); from above the world. The
/// search, the partial update and `clear` each change the generation, and
/// another level's graph never repeats one.
#[test]
fn the_exported_graph_answers_as_contains() {
    let mut o = Occlusion::new(0, 4);
    let first = o.generation();
    answers(&o, (0, 1, 0), 3);
    for (dx, dy, dz) in STEPS {
        o.set((dx, 1 + dy, dz), 0);
    }
    o.settle(at((0, 1, 0)), 3);
    assert_ne!(o.generation(), first, "a search landed");
    let walled = answers(&o, (0, 1, 0), 3);
    let searched = o.generation();
    o.set((0, 1, 1), ALL_VISIBLE);
    o.update(at((0, 1, 0)), 3);
    assert_ne!(o.generation(), searched, "a partial update reached more");
    assert_ne!(answers(&o, (0, 1, 0), 3), walled);
    let opened = o.generation();
    o.clear();
    assert_ne!(o.generation(), opened, "the graph went");
    answers(&o, (0, 1, 0), 3);
    // Another level's graph never takes one of this one's generations, or
    // the pass would keep its copy of this one.
    let mut other = Occlusion::new(0, 4);
    other.settle(at((0, 1, 0)), 3);
    assert_ne!(other.generation(), searched, "another level's graph");

    let (mut clamped, mut band) = (0, [0; 2]);
    let mut check = |mut o: Occlusion, camera: DVec3, radius: i32| {
        o.settle(camera, radius);
        answers(&o, o.columns.section_of(camera), radius);
        let g = o.graph.as_ref().expect("searched");
        if g.height < o.columns.sections + 2 {
            clamped += 1;
            // The first section above the box, over the camera's column.
            band[usize::from(o.contains((g.from.0, g.y0 + g.height, g.from.2)))] += 1;
        }
    };
    for seed in [0x9e37_79b9, 7, 0xdead_beef, 0x2545_f491] {
        for (spread, count, radius) in [(6, 900, 6), (12, 2600, 12)] {
            let mut o = Occlusion::new(0, 24);
            random_world(&mut o, seed, count, spread);
            check(o, at((0, 2, 0)) + DVec3::new(3.3, -5.7, 1.9), radius);
        }
    }
    let mut cave = Occlusion::new(0, 24);
    for (dx, dy, dz) in STEPS {
        cave.set((dx, 1 + dy, dz), 0);
    }
    check(cave, at((0, 1, 0)), 4);
    let mut above = Occlusion::new(0, 24);
    random_world(&mut above, 7, 900, 6);
    check(above, DVec3::new(3.3, 400.0, -5.6), 6);
    assert!(
        clamped > 0 && band[0] > 0 && band[1] > 0,
        "{clamped} clamped, the band above hidden and seen {band:?}"
    );
}

/// Whenever an answer of `contains` changes, so does the generation: over
/// 300 random steps, each a section set (open, closed or anything between),
/// a column removed, the camera moved to another section (now and then
/// above or below the world), or now and then another world, after a
/// `clear` or a teleport that drops every column (`remove_columns`), each
/// followed by a frame's `update` or a settled search; then an update from
/// a section the graph never reached. The exported graph answers as
/// `contains` after every step.
#[test]
fn every_change_of_the_answers_changes_the_generation() {
    let mut seed = 0x0bad_5eed_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let (spread, radius, around) = (5, 3, (0, 0, 0));
    let offset = DVec3::new(3.3, -5.7, 1.9);
    let mut o = Occlusion::new(0, 24);
    random_world(&mut o, 0x2545_f491, 400, spread);
    let mut camera = at((0, 2, 0)) + offset;
    o.settle(camera, radius);
    let mut last = (o.generation(), answers(&o, around, spread + radius));
    let mut changed = 0;
    for step in 0..300 {
        let r = next();
        let key = (
            (r % 11) as i32 - spread,
            ((r >> 8) % 7) as i32,
            ((r >> 16) % 11) as i32 - spread,
        );
        match r >> 24 & 15 {
            0..=7 => {
                let vis = match r >> 28 & 3 {
                    0 => ALL_VISIBLE,
                    1 => 0,
                    _ => next() & ALL_VISIBLE,
                };
                o.set(key, vis);
            }
            8 | 9 => o.remove_column(key.0, key.2),
            10..=13 => {
                let y = match r >> 32 & 7 {
                    0 => 30,
                    1 => -3,
                    _ => key.1,
                };
                camera = at((key.0, y, key.2)) + offset;
            }
            _ if r >> 36 & 7 == 0 => {
                o.clear();
                random_world(&mut o, next(), 400, spread);
            }
            _ if r >> 36 & 7 == 1 => {
                // A teleport: every column goes at once.
                let all: FxHashSet<_> = o.columns.map.keys().copied().collect();
                o.remove_columns(&all);
                random_world(&mut o, next(), 400, spread);
            }
            _ => {}
        }
        if r >> 40 & 1 == 0 {
            o.update(camera, radius);
        } else {
            o.settle(camera, radius);
        }
        let now = (o.generation(), answers(&o, around, spread + radius));
        if now.1 != last.1 {
            assert_ne!(
                now.0, last.0,
                "step {step}: answers changed, the generation not"
            );
            changed += 1;
        }
        last = now;
    }
    assert!(changed >= 50, "{changed} steps changed answers");
    // Walled in, then past the wall: the graph never reached the camera's
    // section, and goes until the next search lands. (The first settle may
    // land a search that started before the wall.)
    for (dx, dy, dz) in STEPS {
        o.set((dx, 1 + dy, dz), 0);
    }
    o.settle(at((0, 1, 0)), radius);
    o.settle(at((0, 1, 0)), radius);
    let walled = (o.generation(), answers(&o, around, spread + radius));
    assert!(!o.contains((3, 1, 0)), "past the wall");
    o.update(at((3, 1, 0)), radius);
    assert_ne!(answers(&o, around, spread + radius), walled.1);
    assert_ne!(o.generation(), walled.0, "the graph went");
}

#[test]
fn edit_queue_stays_bounded_without_searches() {
    let mut o = Occlusion::new(0, 4);
    // Fill the queue, trigger the resend, then edit once while it is pending.
    // Further edits repeat that last branch without exercising a new boundary.
    let total = EDIT_CAP + 2;
    for i in 0..total {
        o.set(((i % 700) as i32, 1, (i / 700) as i32), 1 + (i as u64 % 5));
    }
    assert!(o.edits.len() <= 1 << 16, "{} edits queued", o.edits.len());
    assert!(
        o.changed.len() <= 1 << 16,
        "{} changed queued",
        o.changed.len()
    );

    let mut fresh = Occlusion::new(0, 4);
    for (&(x, z), column) in &o.columns.map {
        for (y, &vis) in column.iter().enumerate() {
            // Missing sections in the fresh world already connect every face.
            if vis != ALL_VISIBLE {
                fresh.set((x, y as i32, z), vis);
            }
        }
    }
    let camera = at((0, 1, 0));
    for _ in 0..2 {
        o.settle(camera, 6);
        fresh.settle(camera, 6);
    }
    assert_eq!(
        reached(&o),
        reached(&fresh),
        "the worker saw the same world"
    );
}
