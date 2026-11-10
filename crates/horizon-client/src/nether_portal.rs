//! `PortalProcessor` and an entity's portal cooldown
//! (`Entity.setAsInsidePortal` / `handlePortal`) for nether portals.
use horizon_core::position::BlockPos;

/// `Entity.getDimensionChangingDelay`: the portal cooldown of an entity that
/// went through a portal.
pub const ENTITY_DELAY: i32 = 300;
/// `VehicleEntity.getDimensionChangingDelay`: a boat, a raft or a minecart.
pub const VEHICLE_DELAY: i32 = 10;
/// `Projectile.getDimensionChangingDelay`.
pub const PROJECTILE_DELAY: i32 = 2;

/// `getDimensionChangingDelay` of the entity type `id` (`minecraft:oak_boat`),
/// for an entity known only by its saved compound: the vehicles
/// (`VehicleEntity`) and the projectiles (`Projectile`) have their own, the
/// rest `Entity`'s.
pub fn delay_of(id: &str) -> i32 {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    let vehicle = name.ends_with("boat") || name.ends_with("raft") || name.ends_with("minecart");
    let projectile = matches!(
        name,
        "arrow"
            | "spectral_arrow"
            | "trident"
            | "snowball"
            | "egg"
            | "ender_pearl"
            | "experience_bottle"
            | "splash_potion"
            | "lingering_potion"
            | "fireball"
            | "small_fireball"
            | "dragon_fireball"
            | "wither_skull"
            | "wind_charge"
            | "breeze_wind_charge"
            | "shulker_bullet"
            | "llama_spit"
            | "firework_rocket"
            | "fishing_bobber"
    );
    if vehicle {
        VEHICLE_DELAY
    } else if projectile {
        PROJECTILE_DELAY
    } else {
        ENTITY_DELAY
    }
}

/// `PortalProcessor` for a nether portal.
#[derive(Clone, Copy, Debug)]
pub struct Process {
    pub entry: BlockPos,
    pub time: i32,
    pub inside: bool,
}
impl Process {
    /// `processPortalTeleportation`: whether the entity goes through now.
    pub fn process(&mut self, allowed: bool, transition_time: i32) -> bool {
        if !self.inside {
            // decayTick
            self.time = (self.time - 4).max(0);
            return false;
        }
        self.inside = false;
        if !allowed {
            return false;
        }
        let time = self.time;
        self.time += 1;
        time >= transition_time
    }
}

/// An entity's `portalCooldown` and `portalProcess`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PortalState {
    pub cooldown: i32,
    pub process: Option<Process>,
}
impl PortalState {
    /// `Entity.setAsInsidePortal` for a nether portal block at `pos`.
    pub fn set_inside(&mut self, pos: BlockPos, delay: i32) {
        if self.cooldown > 0 {
            self.cooldown = delay;
            return;
        }
        match &mut self.process {
            None => {
                self.process = Some(Process {
                    entry: pos,
                    time: 0,
                    inside: true,
                })
            }
            Some(p) if !p.inside => {
                p.entry = pos;
                p.inside = true;
            }
            Some(_) => {}
        }
    }
    /// `Entity.handlePortal`: the entry block when the entity goes through
    /// (its cooldown then set to `delay`).
    pub fn handle(&mut self, allowed: bool, transition_time: i32, delay: i32) -> Option<BlockPos> {
        // processPortalCooldown
        if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        let process = self.process.as_mut()?;
        if process.process(allowed, transition_time) {
            self.cooldown = delay;
            return Some(process.entry);
        }
        // hasExpired
        if process.time <= 0 {
            self.process = None;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vanilla's three `getDimensionChangingDelay`s: `VehicleEntity` 10,
    /// `Projectile` 2, `Entity` 300.
    #[test]
    fn the_portal_cooldown_depends_on_the_entity_type() {
        for vehicle in [
            "minecraft:oak_boat",
            "minecraft:oak_chest_boat",
            "minecraft:bamboo_raft",
            "minecraft:bamboo_chest_raft",
            "minecraft:minecart",
            "minecraft:tnt_minecart",
        ] {
            assert_eq!(delay_of(vehicle), 10, "{vehicle}");
        }
        for projectile in [
            "minecraft:arrow",
            "minecraft:trident",
            "minecraft:ender_pearl",
            "minecraft:firework_rocket",
            "minecraft:wind_charge",
        ] {
            assert_eq!(delay_of(projectile), 2, "{projectile}");
        }
        for other in [
            "minecraft:zombie",
            "minecraft:item",
            "minecraft:tnt",
            "minecraft:falling_block",
            "minecraft:eye_of_ender",
        ] {
            assert_eq!(delay_of(other), 300, "{other}");
        }
    }

    /// `Entity.handlePortal` sets the entity's own delay when it goes
    /// through, and `setAsInsidePortal` restarts it at that delay.
    #[test]
    fn going_through_starts_the_cooldown_at_the_entitys_delay() {
        let portal = BlockPos::new(0, 64, 0);
        let mut boat = PortalState::default();
        boat.set_inside(portal, VEHICLE_DELAY);
        assert_eq!(boat.handle(true, 0, VEHICLE_DELAY), Some(portal));
        assert_eq!(boat.cooldown, 10);
        // processPortalCooldown, then standing in the exit portal restarts it.
        assert_eq!(boat.handle(true, 0, VEHICLE_DELAY), None);
        assert_eq!(boat.cooldown, 9);
        boat.set_inside(portal, VEHICLE_DELAY);
        assert_eq!(boat.cooldown, 10);
    }
}
