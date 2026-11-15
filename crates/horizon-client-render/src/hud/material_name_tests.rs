use super::*;

#[test]
fn ordinary_suffix_names_remain_addressable_beside_their_base() {
    let root = std::env::temp_dir().join(format!("horizon-material-names-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let names = [
        "minecraft/textures/gui/icon",
        "minecraft/textures/gui/icon_n",
        "minecraft/textures/gui/icon_s",
        "minecraft/textures/entity/figure",
        "minecraft/textures/entity/figure_n",
        "minecraft/textures/entity/figure_s",
        "minecraft/textures/particle/spark",
        "minecraft/textures/particle/spark_n",
        "minecraft/textures/particle/spark_s",
        "example/textures/particle/dust",
        "example/textures/particle/dust_n",
        "example/textures/particle/dust_s",
        "minecraft/textures/gui/lone_n",
    ];
    for (i, name) in names.iter().enumerate() {
        let path = root.join(format!("assets/{name}.png"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[i as u8, 22, 33, 255].repeat(4))
            .unwrap();
    }
    // No material-map demand or discovery is needed for these ordinary image names.
    let atlas = HudAtlas::pack(&root, &root, &["gui", "entity", "particle"], false, false).unwrap();
    for (i, name) in names.iter().enumerate() {
        let (ns, path) = name.split_once("/textures/").unwrap();
        let name = if ns == "minecraft" {
            path.to_string()
        } else {
            format!("{ns}:{path}")
        };
        let rect = atlas
            .sprites
            .get(&name)
            .unwrap_or_else(|| panic!("missing ordinary sprite {name}"));
        let x = (rect.u0 * atlas.width as f32).round() as u32;
        let y = (rect.v0 * atlas.height as f32).round() as u32;
        let at = ((y * atlas.width + x) * 4) as usize;
        assert_eq!(&atlas.pixels[at..at + 4], &[i as u8, 22, 33, 255], "{name}");
    }
    // Shared world-sprite lookup already accepts a suffix name directly.
    let mut shared = crate::sprites::Sprites::default();
    assert!(
        shared
            .try_insert(&root, "minecraft:particle/spark_n")
            .is_ok()
    );
    fs::remove_dir_all(root).unwrap();
}
