//! Selective enable and the saved bindings each new store needs.
pub(super) fn sync_enabled<'a>(
    mods: &mut horizon_mods::Mods,
    enabled: impl Fn(&str) -> bool,
    reserved: impl Fn(&str) -> bool,
    chosen: impl Fn(&str) -> Vec<(&'a str, &'a str)>,
) {
    mods.sync_enabled(enabled, reserved);
    mods.set_bindings(chosen);
}
