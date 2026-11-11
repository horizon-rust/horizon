//! The pack formats' textures in the GUI atlas, end to end: OptiFine random entity variants and
//! their overlays, and custom item textures (moved from the engine's `hud` tests, which no longer
//! reach the formats).

mod tests {
    use crate::hud::{HudAtlas, cit_sprites};
    use crate::hud_test_support::{texel_at, write_strip};
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    /// OptiFine random entity variants outside `textures/` pack under the names
    /// `RandomTextures` gives them; one beside its texture packs once, under its own.
    #[test]
    fn random_entity_variants_pack_under_their_scan_names() {
        crate::pack_formats::install();
        let root =
            std::env::temp_dir().join(format!("horizon-atlas-random-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let random = root.join("assets/minecraft/optifine/random/entity/pig");
        fs::create_dir_all(&random).unwrap();
        write_strip(&random.join("pig2.png"), 64, 64, &[[0, 200, 0, 255]], None);
        let beside = root.join("assets/minecraft/textures/entity/cat");
        fs::create_dir_all(&beside).unwrap();
        fs::write(beside.join("cat.properties"), "textures.1=2").unwrap();
        write_strip(&beside.join("cat.png"), 64, 32, &[[1, 1, 1, 255]], None);
        write_strip(&beside.join("cat2.png"), 64, 32, &[[0, 0, 200, 255]], None);
        let atlas = HudAtlas::build(&root).unwrap();
        let rect = atlas.sprites["minecraft:optifine/random/entity/pig/pig2"];
        assert_eq!(texel_at(&atlas, rect, 0, 0), [0, 200, 0, 255]);
        let rect = atlas.sprites["entity/cat/cat2"];
        assert_eq!(texel_at(&atlas, rect, 0, 0), [0, 0, 200, 255]);
        assert!(atlas.random_textures.get("entity/pig/pig").is_some());
        fs::remove_dir_all(&root).unwrap();
    }
    /// A random variant's emissive overlay packs beside the variant, under its name.
    #[test]
    fn variant_overlays_pack_under_their_names() {
        crate::pack_formats::install();
        let root =
            std::env::temp_dir().join(format!("horizon-atlas-overlay-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let random = root.join("assets/minecraft/optifine/random/entity/zombie");
        fs::create_dir_all(&random).unwrap();
        write_strip(
            &random.join("zombie3.png"),
            64,
            64,
            &[[0, 200, 0, 255]],
            None,
        );
        write_strip(
            &random.join("zombie3_e.png"),
            64,
            64,
            &[[200, 0, 0, 255]],
            None,
        );
        // A variant's blink frame, with its own overlay.
        write_strip(
            &random.join("zombie3_blink.png"),
            64,
            64,
            &[[0, 0, 200, 255]],
            None,
        );
        write_strip(
            &random.join("zombie3_blink_e.png"),
            64,
            64,
            &[[9, 9, 9, 255]],
            None,
        );
        let atlas = HudAtlas::build(&root).unwrap();
        let rect = atlas.sprites["minecraft:optifine/random/entity/zombie/zombie3_blink_e"];
        assert_eq!(texel_at(&atlas, rect, 0, 0), [9, 9, 9, 255]);
        let rect = atlas.sprites["minecraft:optifine/random/entity/zombie/zombie3_e"];
        assert_eq!(texel_at(&atlas, rect, 0, 0), [200, 0, 0, 255]);
        assert_eq!(
            atlas
                .emissive
                .overlay("minecraft:optifine/random/entity/zombie/zombie3"),
            Some("minecraft:optifine/random/entity/zombie/zombie3_e")
        );
        fs::remove_dir_all(&root).unwrap();
    }
    /// A resource root with one plain item texture and these CIT files (relative path, contents).
    fn cit_root(name: &str, files: &[(&str, &str)]) -> PathBuf {
        crate::pack_formats::install();
        let root =
            std::env::temp_dir().join(format!("horizon-atlas-cit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let item = root.join("assets/minecraft/textures/item");
        fs::create_dir_all(&item).unwrap();
        write_strip(&item.join("stick.png"), 16, 16, &[[90, 60, 30, 255]], None);
        let cit = root.join("assets/minecraft/optifine/cit");
        fs::create_dir_all(&cit).unwrap();
        for (rel, text) in files {
            fs::write(cit.join(rel), text).unwrap();
        }
        root
    }
    #[test]
    fn a_cit_item_rules_texture_is_packed_under_its_id() {
        let root = cit_root("plain", &[("a.properties", "items=stick\ntexture=a")]);
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("a.png"), 2, 2, &[[10, 200, 30, 255]], None);
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let rect = atlas.sprites["horizon_cit:minecraft/optifine/cit/a"];
        assert_eq!(rect.texels((atlas.width, atlas.height)), (2, 2));
        assert_eq!(texel_at(&atlas, rect, 1, 1), [10, 200, 30, 255]);
    }
    #[test]
    fn a_cit_models_relative_texture_is_packed() {
        let root = cit_root(
            "model",
            &[
                ("a.properties", "items=stick\nmodel=sword"),
                (
                    "sword.json",
                    r#"{"parent":"item/generated","textures":{"layer0":"./blade"}}"#,
                ),
            ],
        );
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("blade.png"), 2, 2, &[[40, 50, 60, 255]], None);
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let rect = atlas.sprites["horizon_cit:minecraft/optifine/cit/blade"];
        assert_eq!(texel_at(&atlas, rect, 1, 1), [40, 50, 60, 255]);
    }
    #[test]
    fn a_still_cit_texture_is_packed_whole() {
        // Not cut to a square as an unmarked strip would be: armour layers are 2:1.
        let root = cit_root("still", &[("a.properties", "items=stick\ntexture=a")]);
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("a.png"), 4, 2, &[[7, 8, 9, 255]], None);
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let rect = atlas.sprites["horizon_cit:minecraft/optifine/cit/a"];
        assert_eq!(rect.texels((atlas.width, atlas.height)), (4, 2));
    }
    #[test]
    fn an_animated_cit_texture_packs_one_frame_and_animates() {
        let root = cit_root("anim", &[("a.properties", "items=stick\ntexture=a")]);
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(
            &cit.join("a.png"),
            16,
            48,
            &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]],
            Some(r#"{"animation": {"frametime": 2}}"#),
        );
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let rect = atlas.sprites["horizon_cit:minecraft/optifine/cit/a"];
        assert_eq!(rect.texels((atlas.width, atlas.height)), (16, 16));
        assert_eq!(texel_at(&atlas, rect, 0, 0), [255, 0, 0, 255]);
        let animations: Vec<_> = atlas
            .sprite_animations
            .iter()
            .filter(|(r, _)| r.u0 == rect.u0 && r.v0 == rect.v0)
            .collect();
        assert_eq!(animations.len(), 1);
        assert_eq!(animations[0].1.frame_count(), 3);
    }
    #[test]
    fn an_armor_rules_texture_is_packed_too() {
        let root = cit_root(
            "armor",
            &[(
                "a.properties",
                "type=armor\nitems=iron_chestplate\ntexture.iron_layer_1=a",
            )],
        );
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("a.png"), 64, 32, &[[1, 2, 3, 255]], None);
        let rules = crate::cit::CitRules::load(&root).rules;
        assert_eq!(rules.len(), 1);
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        // Worn armour draws from the same atlas, under the name the layer looks up.
        let rect = atlas.sprites["horizon_cit:minecraft/optifine/cit/a"];
        assert_eq!(rect.texels((atlas.width, atlas.height)), (64, 32));
        let layer = crate::cit_armor::layer_sprite(&rules[0], "minecraft:iron", false, false);
        assert!(atlas.sprites.contains_key(&layer.unwrap()));
    }
    #[test]
    fn named_cit_textures_pack_once_and_bad_ones_are_skipped() {
        let root = cit_root(
            "mixed",
            &[
                // Vanilla's own texture: already packed, under its usual name.
                (
                    "v.properties",
                    "items=stick
texture=item/stick",
                ),
                (
                    "sub.properties",
                    "items=bow
texture.bow_pulling_0=b",
                ),
                (
                    "broken.properties",
                    "items=apple
texture=broken",
                ),
                (
                    "wide.properties",
                    "items=egg
texture=wide",
                ),
            ],
        );
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("b.png"), 2, 2, &[[5, 6, 7, 255]], None);
        fs::write(cit.join("broken.png"), b"not a png").unwrap();
        // A still image past the 1024x512 limit.
        write_strip(&cit.join("wide.png"), 1040, 1040, &[[1, 1, 1, 255]], None);
        // Vanilla's stick is already packed when the CIT pass runs.
        let stick = ("item/stick".to_owned(), 16, 16, vec![0; 16 * 16 * 4]);
        let added = cit_sprites(&root, &[stick]);
        let names: Vec<_> = added.iter().map(|(image, _, _)| image.0.as_str()).collect();
        assert_eq!(names, ["horizon_cit:minecraft/optifine/cit/b"]);
        // And the atlas still builds around the broken and the too-wide texture.
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert!(
            atlas
                .sprites
                .contains_key("horizon_cit:minecraft/optifine/cit/b")
        );
        assert!(atlas.sprites.contains_key("item/stick"));
    }
    #[test]
    fn one_oversized_texture_leaves_the_others_in() {
        // `a` sorts first and claims more than any atlas; `b` is small and must still pack.
        let root = cit_root(
            "oversized",
            &[
                ("a.properties", "items=stick\ntexture=a"),
                ("b.properties", "items=apple\ntexture=b"),
            ],
        );
        let cit = root.join("assets/minecraft/optifine/cit");
        // Only the header is read for `a`: a PNG signature and an IHDR of 20000x20000.
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        header.extend(20000u32.to_be_bytes());
        header.extend(20000u32.to_be_bytes());
        fs::write(cit.join("a.png"), header).unwrap();
        write_strip(&cit.join("b.png"), 2, 2, &[[4, 5, 6, 255]], None);
        let added = cit_sprites(&root, &[]);
        let _ = fs::remove_dir_all(&root);
        let names: Vec<_> = added.iter().map(|(i, _, _)| i.0.as_str()).collect();
        assert_eq!(names, ["horizon_cit:minecraft/optifine/cit/b"]);
    }
    #[test]
    fn a_wide_grid_animation_packs_its_frame() {
        // 1104 wide is past the limit for a still texture, not for a row of 16x16 frames.
        let root = cit_root("grid", &[("a.properties", "items=stick\ntexture=a")]);
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("a.png"), 1104, 16, &[[9, 9, 9, 255]], None);
        let mcmeta = r#"{"animation":{"width":16,"height":16}}"#;
        fs::write(cit.join("a.png.mcmeta"), mcmeta).unwrap();
        let added = cit_sprites(&root, &[]);
        let _ = fs::remove_dir_all(&root);
        let sizes: Vec<_> = added.iter().map(|(i, _, _)| (i.1, i.2)).collect();
        assert_eq!(sizes, [(16, 16)]);
    }
    #[test]
    fn a_root_without_cit_rules_packs_the_same_sprites() {
        let root = cit_root("none", &[("a.properties", "items=stick\ntexture=a")]);
        let cit = root.join("assets/minecraft/optifine/cit");
        write_strip(&cit.join("a.png"), 2, 2, &[[10, 200, 30, 255]], None);
        let with: HashSet<String> = HudAtlas::build(&root)
            .unwrap()
            .sprites
            .into_keys()
            .collect();
        fs::remove_dir_all(root.join("assets/minecraft/optifine")).unwrap();
        let without: HashSet<String> = HudAtlas::build(&root)
            .unwrap()
            .sprites
            .into_keys()
            .collect();
        let _ = fs::remove_dir_all(&root);
        assert!(without.is_subset(&with));
        let added: Vec<_> = with.difference(&without).collect();
        assert_eq!(added, ["horizon_cit:minecraft/optifine/cit/a"]);
        assert!(without.contains("item/stick"));
    }
}
