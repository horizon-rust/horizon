//! Horizon's look, given to the engine's extension points at startup (docs/layers.md): the
//! windows' icon, and the bundled faces for text drawn before any resource pack has a font (the
//! boot screen, the missing-resources window). The menus vanilla doesn't have keep the engine's
//! own look (`menu_style`'s `NEUTRAL`: vanilla's surfaces, greys and the pack's font).
//! Its own module so it can leave the engine crate as a pure file move, for the official
//! executable to call.
use horizon_client::menu_style::{self, Fonts};
use horizon_theme as theme;

/// Give the engine Horizon's look; call it once, before the first window opens.
pub(crate) fn give() {
    horizon_client::window::set_icon_png(theme::ICON_PNG);
    menu_style::install_bundled_fonts(Fonts {
        regular: theme::fonts::GEIST_REGULAR,
        medium: theme::fonts::GEIST_MEDIUM,
        semibold: theme::fonts::GEIST_SEMIBOLD,
        mono: theme::fonts::GEIST_MONO,
    });
}
