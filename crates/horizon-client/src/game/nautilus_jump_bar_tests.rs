//! The jump bar over a ridden nautilus: `Hud.willPrioritizeJumpInfo` and
//! `JumpableVehicleBar` read `AbstractNautilus.getJumpCooldown`, its dash clock.
use super::tests::{mount, server_game};
use horizon_client::items::ItemStack;

/// `Hud.willPrioritizeJumpInfo` and `JumpableVehicleBar`: the nautilus'
/// own 40-tick `getJumpCooldown` (which `DASH` starts) keeps the jump bar
/// up, and once the dash is ready the bar is the rider's again.
#[test]
fn a_nautilus_jump_bar_follows_its_dash_clock() {
    let Some(mut game) = server_game() else {
        return;
    };
    mount(&mut game, "minecraft:nautilus", 1, [0.5, 64.0, 0.5], 1);
    game.mobs[0].set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
    game.riding = Some(1);
    assert!(game.jumpable_mount());
    game.mobs[0].extra.anim_ticks = 40;
    assert!(
        game.mount_jumping(),
        "the bar shows while the dash clock runs"
    );
    game.mobs[0].extra.anim_ticks = 0;
    assert!(
        !game.mount_jumping(),
        "a ready dash leaves the bar to the rider"
    );
}
