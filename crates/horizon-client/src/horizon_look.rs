//! Horizon's look, given to the engine's extension points at startup (docs/layers.md): the
//! windows' icon. The menus vanilla doesn't have keep the engine's own look (`menu_style`'s
//! `NEUTRAL`: vanilla's surfaces, greys and font).
//! Its own module so it can leave the engine crate as a pure file move, for the official
//! executable to call.
use horizon_theme as theme;

/// Give the engine Horizon's look; call it once, before the first window opens.
pub(crate) fn give() {
    horizon_client::window::set_icon_png(theme::ICON_PNG);
}
