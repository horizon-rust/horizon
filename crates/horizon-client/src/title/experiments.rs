//! CreateWorldScreen's Experiments screen (`ExperimentsScreen`): the feature
//! packs the jar ships under `data/minecraft/datapacks`, each with an on/off
//! switch, under the red warning that they cannot be turned off after the
//! world is created.
use crate::{
    hud::{HudAtlas, HudBatch},
    screen_draw::Measure,
    text::{Text, named_color},
    ui,
};

/// HeaderAndFooterLayout: the title's 33 pixels and the buttons' 33.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 33.0;
/// `ExperimentsScreen.MAIN_CONTENT_WIDTH`.
const CONTENT: f32 = 310.0;

/// One feature pack's switch: `SwitchGrid.addSwitch` with its info underneath.
pub struct Flag {
    pub id: String,
    pub title: Text,
    pub description: Text,
    pub enabled: bool,
}

/// What a press asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// `gui.done`: the switches go to the repository.
    Done,
    /// `gui.cancel` or Escape.
    Cancel,
}

/// A press on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    /// The i-th switch, to the state `Flag::enabled` flips to.
    Switch(usize),
    Outcome(Outcome),
}

/// The screen.
pub struct ExperimentsScreen {
    pub flags: Vec<Flag>,
    /// The switches' scrolling: the list is taller than the panel on a small
    /// window (`ScrollableLayout`).
    pub scroll: ui::Scroll,
    pub in_world: bool,
}
impl ExperimentsScreen {
    pub fn new(flags: Vec<Flag>) -> Self {
        Self {
            flags,
            scroll: ui::Scroll::default(),
            in_world: false,
        }
    }
    /// A press: a switch, Done (`gui.done`) or Cancel (`gui.cancel`).
    pub fn click(&self, atlas: &HudAtlas, (x, y): (f32, f32), size: (f32, f32)) -> Option<Click> {
        let m = Measure(&atlas.font);
        let content = self.content_width(size.0);
        let left = ((size.0 - content) / 2.0).trunc();
        for (i, top) in self.switch_tops(&m, size).into_iter().enumerate() {
            if ui::hovered((x, y), left, top, 150.0, 20.0) {
                return Some(Click::Switch(i));
            }
        }
        let [(dx, dy), (cx, cy)] = footer(size.0, size.1);
        if ui::hovered((x, y), dx, dy, 150.0, 20.0) {
            return Some(Click::Outcome(Outcome::Done));
        }
        if ui::hovered((x, y), cx, cy, 150.0, 20.0) {
            return Some(Click::Outcome(Outcome::Cancel));
        }
        None
    }
    /// The switches' tops, scrolled: `SwitchGrid` rows 4 apart from the
    /// warning text.
    fn switch_tops(&self, m: &Measure, (w, _): (f32, f32)) -> Vec<f32> {
        let content = self.content_width(w);
        let info = lines(m, &self.info(), content);
        let mut y = HEADER + 4.0 + info.len() as f32 * 10.0 + 15.0;
        let mut out = Vec::with_capacity(self.flags.len());
        for flag in &self.flags {
            out.push(y - self.scroll.amount);
            let info = lines(m, &self.description(&flag.description), content);
            y += 20.0 + 4.0 + info.len() as f32 * 10.0 + 4.0;
        }
        out
    }
    /// `ExperimentsScreen.INFO`, red.
    fn info(&self) -> Text {
        Text::translate("selectWorld.experiments.info", &[])
            .with_color(named_color("red").unwrap_or(0xFF5555))
    }
    fn description(&self, description: &Text) -> Text {
        description
            .clone()
            .with_color(named_color("gray").unwrap_or(0xAAAAAA))
    }
    fn content_width(&self, w: f32) -> f32 {
        CONTENT.min(w - 20.0)
    }
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
        let m = Measure(&atlas.font);
        // `addTitleHeader`: the title in the header's 33 pixels.
        let title = Text::translate("selectWorld.experiments", &[]);
        let tw = m.width(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw as f32) / 2.0).trunc(),
            15.0,
            u32::MAX,
            true,
        );
        // The warning, then a switch and its description per pack.
        let content = self.content_width(w);
        let left = ((w - content) / 2.0).trunc();
        let mut y = HEADER + 4.0;
        for line in lines(&m, &self.info(), content) {
            batch.text_styled(atlas, &line, left, y, u32::MAX, true);
            y += 10.0;
        }
        y += 15.0;
        for flag in &self.flags {
            let top = y - self.scroll.amount;
            y += 20.0 + 4.0;
            if top + 20.0 >= HEADER && top <= h - FOOTER {
                let hover = ui::hovered(cursor, left, top, 150.0, 20.0);
                let value = Text::translate(
                    "options.generic_value",
                    &[
                        flag.title.clone(),
                        Text::translate(
                            if flag.enabled {
                                "options.on"
                            } else {
                                "options.off"
                            },
                            &[],
                        ),
                    ],
                );
                ui::button(batch, atlas, left, top, 150.0, 20.0, &value, true, hover);
            }
            let description = self.description(&flag.description);
            for line in lines(&m, &description, content) {
                if top + 20.0 + 4.0 >= HEADER && y <= h - FOOTER {
                    batch.text_styled(atlas, &line, left, y, u32::MAX, true);
                }
                y += 10.0;
            }
            y += 4.0;
        }
        let [(dx, dy), (cx, cy)] = footer(w, h);
        for (bx, by, key) in [(dx, dy, "gui.done"), (cx, cy, "gui.cancel")] {
            let hover = ui::hovered(cursor, bx, by, 150.0, 20.0);
            ui::button(
                batch,
                atlas,
                bx,
                by,
                150.0,
                20.0,
                &Text::translate(key, &[]),
                true,
                hover,
            );
        }
    }
    pub fn wheel(&mut self, delta: f32, size: (f32, f32)) {
        let content = self.flags.len() as f32 * 38.0 + 60.0;
        self.scroll
            .wheel(delta, 18.0, HEADER, size.1 - FOOTER, content);
    }
}

/// LinearLayout's footer: two 150-wide buttons 8 apart, centred.
fn footer(w: f32, h: f32) -> [(f32, f32); 2] {
    let x = ((w - 308.0) / 2.0).trunc();
    let y = h - FOOTER + 6.0;
    [(x, y), (x + 158.0, y)]
}

/// `MultiLineTextWidget`'s wrap of `text` into `width`.
fn lines(m: &Measure, text: &Text, width: f32) -> Vec<Text> {
    // The first span's style carries the whole line's colour here.
    let style = text
        .spans
        .first()
        .map(|s| s.style.clone())
        .unwrap_or_default();
    let plain = text.resolve_legacy().plain();
    let mut out: Vec<Text> = Vec::new();
    let mut line = String::new();
    for word in plain.split(' ') {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && m.width_str(&candidate) as f32 > width {
            out.push(Text::literal(line).with_style(&style));
            line = word.to_owned();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(Text::literal(line).with_style(&style));
    }
    out
}
