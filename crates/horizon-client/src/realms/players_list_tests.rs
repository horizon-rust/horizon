use super::*;
use crate::realms::{ConfigureTab, Popup, configure_screen, tests::listed};
use crate::title::{Mode, Title};
use horizon_assets::resource_source::{Resource, ResourceSource};
use std::path::{Path, PathBuf};

const SIZE: (f32, f32) = (800.0, 240.0);

fn assets() -> (HudAtlas, crate::lang::Language) {
    let chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let width = chars.len() as u32 * 8;
    let mut png_bytes = Vec::new();
    let mut png = png::Encoder::new(&mut png_bytes, width, 8);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&vec![255; width as usize * 32])
        .unwrap();
    let font = format!(
        r#"{{"providers":[{{"type":"bitmap","file":"minecraft:font/players_test.png","ascent":7,"chars":["{chars}"]}}]}}"#
    );
    let lang = br#"{
        "mco.configure.world.invited.number":"INVITED %s",
        "mco.configure.world.invites.normal.tooltip":"OP",
        "mco.configure.world.invites.ops.tooltip":"DEOP",
        "mco.configure.world.invites.remove.tooltip":"REMOVE",
        "mco.invited.player.narration":"PLAYER %s"
    }"#;
    let source = ResourceSource::indexed(
        [
            (
                "assets/minecraft/font/default.json".into(),
                Resource::bytes(font.into_bytes()),
            ),
            (
                "assets/minecraft/textures/font/players_test.png".into(),
                Resource::bytes(png_bytes),
            ),
            (
                "assets/minecraft/lang/en_us.json".into(),
                Resource::bytes(lang.to_vec()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let mut atlas = HudAtlas::build(&source).unwrap();
    for (i, sprite) in [
        "gui/sprites/player_list/make_operator",
        "gui/sprites/player_list/remove_operator",
        "gui/sprites/player_list/remove_player",
    ]
    .into_iter()
    .enumerate()
    {
        atlas.sprites.insert(
            sprite.into(),
            crate::hud::Rect {
                u0: i as f32 / 8.0,
                u1: i as f32 / 8.0 + 1.0 / 16.0,
                v0: 0.8125,
                v1: 0.875,
                w: 8,
                h: 7,
            },
        );
    }
    (atlas, crate::lang::Language::load(&[&source], "en_us"))
}

fn title(count: u128) -> Title {
    let realm = crate::realms::client::RealmServer {
        id: 7,
        players: (1..=count)
            .map(|id| PlayerInfo {
                name: format!("P{id}"),
                uuid: Some(format!("{id:032x}")),
                accepted: true,
                operator: id % 2 == 0,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut screen = listed(vec![realm.clone()]);
    screen.subscreen = configure_screen(realm.clone(), ConfigureTab::Players);
    if let SubScreen::Configure {
        detail,
        subscription,
        regions,
        tier,
        ..
    } = &mut screen.subscreen
    {
        *detail = Some(Ok(realm));
        *subscription = Some(Ok(Default::default()));
        *regions = Some(Ok(Vec::new()));
        *tier = Some(Ok(Default::default()));
    }
    screen.players_list.offline_faces = true;
    let mut title = Title::new(
        PathBuf::new(),
        Path::new("/nonexistent/horizon-test-resources"),
        "",
    );
    title.mode = Mode::Realms(screen);
    title.focused_button = None;
    title
}

fn screen(title: &Title) -> &Screen {
    let Mode::Realms(screen) = &title.mode else {
        panic!("expected Realms")
    };
    screen
}

fn draw(title: &Title, atlas: &HudAtlas, cursor: (f32, f32), frame: u64) -> HudBatch {
    let mut batch = HudBatch::new(
        [800, 240],
        crate::hud::GuiScale {
            requested: 1,
            unicode: false,
        },
    );
    widgets::cover_frame();
    title.draw(atlas, &mut batch, cursor, true, &mut |_, _| {});
    widgets::end_frame(frame);
    batch
}

fn confirmation_uuid(title: &Title) -> Option<u128> {
    let SubScreen::Popup(Popup::Uninvite { player, .. }) = &screen(title).subscreen else {
        return None;
    };
    player_uuid(player)
}

#[test]
fn players_draw_labels_with_trailing_icons_and_clip_the_scrolling_heading_and_buttons() {
    let (atlas, lang) = assets();
    crate::lang::scoped(lang, || {
        let mut title = title(24);
        if let Mode::Realms(screen) = &mut title.mode
            && let SubScreen::Configure {
                detail: Some(Ok(realm)),
                ..
            } = &mut screen.subscreen
        {
            realm.players[2].uuid = Some("00000000000000000000000000000000".into());
        }
        let batch = draw(&title, &atlas, (0.0, 0.0), 1);
        let widgets = widgets::last();
        let measures = |label, y| {
            widgets
                .iter()
                .find(|w| w.label == label && w.y == y)
                .unwrap()
        };
        assert!(measures("INVITED 24", 27.0).enabled);
        assert!(!measures("REMOVE", 124.0).enabled);
        for (label, index, button_y) in [("OP", 0, 52.0), ("DEOP", 1, 88.0), ("REMOVE", 2, 52.0)] {
            let button = measures(label, button_y);
            let icon_x = button.x + button.w - 10.0;
            let icon_y = button_y + 7.0;
            let u = index as f32 / 8.0;
            assert!(
                batch
                    .vertices
                    .iter()
                    .any(|v| v.text == 0 && v.uv == [u, 0.8125] && v.pos == [icon_x, icon_y])
            );
            assert!(batch.vertices.iter().any(|v| v.text == 0
                && v.uv == [u + 0.0625, 0.875]
                && v.pos == [icon_x + 8.0, icon_y + 7.0]));
            assert!(batch.vertices.iter().any(|v| v.text == 1
                && v.pos[0] >= button.x + 2.0
                && v.pos[0] < icon_x - 2.0
                && (button_y..button_y + 20.0).contains(&v.pos[1])));
        }
        // An operator reply between frames must not reinterpret the visible Op as Deop.
        if let Mode::Realms(screen) = &mut title.mode
            && let SubScreen::Configure {
                detail: Some(Ok(realm)),
                ..
            } = &mut screen.subscreen
        {
            realm.players[0].operator = true;
        }
        let op = measures("OP", 52.0);
        assert!(!title.click(op.x + op.w / 2.0, 62.0, SIZE).1);
        // The fourth button straddles the real bottom of the viewport at 172.
        assert_eq!(measures("REMOVE", 160.0).h, 12.0);
        assert!(!title.click(543.0, 174.0, SIZE).1);
        assert_eq!(confirmation_uuid(&title), None);
        title.scroll_worlds(-2.0, (400.0, 80.0), SIZE);
        let batch = draw(&title, &atlas, (543.0, 24.0), 2);
        let widgets = widgets::last();
        assert!(!widgets.iter().any(|w| w.label == "INVITED 24"));
        assert!(
            widgets
                .iter()
                .any(|w| w.label == "REMOVE" && w.y == 25.0 && w.h == 11.0)
        );
        let icons: Vec<_> = batch
            .vertices
            .iter()
            .filter(|v| {
                v.text == 0
                    && (0.8125..=0.875).contains(&v.uv[1])
                    && (500.0..548.0).contains(&v.pos[0])
            })
            .collect();
        assert!(icons.iter().any(|v| v.pos[1] == 25.0));
        assert!(icons.iter().all(|v| (25.0..=172.0).contains(&v.pos[1])));
        assert!(!title.click(543.0, 24.5, SIZE).1);
        assert!(title.click(543.0, 28.0, SIZE).1);
        assert_eq!(confirmation_uuid(&title), Some(1));
    });
}

#[test]
fn player_confirmation_waits_for_the_updated_operator_action_to_be_drawn() {
    let (atlas, lang) = assets();
    crate::lang::scoped(lang, || {
        let mut title = title(1);
        draw(&title, &atlas, (0.0, 0.0), 1);
        title.tab_dir(false, SIZE);
        title.tab_dir(false, SIZE);
        assert_eq!(
            screen(&title).displayed_player_action(),
            Some(PlayerAction::Op(1))
        );
        if let Mode::Realms(screen) = &mut title.mode
            && let SubScreen::Configure {
                detail: Some(Ok(realm)),
                ..
            } = &mut screen.subscreen
        {
            realm.players[0].operator = true;
        }
        assert!(
            !title.confirm_key_size(SIZE).1,
            "the visible Op must not become a Deop"
        );
        assert!(screen(&title).displayed_player_action().is_none());
        draw(&title, &atlas, (0.0, 0.0), 2);
        // Inspect the activation gate without submitting an operator request.
        assert_eq!(
            screen(&title).displayed_player_action(),
            Some(PlayerAction::Deop(1))
        );
    });
}

#[test]
fn players_shared_scrollbar_reaches_the_35th_uuid_and_rejects_stale_hits() {
    let (atlas, lang) = assets();
    crate::lang::scoped(lang, || {
        let mut title = title(35);
        draw(&title, &atlas, (0.0, 0.0), 1);
        assert!(!title.click(568.0, 90.0, SIZE).1);
        title.drag((600.0, 192.0), SIZE);
        assert_eq!(
            screen(&title).player_control_at((543.0, 128.0), SIZE),
            Some(PlayerAction::Remove(3))
        );
        assert!(!title.click(559.0, 90.0, SIZE).1);
        title.drag((690.0, 192.0), SIZE);
        title.release();
        title.drag((690.0, 10.0), SIZE);
        assert!(
            !title.click(543.0, 128.0, SIZE).1,
            "the old drawn row is stale"
        );
        draw(&title, &atlas, (543.0, 146.0), 2);
        assert!(title.click(543.0, 146.0, SIZE).1);
        assert_eq!(confirmation_uuid(&title), Some(35));
    });
}

#[test]
fn players_keyboard_keeps_its_column_through_53_rows_and_activates_the_uuid_after_reorder() {
    let (atlas, lang) = assets();
    crate::lang::scoped(lang, || {
        let mut title = title(53);
        draw(&title, &atlas, (0.0, 0.0), 1);
        title.tab_dir(false, SIZE);
        assert!(
            !title.confirm_key_size(SIZE).1,
            "the heading is focusable, not an action"
        );
        title.tab_dir(false, SIZE);
        for id in 1..=53 {
            draw(&title, &atlas, (0.0, 0.0), id as u64 + 1);
            let focused = screen(&title).focused_player_rect(SIZE).unwrap();
            let point = (focused[0] + focused[2] / 2.0, focused[1] + 10.0);
            assert_eq!(focused[3], 20.0, "row {id} was not revealed");
            let action = if id % 2 == 0 {
                PlayerAction::Deop(id)
            } else {
                PlayerAction::Op(id)
            };
            assert_eq!(screen(&title).player_control_at(point, SIZE), Some(action));
            if id < 53 {
                title.arrow_key(KeyCode::ArrowDown, SIZE, false);
            }
        }
        title.arrow_key(KeyCode::ArrowRight, SIZE, false);
        if let Mode::Realms(screen) = &mut title.mode
            && let SubScreen::Configure {
                detail: Some(Ok(realm)),
                ..
            } = &mut screen.subscreen
        {
            realm.players.swap(1, 52);
        }
        assert!(title.confirm_key_size(SIZE).1);
        assert_eq!(confirmation_uuid(&title), Some(53));
        title.escape();
        assert!(title.player_action_focused());
    });
}
