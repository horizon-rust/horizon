//! Pure player selection shared by the serial coordinator and region workers.
use super::{Mob, PlayerView, Species, Who};
use horizon_core::geometry::Vec3;

/// The guests mobs can see this tick, by connection.
pub type GuestViews = Vec<(u32, PlayerView)>;

/// The player a mob sees this tick: `None` for the host, else a guest.
/// A mob that targets a guest, or that a guest hurt in the last 100 ticks
/// (`LivingEntity.getLastHurtByMob`), keeps them while they are still a
/// valid target; otherwise it sees the nearest player
/// (`TargetingConditions`: alive, neither creative nor spectator), or the
/// nearest one that is not a spectator if none qualifies.
pub fn player_for(
    m: &Mob,
    host: Option<&PlayerView>,
    guests: &[(u32, PlayerView)],
    bound: Option<u32>,
) -> Option<u32> {
    player_for_with(m, host, guests, bound, |at, targeting, bound| {
        choose(at, targeting, host, guests, bound)
    })
}
fn player_for_with(
    m: &Mob,
    host: Option<&PlayerView>,
    guests: &[(u32, PlayerView)],
    bound: Option<u32>,
    pick: impl Fn(Vec3, bool, Option<u32>) -> Option<u32>,
) -> Option<u32> {
    // A tame cat reads its owner wherever they are while they sleep, or just
    // woke up (`CatRelaxOnOwnerGoal` uses `getOwner`, not the nearest player).
    if m.species() == Species::Cat
        && m.extra.tame
        && let Some(owner) = m.extra.owner
    {
        let asleep = |v: &PlayerView| v.uuid == owner && (v.sleeping || v.sleep_timer > 0);
        if host.is_some_and(asleep) {
            return None;
        }
        if let Some((conn, _)) = guests.iter().find(|(_, v)| asleep(v)) {
            return Some(*conn);
        }
    }
    // An iron golem's anger is for the player who hit it (`NeutralMob` keeps
    // the angry-at UUID), and outlasts the 100 ticks of `getLastHurtByMob`:
    // while it lasts, the golem keeps that player, not whoever is nearest.
    let angry = m.species() == Species::IronGolem && m.extra.anger > 0;
    let targeting = m.ai.target == Some(Who::Player)
        || angry
        || m.ai.last_hurt_by == Some(Who::Player) && m.age as i32 - m.ai.last_hurt_by_time <= 100;
    // A tame wolf, cat or parrot sees its owner (`TamableAnimal.getOwner`), while they
    // are in play, whoever is nearer. Not while it targets or was just hurt by
    // a player: then `Who::Player` is that attacker, whoever it is, and the
    // owner goals ignore it until the wolf lets go.
    if !targeting
        && matches!(m.species(), Species::Wolf | Species::Cat | Species::Parrot)
        && m.extra.tame
        && let Some(owner) = m.extra.owner
    {
        if host.is_some_and(|v| v.alive && v.uuid == owner) {
            return None;
        }
        if let Some((conn, _)) = guests.iter().find(|(_, v)| v.alive && v.uuid == owner) {
            return Some(*conn);
        }
    }
    // A mob on a player's lead follows and measures its rope against the
    // player who holds it, not whoever is nearest or in survival mode.
    if m.extra.leash == Some(horizon_client::mobs::leash::Holder::Player)
        && let Some(holder) = m.extra.leash_player
    {
        if host.is_some_and(|v| v.uuid == holder) {
            return None;
        }
        if let Some((conn, _)) = guests.iter().find(|(_, v)| v.uuid == holder) {
            return Some(*conn);
        }
    }
    pick(m.position, targeting, bound)
}

#[doc(hidden)]
pub fn choose(
    at: Vec3,
    targeting: bool,
    host: Option<&PlayerView>,
    guests: &[(u32, PlayerView)],
    bound: Option<u32>,
) -> Option<u32> {
    let targetable = |v: &PlayerView| v.alive && !v.creative && !v.spectator;
    if targeting
        && let Some(conn) = bound
        && guests.iter().any(|(c, v)| *c == conn && targetable(v))
    {
        return Some(conn);
    }
    let d = |v: &PlayerView| at.distance_squared(v.position);
    let nearest = |ok: &dyn Fn(&PlayerView) -> bool| {
        let mut best = host.filter(|v| ok(v)).map(|v| (None, d(v)));
        for (conn, v) in guests.iter().filter(|(_, v)| ok(v)) {
            if best.is_none_or(|(_, b)| d(v) < b) {
                best = Some((Some(*conn), d(v)));
            }
        }
        best
    };
    nearest(&targetable)
        .or_else(|| nearest(&|v: &PlayerView| v.alive && !v.spectator))
        .and_then(|(conn, _)| conn)
}

