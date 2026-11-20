//! A higher-resolution pack's sprites in the HUD atlas: the packed pixels keep
//! the pack's resolution while a `Rect` carries the default resources' logical
//! size, so the GUI draws in the units its code uses whatever the pack
//! (issue #19022's Faithful 32x/64x inventory sprites).
use super::*;
use horizon_assets::resource_source::Resource;

/// A 16x16 default replaced by a 32x32 pack texture: the sprite's logical size
/// stays the default's, and the atlas holds the pack's own pixels.
#[test]
fn a_32x_pack_texture_keeps_the_default_sprite_size() {
    let defaults = index(vec![("item/test", 16)]);
    let overlay = [(
        "assets/minecraft/textures/item/test.png".to_owned(),
        Resource::bytes(grey_png(32, 32)),
    )]
    .into_iter()
    .collect();
    let root = ResourceSource::layered(overlay, defaults.clone());
    let atlas = HudAtlas::pack(&root, &defaults, &["item"], false, false).unwrap();
    let rect = atlas.sprites["item/test"];
    let (tw, th) = rect.texels((atlas.width, atlas.height));
    assert_eq!(
        (rect.w, rect.h),
        (16, 16),
        "the sprite's logical size must stay the default's, not the pack's"
    );
    assert_eq!(
        (tw, th),
        (32, 32),
        "the atlas must hold the pack's own 32x32 pixels"
    );
}

/// The production callers pass the resolved overlay as its own `defaults`
/// (`build_over_with_image(&source, &source, ..)`): the sprite's logical size
/// must then still come from the layer the pack resolved over, not from the
/// root — the root answers a replaced texture with the pack's own file, whose
/// size is no logical size at all (#19022).
#[test]
fn a_pack_over_its_own_defaults_keeps_the_default_sprite_size() {
    let base = index(vec![("item/test", 16)]);
    let overlay = [(
        "assets/minecraft/textures/item/test.png".to_owned(),
        Resource::bytes(grey_png(32, 32)),
    )]
    .into_iter()
    .collect();
    let root = ResourceSource::layered(overlay, base);
    let atlas = HudAtlas::pack(&root, &root, &["item"], false, false).unwrap();
    let rect = atlas.sprites["item/test"];
    assert_eq!(
        (rect.w, rect.h),
        (16, 16),
        "the sprite's logical size must be the layer the pack resolved over"
    );
}

/// An index with one grey PNG per entry.
fn index(files: Vec<(&str, u32)>) -> ResourceSource {
    let map = files
        .into_iter()
        .map(|(name, side)| {
            let path = format!("assets/minecraft/textures/{name}.png");
            (path, Resource::bytes(grey_png(side, side)))
        })
        .collect();
    ResourceSource::indexed(map)
}
