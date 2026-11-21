//! The redesign's specimen: every motion the mockup asks of the launcher, drawn by the launcher
//! itself, for checking the effects in the real renderer before the pages take them
//! (`HORIZON_LAUNCHER_PAGE=specimen`; not a page players reach). Hover everything: the point of
//! this page is what happens under the pointer.
use crate::glass_ui::{self as ui};
use crate::theme::Theme;
use gpui::{div, prelude::*, px, FontWeight};
use horizon_theme::glass::{ink, radius, Glass};
use std::sync::{LazyLock, OnceLock};
use std::time::Instant;

/// Whether the launcher was opened on the specimen.
pub fn wanted() -> bool {
    static WANTED: LazyLock<bool> =
        LazyLock::new(|| std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("specimen"));
    *WANTED
}

#[derive(Clone, Copy, PartialEq)]
enum Seg {
    One,
    Two,
    Three,
}

/// The specimen's own state: which segmented option is chosen, so the pill can be made to slide.
static SEG: std::sync::Mutex<Seg> = std::sync::Mutex::new(Seg::One);

/// Synchronized proportional-vs-tabular live demo for checking OpenType `tnum` across fonts/OSes.
fn tabular_demo() -> gpui::Div {
    const TOTAL_FILES: usize = 5160;
    const CYCLE_SECS: f32 = 10.0;
    static START: OnceLock<Instant> = OnceLock::new();
    let still = crate::motion::still();
    let elapsed = if still {
        1.11
    } else {
        START.get_or_init(Instant::now).elapsed().as_secs_f32()
    };
    let fraction = if still {
        0.11
    } else {
        (elapsed % CYCLE_SECS) / CYCLE_SECS
    };
    let pct = (fraction * 100.).round() as u32;
    let files = ((fraction * TOTAL_FILES as f32).round() as usize).min(TOTAL_FILES);
    let tenths = ((elapsed * 10.).floor() as u64) % 6000;
    let mins = tenths / 600;
    let secs = (tenths / 10) % 60;
    let dsec = tenths % 10;
    let look = Glass::default_look();
    let accent = ui::color(look.accent);
    let on_accent = ui::color(look.on_accent);
    let row = |tag: &str, tabular: bool| {
        let play_text = if still {
            "Preparing 11% / 88%".to_string()
        } else {
            format!("Preparing {pct}%")
        };
        let detail = if still {
            "1,111 of 5,160 files  ·  00:11.1 / 00:88.8".to_string()
        } else {
            format!(
                "{} of {} files  ·  {mins:02}:{secs:02}.{dsec}",
                crate::thousands(files as u64),
                crate::thousands(TOTAL_FILES as u64)
            )
        };
        let play_label = div()
            .relative()
            .flex()
            .flex_1()
            .justify_center()
            .px(px(16.))
            .whitespace_nowrap()
            .text_color(on_accent)
            .font_family(horizon_theme::glass::FONT)
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(play_text);
        let detail_label = ui::text(13., FontWeight::NORMAL, ink::BODY)
            .whitespace_nowrap()
            .child(detail);
        let (play_label, detail_label) = if tabular {
            (ui::tabular(play_label), ui::tabular(detail_label))
        } else {
            (play_label, detail_label)
        };
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                ui::text(12., FontWeight::MEDIUM, ink::QUIET)
                    .w(px(136.))
                    .child(tag.to_string()),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .w(px(210.))
                    .h(px(40.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(accent.opacity(0.5))
                    .rounded(px(radius::LARGE))
                    .bg(accent.opacity(0.35))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .w(gpui::relative(fraction.clamp(0., 1.)))
                            .bg(accent),
                    )
                    .child(play_label),
            )
            .child(detail_label)
    };
    div()
        .flex()
        .flex_col()
        .rounded(px(14.))
        .bg(ui::white(0.035))
        .border_1()
        .border_color(ui::white(ui::edge::REST + 0.03))
        .p(px(16.))
        .gap(px(10.))
        .child(ui::text(14., FontWeight::SEMIBOLD, 1.).child("Tabular figures (OpenType tnum)"))
        .child(row("Proportional", false))
        .child(row("Tabular (tnum)", true))
}

pub fn render(t: &Theme) -> gpui::Div {
    ui::set_on(true);
    ui::set_light(false);
    crate::glass::set_light(false);
    ui::set_look(Glass::default_look());
    crate::glass::set_look(Glass::default_look());
    let seg = *SEG.lock().unwrap();
    let pick = |v: Seg, window: &mut gpui::Window, _: &mut gpui::App| {
        *SEG.lock().unwrap() = v;
        window.refresh();
    };
    let cards = div()
        .flex()
        .items_start()
        .gap(px(16.))
        .child(ui::lift(
            &"spec-card-a".into(),
            ui::card("spec-card-a", 14., t)
                .w(px(220.))
                .p(px(16.))
                .gap(px(8.))
                .child(ui::text(16., gpui::FontWeight::SEMIBOLD, 1.).child("Lifting card"))
                .child(ui::text(13., gpui::FontWeight::NORMAL, 0.6).child(
                    "Rises 3 px on the spring while the pointer is on it; its outline warms \
                         to the accent and a glow opens under it.",
                )),
        ))
        .child(ui::lift(
            &"spec-card-b".into(),
            ui::card("spec-card-b", 14., t)
                .w(px(220.))
                .p(px(16.))
                .gap(px(8.))
                .child(ui::text(16., gpui::FontWeight::SEMIBOLD, 1.).child("Second card"))
                .child(
                    ui::text(13., gpui::FontWeight::NORMAL, 0.6)
                        .child("Pressing sinks it 1 px instead of scaling it."),
                ),
        ));
    let buttons = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ui::button(
            "spec-primary".into(),
            "Primary, with sheen".into(),
            crate::Kind::Primary,
            t,
        ))
        .child(ui::button(
            "spec-secondary".into(),
            "Secondary".into(),
            crate::Kind::Secondary,
            t,
        ))
        .child(ui::button(
            "spec-ghost".into(),
            "Ghost".into(),
            crate::Kind::Ghost,
            t,
        ));
    let nav = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(ui::nav_tab("spec-nav-open", "Open page", true, t))
        .child(ui::nav_tab("spec-nav-shut", "Another page", false, t));
    let segments = crate::segments(
        "spec-seg",
        &[(Seg::One, "One"), (Seg::Two, "Two"), (Seg::Three, "Three")],
        seg,
        t,
        pick,
    );
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .bg(ui::color(Glass::default_look().base))
        .flex()
        .flex_col()
        .items_start()
        .gap(px(20.))
        .p(px(32.))
        .child(ui::text(24., gpui::FontWeight::SEMIBOLD, 1.).child("Motion specimen"))
        .child(cards)
        .child(buttons)
        .child(nav)
        .child(segments)
        .child(ui::text(13., gpui::FontWeight::NORMAL, 0.5).child(
            "Hover each part; click the segmented control to slide its pill. Reduced motion \
                 draws everything where it lands.",
        ))
        .child(tabular_demo())
}