/// Immutable per-tick player indexes. Region-local players seed the nearest
/// search; the complete tree is still searched with exact distance bounds, so
/// a target or owner outside a region is never hidden by a boundary margin.
pub struct PlayerDirectory<'a> {
    host: Option<&'a PlayerView>,
    guests: &'a [(u32, PlayerView)],
    targetable: PlayerTree,
    nonspectators: PlayerTree,
    local: crate::fast_hash::FxHashMap<usize, Vec<usize>>,
    connections: crate::fast_hash::FxHashMap<u32, usize>,
    targetable_connections: crate::fast_hash::FxHashSet<u32>,
    finite: bool,
}
#[derive(Default)]
struct PlayerTree {
    nodes: Vec<PlayerNode>,
}
struct PlayerNode {
    slot: usize,
    axis: usize,
    left: Option<usize>,
    right: Option<usize>,
}
fn coordinate(position: Vec3, axis: usize) -> f64 {
    match axis {
        0 => position.x,
        1 => position.y,
        _ => position.z,
    }
}
fn can_target(player: &PlayerView) -> bool {
    player.alive && !player.creative && !player.spectator
}
impl PlayerTree {
    fn new(mut slots: Vec<usize>, position: impl Fn(usize) -> Vec3) -> Self {
        fn build(
            nodes: &mut Vec<PlayerNode>,
            slots: &mut [usize],
            depth: usize,
            position: &impl Fn(usize) -> Vec3,
        ) -> Option<usize> {
            if slots.is_empty() {
                return None;
            }
            let axis = depth % 3;
            slots.sort_unstable_by(|&a, &b| {
                coordinate(position(a), axis)
                    .total_cmp(&coordinate(position(b), axis))
                    .then(a.cmp(&b))
            });
            let middle = slots.len() / 2;
            let index = nodes.len();
            nodes.push(PlayerNode {
                slot: slots[middle],
                axis,
                left: None,
                right: None,
            });
            let (left, rest) = slots.split_at_mut(middle);
            nodes[index].left = build(nodes, left, depth + 1, position);
            nodes[index].right = build(nodes, &mut rest[1..], depth + 1, position);
            Some(index)
        }
        let mut nodes = Vec::with_capacity(slots.len());
        build(&mut nodes, &mut slots, 0, &position);
        Self { nodes }
    }
    fn nearest(
        &self,
        at: Vec3,
        position: impl Fn(usize) -> Vec3,
        mut best: Option<(usize, f64)>,
    ) -> Option<usize> {
        fn visit(
            tree: &PlayerTree,
            index: usize,
            at: Vec3,
            position: &impl Fn(usize) -> Vec3,
            best: &mut Option<(usize, f64)>,
        ) {
            let node = &tree.nodes[index];
            let point = position(node.slot);
            let distance = at.distance_squared(point);
            if best.is_none_or(|(slot, old)| distance < old || distance == old && node.slot < slot)
            {
                *best = Some((node.slot, distance));
            }
            let delta = coordinate(at, node.axis) - coordinate(point, node.axis);
            let (near, far) = if delta < 0.0 {
                (node.left, node.right)
            } else {
                (node.right, node.left)
            };
            if let Some(near) = near {
                visit(tree, near, at, position, best);
            }
            // Equality must be visited: an equally distant earlier player wins.
            if let Some(far) = far
                && best.is_none_or(|(_, distance)| delta * delta <= distance)
            {
                visit(tree, far, at, position, best);
            }
        }
        if !self.nodes.is_empty() {
            visit(self, 0, at, &position, &mut best);
        }
        best.map(|(slot, _)| slot)
    }
}
impl<'a> PlayerDirectory<'a> {
    pub fn new(host: Option<&'a PlayerView>, guests: &'a [(u32, PlayerView)]) -> Self {
        let mut directory = Self {
            host,
            guests,
            targetable: Default::default(),
            nonspectators: Default::default(),
            local: Default::default(),
            connections: Default::default(),
            targetable_connections: Default::default(),
            finite: true,
        };
        let mut targetable = Vec::new();
        let mut nonspectators = Vec::new();
        for slot in 0..directory.len() {
            let view = directory.view(slot);
            let finite = view.position.x.is_finite()
                && view.position.y.is_finite()
                && view.position.z.is_finite();
            let target = can_target(view);
            let nonspectator = view.alive && !view.spectator;
            directory.finite &= finite;
            if target {
                targetable.push(slot);
            }
            if nonspectator {
                nonspectators.push(slot);
            }
        }
        for (index, (conn, view)) in guests.iter().enumerate() {
            directory.connections.entry(*conn).or_insert(index);
            if can_target(view) {
                directory.targetable_connections.insert(*conn);
            }
        }
        directory.targetable = PlayerTree::new(targetable, |slot| directory.view(slot).position);
        directory.nonspectators =
            PlayerTree::new(nonspectators, |slot| directory.view(slot).position);
        directory
    }
    pub fn with_owners(mut self, owner: impl Fn(Vec3) -> Option<usize>) -> Self {
        for slot in 0..self.len() {
            if let Some(owner) = owner(self.view(slot).position) {
                self.local.entry(owner).or_default().push(slot);
            }
        }
        self
    }
    fn len(&self) -> usize {
        self.guests.len() + usize::from(self.host.is_some())
    }
    fn view(&self, slot: usize) -> &PlayerView {
        if slot == 0
            && let Some(host) = self.host
        {
            return host;
        }
        &self.guests[slot - usize::from(self.host.is_some())].1
    }
    pub fn guest(&self, conn: u32) -> Option<&PlayerView> {
        self.connections
            .get(&conn)
            .map(|&index| &self.guests[index].1)
    }
    /// Where a guest stands in the list this directory was built from.
    pub fn guest_slot(&self, conn: u32) -> Option<usize> {
        self.connections.get(&conn).copied()
    }
    fn connection(&self, slot: usize) -> Option<u32> {
        if self.host.is_some() && slot == 0 {
            None
        } else {
            Some(self.guests[slot - usize::from(self.host.is_some())].0)
        }
    }
    fn choose(&self, owner: usize, at: Vec3, targeting: bool, bound: Option<u32>) -> Option<u32> {
        if !self.finite || !at.x.is_finite() || !at.y.is_finite() || !at.z.is_finite() {
            return choose(at, targeting, self.host, self.guests, bound);
        }
        if targeting
            && let Some(bound) = bound
            && self.targetable_connections.contains(&bound)
        {
            return Some(bound);
        }
        let nearest = |tree: &PlayerTree, targetable: bool| {
            let mut seed = None;
            if let Some(local) = self.local.get(&owner) {
                for &slot in local {
                    let view = self.view(slot);
                    if if targetable {
                        can_target(view)
                    } else {
                        view.alive && !view.spectator
                    } {
                        let distance = at.distance_squared(view.position);
                        if seed.is_none_or(|(old_slot, old)| {
                            distance < old || distance == old && slot < old_slot
                        }) {
                            seed = Some((slot, distance));
                        }
                    }
                }
            }
            tree.nearest(at, |slot| self.view(slot).position, seed)
        };
        nearest(&self.targetable, true)
            .or_else(|| nearest(&self.nonspectators, false))
            .and_then(|slot| self.connection(slot))
    }
    pub fn player_for(&self, owner: usize, mob: &Mob, bound: Option<u32>) -> Option<u32> {
        player_for_with(
            mob,
            self.host,
            self.guests,
            bound,
            |at, targeting, bound| self.choose(owner, at, targeting, bound),
        )
    }
    /// A controlling rider is independent of the player selected as an AI target.
    pub fn controlling_movement(&self, mob: &Mob) -> Option<Vec3> {
        if !super::equine::controlled(mob) {
            return None;
        }
        self.host
            .filter(|player| player.riding_vehicle == Some(mob.id))
            .or_else(|| {
                self.guests
                    .iter()
                    .map(|(_, player)| player)
                    .find(|player| player.riding_vehicle == Some(mob.id))
            })
            .map(|player| player.velocity)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{sensor_scan_tests::kind, world::test_player};
    use super::*;

    #[test]
    fn controlling_rider_is_found_across_regions_with_host_precedence() {
        let mut kind = kind();
        std::sync::Arc::make_mut(&mut kind).species = Species::Horse;
        let mut mount = Mob::new(42, kind, Vec3::ZERO, 0.0);
        mount.rider = Some(Default::default());
        mount.extra.saddled = true;
        let mut host = test_player(Vec3::ZERO);
        host.velocity = Vec3::new(0.1, 0.0, 0.0);
        let near = test_player(Vec3::ZERO);
        let mut rider = test_player(Vec3::new(8192.0, 0.0, 0.0));
        rider.riding_vehicle = Some(mount.id);
        rider.velocity = Vec3::new(0.3, 0.0, 0.0);
        let guests = [(1, near), (2, rider)];
        assert_eq!(
            PlayerDirectory::new(Some(&host), &guests).controlling_movement(&mount),
            Some(Vec3::new(0.3, 0.0, 0.0))
        );
        host.riding_vehicle = Some(mount.id);
        assert_eq!(
            PlayerDirectory::new(Some(&host), &guests).controlling_movement(&mount),
            Some(Vec3::new(0.1, 0.0, 0.0))
        );
        mount.extra.saddled = false;
        assert!(
            PlayerDirectory::new(Some(&host), &guests)
                .controlling_movement(&mount)
                .is_none()
        );
    }

    #[test]
    fn indexed_selection_matches_ordered_scan_across_regions_and_modes() {
        let mut host = test_player(Vec3::ZERO);
        let guests: Vec<_> = (0..100)
            .map(|slot| {
                let mut view = test_player(Vec3::new(
                    ((slot * 37) % 101) as f64 * 16.0 - 800.0,
                    (slot % 7) as f64 * 4.0,
                    ((slot * 71) % 103) as f64 * 16.0 - 800.0,
                ));
                view.alive = slot % 11 != 0;
                view.creative = slot % 3 == 0;
                view.spectator = slot % 5 == 0;
                (slot + 1, view)
            })
            .collect();
        for host_mode in 0..4 {
            host.creative = host_mode == 1;
            host.spectator = host_mode == 2;
            host.alive = host_mode != 3;
            for host in [None, Some(&host)] {
                let directory = PlayerDirectory::new(host, &guests)
                    .with_owners(|at| Some(usize::from(at.x >= 0.0)));
                for sample in 0..600 {
                    let at = Vec3::new(
                        ((sample * 29) % 997) as f64 * 2.0 - 1000.0,
                        (sample % 9) as f64,
                        ((sample * 97) % 991) as f64 * 2.0 - 1000.0,
                    );
                    for targeting in [false, true] {
                        for bound in [None, Some((sample % 103) as u32)] {
                            assert_eq!(
                                directory.choose(usize::from(at.x >= 0.0), at, targeting, bound),
                                choose(at, targeting, host, &guests, bound),
                                "sample={sample} targeting={targeting} bound={bound:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ties_dead_players_and_nonfinite_positions_keep_serial_selection() {
        let mut host = test_player(Vec3::new(-1.0, 0.0, 0.0));
        let mut guests = vec![
            (7, test_player(Vec3::new(1.0, 0.0, 0.0))),
            (8, test_player(Vec3::new(0.0, 0.0, 1.0))),
        ];
        let at = Vec3::ZERO;
        fn indexed(
            host: Option<&PlayerView>,
            guests: &[(u32, PlayerView)],
            at: Vec3,
        ) -> Option<u32> {
            PlayerDirectory::new(host, guests)
                .with_owners(|_| Some(99))
                .choose(99, at, false, None)
        }
        assert_eq!(indexed(Some(&host), &guests, at), None);
        assert_eq!(indexed(None, &guests, at), Some(7));
        host.alive = false;
        guests[0].1.alive = false;
        guests[1].1.creative = true;
        assert_eq!(indexed(Some(&host), &guests, at), Some(8));
        for unusual in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            guests[0].1.position.x = unusual;
            for at in [at, Vec3::new(unusual, 0.0, 0.0)] {
                assert_eq!(
                    indexed(Some(&host), &guests, at),
                    choose(at, false, Some(&host), &guests, None)
                );
            }
        }
    }

    #[test]
    fn owners_leashes_and_bound_targets_can_cross_region_ownership() {
        let mut host = test_player(Vec3::ZERO);
        host.uuid = [1; 4];
        let mut guest = test_player(Vec3::new(10000.0, 0.0, 0.0));
        guest.uuid = [2; 4];
        guest.sleeping = true;
        let guests = vec![(9, guest)];
        let directory = PlayerDirectory::new(Some(&host), &guests)
            .with_owners(|at| Some(usize::from(at.x > 0.0)));
        let mut mob = Mob::new(1, kind(), Vec3::ZERO, 0.0);
        for species in [
            Species::Cow,
            Species::Cat,
            Species::Wolf,
            Species::IronGolem,
        ] {
            let mut mob_kind = kind();
            std::sync::Arc::get_mut(&mut mob_kind).unwrap().species = species;
            mob.kind = mob_kind;
            for target in [None, Some(Who::Player)] {
                mob.ai.target = target;
                for leash in [false, true] {
                    mob.extra.tame = true;
                    mob.extra.owner = Some([2; 4]);
                    mob.extra.leash = leash.then_some(super::super::leash::Holder::Player);
                    mob.extra.leash_player = Some([2; 4]);
                    assert_eq!(
                        directory.player_for(0, &mob, Some(9)),
                        player_for(&mob, Some(&host), &guests, Some(9))
                    );
                }
            }
        }
        // The target can be valid in a later duplicate entry, while resolving
        // its view still uses the first connection entry, as the coordinator did.
        let mut duplicate = guests.clone();
        duplicate[0].1.creative = true;
        duplicate.push(guests[0].clone());
        let directory = PlayerDirectory::new(None, &duplicate);
        assert_eq!(directory.choose(0, Vec3::ZERO, true, Some(9)), Some(9));
        assert!(directory.guest(9).unwrap().creative);
    }
}
