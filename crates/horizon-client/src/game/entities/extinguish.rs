//! Powder snow and water cauldrons putting a burning player out: the block
//! change they make is gated on `mayInteract`, and the move that put the fire
//! out plays the extinguish hiss (`Entity.playEntityOnFireExtinguishedSound`).
//! The `mayInteract` answers live here too: the acting player's, and the one a
//! projectile's owner gets (`Projectile.mayInteract`).
use super::Game;
use horizon_client::motion::MotionEvent;
use horizon_core::position::BlockPos;

impl Game<'_> {
    /// `ServerLevel.mayInteract` for the player whose turn it is: a player
    /// under spawn protection or outside the world border may not change the
    /// block (`ServerPlayer.mayInteract`). The fire goes out either way.
    pub(super) fn may_interact_at(&self, pos: BlockPos) -> bool {
        self.border
            .geometry()
            .is_within_bounds(f64::from(pos.x), f64::from(pos.z), 0.0)
            && !self.lan_acting_spawn_protection.is_some_and(|radius| {
                self.spawn_protects(radius, self.dim.id(), [pos.x, pos.y, pos.z])
            })
    }
    /// `ServerLevel.mayInteract` at a block a ray met, for the player whose
    /// turn it is (`BottleItem.use`).
    pub(super) fn may_interact_block(&self, pos: [i32; 3]) -> bool {
        self.may_interact_at(BlockPos::new(pos[0], pos[1], pos[2]))
    }
    /// `Entity.applyEffectsFromBlocks` along a server player's moves: the
    /// blocks that put the fire out (`PowderSnowBlock.entityInside`'s
    /// runBefore EXTINGUISH, `LayeredCauldronBlock.entityInside`) change
    /// first, then the hiss plays once, as vanilla plays it when the move
    /// left `wasOnFire && !isOnFire()`.
    pub(super) fn block_extinguish_player(&mut self, events: &[MotionEvent]) {
        let mut out = false;
        for event in events {
            match event {
                MotionEvent::CauldronExtinguish { pos } => self.cauldron_extinguish(*pos),
                MotionEvent::PowderSnowMelt { pos } => self.powder_snow_melt(*pos),
                _ => continue,
            }
            out = true;
        }
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if out && let Some(at) = at {
            self.extinguish_sound(at);
        }
    }
    /// `LayeredCauldronBlock.handleEntityOnFireInside` for a burning player
    /// in the cauldron at `pos` (`mayInteract`): the fill level drops
    /// (`lowerFillLevel`, a `block_change` with no source). A server's
    /// cauldron is the server's.
    pub(super) fn cauldron_extinguish(&mut self, pos: BlockPos) {
        if self.remote.is_some() || !self.may_interact_at(pos) {
            return;
        }
        let at = [pos.x, pos.y, pos.z];
        let registry = self.pack.assets.registry;
        let Some(old) = registry
            .state(self.world.block(at[0], at[1], at[2]))
            .filter(|s| {
                matches!(
                    s.name.as_str(),
                    "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron"
                )
            })
            .cloned()
        else {
            return;
        };
        let Ok(new) = registry.resolve(&horizon_client::cauldron::extinguished(&old)) else {
            return;
        };
        let level = |state: &horizon_world::chunk::BlockState| {
            if matches!(state.name.as_str(), "minecraft:cauldron") {
                0
            } else if state.name == "minecraft:lava_cauldron" {
                3
            } else {
                state
                    .properties
                    .get("level")
                    .and_then(|level| level.parse().ok())
                    .unwrap_or(1)
            }
        };
        let Some(new_state) = registry.state(new).cloned() else {
            return;
        };
        if !self.native_cauldron_level_change_allowed(
            pos,
            level(&old),
            level(&new_state),
            "extinguishing",
        ) {
            return;
        }
        // The hook had `&mut self`: the block must still be the one it judged.
        let unchanged = registry
            .state(self.world.block(at[0], at[1], at[2]))
            .is_some_and(|now| now.name == old.name && now.properties == old.properties);
        if !unchanged {
            return;
        }
        if self.level_set(at, new).is_ok() {
            self.post_block_game_event("minecraft:block_change", at, None, Some(new));
        }
    }
}

impl horizon_client::arrows::OwnerLookup for Game<'_> {
    fn non_player_entity(&self, uuid: [i32; 4]) -> bool {
        let found = self.target_by_uuid(horizon_client::lan::playerdata::ints_uuid(uuid));
        !matches!(
            found,
            None | Some(super::commands::Target::Player | super::commands::Target::Guest(_))
        )
    }
    /// `Projectile.mayInteract` for a player's arrow: `ServerPlayer.mayInteract`
    /// is `ServerLevel.mayInteract` in the arrow's level for the guest who shot it,
    /// even after that guest travels to another dimension. It refuses a
    /// block inside the spawn protection radius (`DedicatedServer.isUnderSpawnProtection`)
    /// and one outside the world border.
    fn player_may_interact(&self, guest: Option<u32>, pos: BlockPos) -> bool {
        let protected = self.lan.as_ref().is_some_and(|host| {
            host.hub
                .guests
                .iter()
                .find(|g| Some(g.conn) == guest)
                .is_some_and(|g| {
                    self.spawn_protection_for(host, g.uuid)
                        .is_some_and(|radius| {
                            self.spawn_protects(radius, self.dim.id(), [pos.x, pos.y, pos.z])
                        })
                })
        });
        !protected
            && self
                .border
                .geometry()
                .is_within_bounds(f64::from(pos.x), f64::from(pos.z), 0.0)
    }
}

#[cfg(test)]
#[path = "tests/arrow_spawn_protection_tests.rs"]
mod arrow_spawn_protection_tests;
#[cfg(test)]
#[path = "tests/bottle_spawn_protection_tests.rs"]
mod bottle_spawn_protection_tests;
#[cfg(test)]
#[path = "extinguish/extinguish_tests.rs"]
mod extinguish_tests;
