use super::*;
use std::path::{Path, PathBuf};
#[test]
fn unihex_rows_and_widths() {
    let mut out = HashMap::new();
    // 'A' from unifont (8 wide) and a 16-wide glyph with an empty left column.
    let wide = format!("4E00:{}7FFE{}\n", "0000".repeat(4), "0000".repeat(11));
    parse_hex(b"0041:0000000018242442427E424242420000\n", &mut out);
    parse_hex(wide.as_bytes(), &mut out);
    let a = out[&0x41];
    assert_eq!((a.left, a.right), (1, 6));
    assert_eq!(a.rows[4], 0x18 << 24);
    let yi = out[&0x4E00];
    assert_eq!((yi.left, yi.right), (1, 14));
    // Unparseable lines are skipped.
    parse_hex(b"zz:00\n", &mut out);
    assert_eq!(out.len(), 2);
}

#[test]
fn glyph_uvs_have_the_native_texel_inset() {
    for (x, y, w, h) in [(120, 48, 8, 8), (0, 0, 1, 16), (4000, 2000, 32, 16)] {
        let r = glyph_rect(x, y, w, h, (4096, 4096));
        assert_eq!((r.w, r.h), (w, h));
        assert!(r.u0 > x as f32 / 4096.0 && r.v0 > y as f32 / 4096.0);
        assert!(r.u1 < (x + w) as f32 / 4096.0 && r.v1 < (y + h) as f32 / 4096.0);
        assert_eq!(r.u0, (x as f32 + 0.01) / 4096.0);
        assert_eq!(r.v1, ((y + h) as f32 - 0.01) / 4096.0);
    }
}

/// A resource root with these font definitions and files.
fn root(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-font-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    root
}
fn white() -> Rect {
    Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 0.0,
        v1: 0.0,
        w: 1,
        h: 1,
    }
}
fn load(root: &Path) -> Font {
    Font::load(
        &ResourceSource::directory(root.to_owned()),
        &HashMap::new(),
        &[],
        (4096, 1024),
        (512, 512),
        white(),
    )
}
fn png(width: u32, height: u32, opaque: impl Fn(u32, u32) -> bool) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend(if opaque(x, y) { [255; 4] } else { [0; 4] });
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    out
}

#[test]
fn archive_fonts_keep_lazy_unihex_on_the_open_reload_snapshot() {
    use std::io::{Cursor, Write};
    let root = std::env::temp_dir().join(format!("horizon-font-archive-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let write = |path: &Path, bits: &str| {
        let mut unifont = zip::ZipWriter::new(Cursor::new(Vec::new()));
        unifont
            .start_file("glyphs.hex", zip::write::SimpleFileOptions::default())
            .unwrap();
        unifont
            .write_all(format!("0041:{}\n", bits.repeat(16)).as_bytes())
            .unwrap();
        let unifont = unifont.finish().unwrap().into_inner();
        let image = png(4, 8, |_, _| true);
        let mut pack = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in [
            ("assets/minecraft/font/default.json", br#"{"providers":[{"type":"reference","id":"demo:nested/bitmap"}]}"#.as_slice()),
            ("assets/demo/font/nested/bitmap.json", br#"{"providers":[{"type":"bitmap","file":"demo:font/sheet.png","ascent":7,"chars":["a"]}]}"#),
            ("assets/demo/font/hex.json", br#"{"providers":[{"type":"unihex","hex_file":"demo:font/unifont.zip"}]}"#),
            ("assets/demo/font/type.json", br#"{"providers":[{"type":"ttf","file":"demo:mono.ttf","size":11}]}"#),
            ("assets/demo/font/mono.ttf", horizon_theme::fonts::GEIST_MONO),
            ("assets/demo/font/unifont.zip", unifont.as_slice()),
            ("assets/demo/textures/font/sheet.png", image.as_slice()),
        ] {
            pack.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            pack.write_all(bytes).unwrap();
        }
        pack.finish().unwrap();
    };
    let jar = root.join("fonts.jar");
    write(&jar, "FF");
    let source = ResourceSource::archive(&jar).unwrap();
    let font = Font::load(
        &source,
        &HashMap::new(),
        &[],
        (4096, 1024),
        (512, 512),
        white(),
    );
    let metrics = Font::metrics(&source);
    let replacement = root.join("replacement.jar");
    write(&replacement, "F0");
    std::fs::rename(&jar, root.join("old.jar")).unwrap();
    std::fs::rename(replacement, &jar).unwrap();
    assert_eq!(
        font.glyph('a').advance,
        5.0,
        "bitmap font reference in another namespace"
    );
    assert!(
        font.glyph_in("demo:type", 'A').rect.is_some(),
        "archive TrueType data"
    );
    assert_eq!(
        font.glyph_in("demo:hex", 'A').advance,
        5.0,
        "first lazy read uses the old nested archive"
    );
    assert_eq!(metrics.glyph_in("demo:hex", 'A').advance, 5.0);
    assert!(metrics.glyph_in("demo:hex", 'A').rect.is_none());
    let reloaded_source = ResourceSource::archive(&jar).unwrap();
    let reloaded = Font::metrics(&reloaded_source);
    assert_eq!(reloaded.glyph_in("demo:hex", 'A').advance, 3.0);
    assert!(!root.join("assets").exists());
    drop((font, metrics, reloaded, source, reloaded_source));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn server_metrics_preserve_bitmap_true_type_and_unihex_advances() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let def = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let ttf =
        br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2}]}"#;
    let root = root(
        "metrics",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/font/alt.json", ttf),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let rendered = load(&root);
    let metrics = Font::metrics(&ResourceSource::directory(root.clone()));
    assert_eq!(metrics.width("a b"), 14.0);
    assert_eq!(metrics.width("§lab"), 12.0);
    let text = Text::literal("ab ab");
    assert_eq!(
        metrics.split_formatted(&text, 10.0),
        rendered.split_formatted(&text, 10.0)
    );
    let alt = Style {
        font: Some("minecraft:alt".into()),
        bold: Some(true),
        ..Style::default()
    };
    for c in ['A', 'i', ' ', '\u{fffd}'] {
        assert_eq!(metrics.advance(c, &alt), rendered.advance(c, &alt));
        assert!(metrics.glyph_styled(c, &alt).rect.is_none());
    }
    let mut hex = HashMap::new();
    parse_hex(b"0041:0000000018242442427E424242420000\n", &mut hex);
    let h = hex[&0x41];
    let glyph = metrics
        .rasterize_hex(&mut metrics.cache.borrow_mut().region, h)
        .unwrap();
    assert_eq!(glyph.advance, 4.0);
    assert_eq!(glyph.offset, 0.5);
    assert!(glyph.rect.is_none());
    assert!(metrics.take_patches().is_empty());
    assert!(!rendered.take_patches().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn true_type_glyphs_are_rasterized_at_the_provider_size() {
    let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2,"shift":[0.5,1],"skip":["B"]}]}"#;
    let root = root(
        "ttf",
        &[
            ("assets/minecraft/font/default.json", def),
            (
                "assets/minecraft/font/mono.ttf",
                horizon_theme::fonts::GEIST_MONO,
            ),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let a = font.glyph('A');
    let rect = a.rect.expect("A is drawn");
    // Oversampled 2x: the bitmap is twice the glyph's GUI size.
    assert_eq!(rect.w as f32 / 2.0, a.right - a.left);
    assert_eq!(rect.h as f32 / 2.0, a.bottom - a.top);
    // A monospaced font: every advance is the same, in half pixels.
    assert_eq!(a.advance, font.glyph('i').advance);
    assert_eq!((a.advance * 2.0).fract(), 0.0);
    assert!(a.advance > 4.0 && a.advance < 9.0, "{}", a.advance);
    // Capitals sit on the baseline at y = 7, shifted down a pixel.
    assert!((a.bottom - 8.0).abs() <= 0.5, "{}", a.bottom);
    assert_eq!(a.offset, 1.0);
    // The space has an advance and nothing to draw.
    let space = font.glyph(' ');
    assert!(space.rect.is_none() && space.advance > 0.0);
    // Skipped characters fall through to the missing glyph.
    assert_eq!(font.glyph('B').advance, 6.0);
    // Coverage as `rrrr` texels, faint coverage cut.
    let patches = font.take_patches();
    assert_eq!(patches.len(), 2);
    let texels: Vec<&[u8]> = patches.iter().flat_map(|p| p.rgba.chunks(4)).collect();
    assert!(texels.iter().all(|t| t.iter().all(|v| *v == t[0])));
    assert!(texels.iter().all(|t| t[0] == 0 || t[0] >= 25));
    assert!(texels.iter().any(|t| t[0] == 255));
}

/// A bitmap provider's `file` can name any texture. When the atlas has that
/// texture (HUD packs keep their glyph sheets outside `textures/font`), the
/// glyphs are read from it instead of being copied into the small run-time
/// region one by one; the region then cannot fill up and turn later glyphs
/// into the missing one, whose advance shifts the rest of a HUD line.
#[test]
fn a_bitmap_sheet_outside_font_is_read_from_the_atlas() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let (_, _, pixels) = crate::hud::decode_bytes(&sheet).unwrap();
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 8,
    };
    for (ns, file, key) in [
        ("minecraft", "minecraft:hud/sheet.png", "hud/sheet"),
        (
            "hudengine",
            "hudengine:font/sheet.png",
            "hudengine:font/sheet",
        ),
    ] {
        let def = format!(
            r#"{{"providers":[{{"type":"bitmap","file":"{file}","ascent":7,"chars":["ab"]}}]}}"#
        );
        // The image is only in the atlas: the pack's copy is not on disk.
        let root = root(
            ns,
            &[("assets/minecraft/font/default.json", def.as_bytes())],
        );
        let sprites = HashMap::from([(key.to_string(), rect)]);
        let font = Font::load(
            &ResourceSource::directory(root.clone()),
            &sprites,
            &pixels,
            (16, 8),
            (0, 8),
            white(),
        );
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 3.0, "{ns}");
        assert_eq!(font.glyph('b').advance, 7.0, "{ns}");
        assert!(font.glyph('a').rect.is_some(), "{ns}");
        assert!(
            font.take_patches().is_empty(),
            "{ns}: the atlas sheet is not copied into the run-time region"
        );
    }
}

#[test]
fn styles_pick_their_font_and_unknown_fonts_draw_missing_glyphs() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let alt = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ba"]}]}"#;
    let root = root(
        "styles",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/font/alt.json", alt),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    // The sheet is not in this atlas: glyphs are copied in when first drawn.
    assert_eq!(font.glyph('a').advance, 3.0);
    assert_eq!(font.glyph('b').advance, 7.0);
    let alt = Style {
        font: Some("minecraft:alt".into()),
        ..Style::default()
    };
    assert_eq!(font.glyph_styled('a', &alt).advance, 7.0);
    assert_eq!(font.glyph_in("alt", 'b').advance, 3.0);
    assert!(font.glyph_styled('a', &alt).rect.is_some());
    // Not in alt (no space provider there), or no such font at all.
    assert_eq!(font.glyph_styled(' ', &alt).advance, 6.0);
    let missing = Style {
        font: Some("minecraft:nope".into()),
        ..Style::default()
    };
    assert_eq!(font.glyph_styled(' ', &missing).advance, 6.0);
    assert_eq!(font.advance('a', &alt), 7.0);
    let text = Text::literal("ab").with_style(&alt);
    assert_eq!(font.width_text(&text), 10.0);
    assert_eq!(font.random_glyph(&alt, 7.0).advance, 7.0);
    assert!(!font.take_patches().is_empty());
}

/// `FontSet.computeGlyphInfo`: `SelectedGlyphs.any` is the first provider's glyph, fishy
/// advance or not, and is what is drawn and measured; `nonFishy`, what obfuscated text draws,
/// is the first provider whose advance is within 0..32, else the missing glyph.
#[test]
fn a_fishy_glyph_is_drawn_and_only_obfuscated_text_falls_through() {
    // 64 opaque columns: the first provider's 'a' advances 65, past the 32 vanilla
    // calls fishy, and the narrow provider's 'a' advances 3.
    let wide = png(64, 8, |_, _| true);
    let narrow = png(16, 8, |x, _| x < 2);
    let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/wide.png","ascent":7,"chars":["a"]},{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}]}"#;
    let root = root(
        "fishy",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/textures/font/wide.png", &wide),
            ("assets/minecraft/textures/font/narrow.png", &narrow),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let drawn = font.glyph('a');
    assert_eq!(drawn.advance, 65.0, "the first provider's glyph is drawn");
    assert!(drawn.rect.is_some());
    assert_eq!(font.advance('a', &Style::default()), 65.0);
    // The by-width table files 'a' under its first glyph's 65; the glyph it then draws is
    // the narrow provider's (`nonFishy`).
    assert_eq!(font.random_glyph(&Style::default(), 65.0).advance, 3.0);
    // Nothing else is 3 wide in the table, and the missing glyph is 6.
    assert_eq!(font.random_glyph(&Style::default(), 3.0).advance, 6.0);
}

/// A `space` provider's shift glyphs (kerning in server packs: -512 to 512) are drawn and
/// measured as they are, and `nonFishy` has nothing else to draw for them.
#[test]
fn space_glyphs_with_any_advance_are_kept() {
    let shifts: Vec<i32> = (0..10)
        .map(|i| -(1 << i))
        .chain((0..10).map(|i| 1 << i))
        .collect();
    let advances: Vec<String> = shifts
        .iter()
        .enumerate()
        .map(|(i, a)| format!("\"\\ue{:03x}\": {a}", i))
        .collect();
    let def = format!(
        r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
        advances.join(",")
    );
    let root = root(
        "space-shifts",
        &[("assets/minecraft/font/default.json", def.as_bytes())],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    for (i, a) in shifts.iter().enumerate() {
        let c = char::from_u32(0xe000 + i as u32).unwrap();
        assert_eq!(font.glyph(c).advance, *a as f32, "{c:?}");
        assert_eq!(font.advance(c, &Style::default()), *a as f32);
    }
    // Obfuscated text asks for a glyph of a given width: a fishy space has none to give.
    let c = char::from_u32(0xe000 + 19).unwrap();
    assert_eq!(font.glyph(c).advance, 512.0);
    assert_eq!(
        font.glyph_non_fishy(DEFAULT, c).advance,
        6.0,
        "the missing glyph"
    );
    assert_eq!(font.random_glyph(&Style::default(), 512.0).advance, 6.0);
}

/// Nexo's GUI glyphs: a bitmap `height` of 287 with no advance, so every glyph wider than 31
/// pixels has a fishy advance, and is still drawn.
#[test]
fn a_wide_bitmap_glyph_is_drawn() {
    let sheet = png(40, 287, |_, _| true);
    let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/gui.png","height":287,"ascent":65,"chars":["a"]}]}"#;
    let root = root(
        "wide-bitmap",
        &[
            ("assets/minecraft/font/default.json", def),
            ("assets/minecraft/textures/font/gui.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let g = font.glyph('a');
    assert_eq!(g.advance, 41.0, "the glyph's 40 columns and one");
    assert!(g.rect.is_some(), "drawn, not the missing glyph");
    assert!(fishy_advance(g.advance));
    assert_eq!(
        font.glyph_non_fishy(DEFAULT, 'a').advance,
        6.0,
        "excluded from what obfuscated text draws"
    );
}

/// A font whose first provider is `first` (its sheet `sheet`), then a
/// valid provider drawing `a` from a sheet with two columns used.
fn font_after(name: &str, first: &str, sheet: &[u8]) -> Font {
    let narrow = png(8, 8, |x, _| x < 2);
    let def = format!(
        r#"{{"providers":[{first},{{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}}]}}"#
    );
    let root = root(
        name,
        &[
            ("assets/minecraft/font/default.json", def.as_bytes()),
            ("assets/minecraft/textures/font/first.png", sheet),
            ("assets/minecraft/textures/font/narrow.png", &narrow),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    font
}

/// `BitmapProvider.Definition` refuses these while the file decodes, and
/// the whole file goes. Rows of unequal length were sized from the first
/// row, so the longer row's last cell reached past the sheet and panicked
/// when drawn.
#[test]
fn a_font_file_with_a_refused_bitmap_provider_is_skipped() {
    let sheet = png(16, 16, |_, _| true);
    for (name, first) in [
        ("uneven", r#""ascent":7,"chars":["ab","cdef"]"#),
        (
            "ascent-above-height",
            r#""height":8,"ascent":9,"chars":["af"]"#,
        ),
        ("text-ascent", r#""ascent":"7","chars":["af"]"#),
        ("no-ascent", r#""chars":["af"]"#),
        ("empty-row", r#""ascent":7,"chars":["","af"]"#),
    ] {
        let first = format!(r#"{{"type":"bitmap","file":"minecraft:font/first.png",{first}}}"#);
        let font = font_after(name, &first, &sheet);
        let missing = font.glyph('z');
        for c in ['a', 'f'] {
            let g = font.glyph(c);
            assert_eq!(
                g.advance, missing.advance,
                "{name}: {c} is the missing glyph"
            );
            assert_eq!(g.rect.map(|r| r.u0), missing.rect.map(|r| r.u0), "{name}");
        }
    }
}

/// Vanilla reads `height` and `ascent` as integers, a fraction truncated
/// toward zero: 8.9 and 6.5 load as 8 and 6.
#[test]
fn fractional_height_and_ascent_are_truncated() {
    let sheet = png(16, 16, |_, _| true);
    let first = r#"{"type":"bitmap","file":"minecraft:font/first.png","height":8.9,"ascent":6.5,"chars":["af"]}"#;
    let g = font_after("fractional", first, &sheet).glyph('f');
    // Cells 8x16 at height 8: half scale; top 7 - 6.
    assert_eq!((g.top, g.bottom), (1.0, 9.0));
    assert_eq!(g.advance, 5.0, "8 columns used, at half scale, and one");
}

/// `BitmapProvider.Definition.load` puts each cell in a map: a character
/// listed twice keeps its last cell. Character 0 is padding, and an earlier
/// provider still wins over a later one.
#[test]
fn a_repeated_character_keeps_its_last_cell() {
    // Cells 8 wide using 2, 6 and 4 columns: advances 3, 7 and 5.
    let sheet = png(24, 8, |x, _| {
        x < 2 || (8..14).contains(&x) || (16..20).contains(&x)
    });
    let first =
        r#"{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":["a\u0000a"]}"#;
    let font = font_after("repeated", first, &sheet);
    assert_eq!(
        font.glyph('a').advance,
        5.0,
        "the last cell, not the first (3) or the next provider's (3)"
    );
    assert_eq!(
        font.glyph('\0').advance,
        font.glyph('z').advance,
        "character 0 is not the middle cell (7)"
    );
}

/// More rows or columns than the sheet has pixels leave cells of no size.
/// Vanilla keeps the provider: each character is an empty glyph of advance
/// 1, and a later provider's glyph for it isn't used.
#[test]
fn a_grid_larger_than_its_sheet_draws_empty_glyphs() {
    for (name, sheet, chars) in [
        ("too-many-rows", png(16, 1, |_, _| true), r#"["a","b"]"#),
        ("too-many-columns", png(4, 8, |_, _| true), r#"["abcde"]"#),
    ] {
        let first = format!(
            r#"{{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":{chars}}}"#
        );
        let g = font_after(name, &first, &sheet).glyph('a');
        assert_eq!(g.advance, 1.0, "{name}: not the next provider's 3");
        assert!(g.rect.is_none(), "{name}: nothing drawn");
        assert!(g.top.is_finite() && g.bottom.is_finite(), "{name}");
    }
}

/// StringDecomposer.iterateFormatted: a `§` that ends a part stops that
/// part only; the next part is still measured and can break.
#[test]
fn a_section_sign_ending_a_part_still_breaks_the_next() {
    let sheet = png(16, 8, |x, _| x < 2);
    let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let root = root(
        "section-sign",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let text = Text::from_spans(
        ["a§", "a a"]
            .map(|part| Span::new(part, Style::default()))
            .into(),
    );
    // 3 + 3 + 4 + 3 is past 12: the line breaks at the space.
    let lines: Vec<String> = font
        .split_formatted(&text, 12.0)
        .iter()
        .map(Text::plain)
        .collect();
    assert_eq!(lines, ["a§a", "a"]);
    let mixed = Text::literal("aa")
        .append(Text::untrusted("aa"))
        .with_style(&Style {
            click: Some(crate::text::ClickEvent::OpenFile("/tmp/local".into())),
            ..Style::default()
        });
    let lines = font.split_formatted(&mixed, 6.0);
    assert_eq!(
        lines.iter().map(Text::plain).collect::<Vec<_>>(),
        ["aa", "aa"]
    );
    assert_eq!(lines[0].spans[0].origin(), crate::text::Origin::Local);
    assert_eq!(lines[1].spans[0].origin(), crate::text::Origin::Remote);
    assert!(lines[0].spans[0].click_event().is_some());
    assert!(lines[1].spans[0].click_event().is_none());
}

/// A renderer rebuilt over the same font (the loading screen's handoff)
/// makes its atlas texture from the textures alone, so the glyphs it had
/// rasterized into the previous texture must be placed into the new one.
#[test]
fn a_rebuilt_atlas_gets_its_glyphs_again() {
    let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
    let default = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
    let root = root(
        "rebuild",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/textures/font/a.png", &sheet),
        ],
    );
    let font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    let first = font.glyph('a');
    assert_eq!(font.take_patches().len(), 1);
    font.reset_cache();
    // Nothing is left over: drawing the character again places its pixels.
    assert!(font.take_patches().is_empty());
    assert_eq!(font.glyph('a').advance, first.advance);
    assert_eq!(font.take_patches().len(), 1);
}

#[test]
fn provider_filters_follow_the_font_options() {
    let sheet = png(8, 8, |x, _| x < 4);
    let default = br#"{"providers":[{"type":"reference","id":"minecraft:include/sheet","filter":{"uniform":false}},{"type":"space","advances":{"a":2}}]}"#;
    let sheet_def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/s.png","ascent":7,"chars":["a"]}]}"#;
    let root = root(
        "filters",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/minecraft/font/include/sheet.json", sheet_def),
            ("assets/minecraft/textures/font/s.png", &sheet),
        ],
    );
    let mut font = load(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(font.glyph('a').advance, 5.0);
    font.uniform = true;
    font.reset_cache();
    assert_eq!(font.glyph('a').advance, 2.0);
}

#[test]
fn a_full_glyph_region_recovers_at_the_next_frame() {
    // Two 8x8 glyphs from a sheet outside the atlas; the region holds one row.
    let sheet = png(16, 8, |x, _| x % 8 < 4);
    let default =
        br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["ab"]}]}"#;
    let root = root(
        "full",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/pack/textures/font/s.png", &sheet),
        ],
    );
    // A 10 wide atlas fits one 8 pixel glyph per row; the region two rows.
    let font = Font::load(
        &ResourceSource::directory(root.clone()),
        &HashMap::new(),
        &[],
        (10, 64),
        (0, 8),
        white(),
    );
    let _ = std::fs::remove_dir_all(&root);
    let epoch = font.epoch();
    let a = font.glyph('a');
    assert!(a.rect.is_some());
    // No room for 'b': missing for now, and asked again next time.
    assert!(font.glyph('b').rect.is_none());
    assert!(font.glyph('b').rect.is_none());
    assert_eq!(font.epoch(), epoch);
    font.begin_frame();
    assert_ne!(font.epoch(), epoch);
    let b = font.glyph('b');
    assert!(b.rect.is_some());
    assert_eq!(b.advance, a.advance);
    // Nothing overflowed since: the next frame keeps the glyphs.
    let epoch = font.epoch();
    font.begin_frame();
    assert_eq!(font.epoch(), epoch);
}

#[test]
fn a_character_without_a_provider_does_not_reset_the_region() {
    let sheet = png(8, 8, |x, _| x < 4);
    let default =
        br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["a"]}]}"#;
    let root = root(
        "nofill",
        &[
            ("assets/minecraft/font/default.json", default),
            ("assets/pack/textures/font/s.png", &sheet),
        ],
    );
    let font = Font::load(
        &ResourceSource::directory(root.clone()),
        &HashMap::new(),
        &[],
        (10, 64),
        (0, 8),
        white(),
    );
    let _ = std::fs::remove_dir_all(&root);
    let epoch = font.epoch();
    assert!(font.glyph('z').rect.is_none());
    font.begin_frame();
    assert_eq!(font.epoch(), epoch);
}
