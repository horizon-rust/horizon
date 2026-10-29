//! Setup's monitor map and draft window choices. Apply is consumed by the game adapter.
use crate::{
    graphics_style::STYLE,
    hud::{HudAtlas, HudBatch, rgba},
    split_layout::{Draft, Monitor, Placement, Spot, Warning, ordered},
    split_map::{self, BOTTOM, uncovered},
    text::Text,
    ui,
};

#[derive(Default)]
pub struct SplitUi {
    pub on: bool,
    pub idle: bool,
    pub live: Option<Draft>,
    pub monitors: Vec<Monitor>,
    pub main: String,
    pub names: Vec<String>,
    pub apply: Option<Draft>,
    edit: Option<Draft>,
    selected: Option<usize>,
}

#[derive(Clone)]
enum Target {
    Player(usize),
    Move(Spot),
    Revert,
    Apply,
    Done,
    Switch,
}
type Button = ([f32; 4], String, Target, bool);
pub enum Click {
    Nothing,
    Pressed,
    Done,
    Switch,
}

impl SplitUi {
    pub fn reset(&mut self) {
        self.edit = None;
        self.selected = None;
        self.apply = None;
    }
    fn draft(&self) -> Option<&Draft> {
        self.edit.as_ref().or(self.live.as_ref())
    }
    fn map(&self, w: f32, h: f32) -> split_map::Map {
        if self.idle {
            return Default::default();
        }
        self.draft().map_or_else(Default::default, |draft| {
            split_map::project(draft, &self.monitors, &self.main, w, h)
        })
    }
    fn editing(&mut self) -> Option<&mut Draft> {
        if self.edit.is_none() {
            self.edit = self.live.clone();
        }
        self.edit.as_mut()
    }

    fn monitor_name(&self, name: &str) -> String {
        match self.monitors.iter().position(|m| m.name == name) {
            Some(n) => format!("Monitor {}", n + 1),
            None => "a monitor".to_owned(),
        }
    }

    /// Where the selected player can go: the other windows, a new one, a monitor no window fills.
    fn spots(&self) -> Vec<(String, Spot)> {
        let (Some(draft), Some(i)) = (self.draft(), self.selected) else {
            return Vec::new();
        };
        let mine = draft.players.get(i).copied();
        let mut spots: Vec<(String, Spot)> = draft
            .windows
            .iter()
            .filter(|w| Some(w.number) != mine)
            .map(|w| (format!("Window {}", w.number), Spot::Window(w.number)))
            .collect();
        spots.push(("New window".to_owned(), Spot::New));
        for m in &self.monitors {
            let filled = draft
                .windows
                .iter()
                .any(|w| w.placement == Placement::Fill(m.name.clone()));
            if m.name != self.main && !filled {
                spots.push((
                    format!("Fill {}", self.monitor_name(&m.name)),
                    Spot::Fill(m.name.clone()),
                ));
            }
        }
        spots
    }

    fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        let row = |out: &mut Vec<Button>, y: f32, items: Vec<(f32, String, Target, bool)>| {
            let total: f32 = items.iter().map(|i| i.0 + 4.0).sum::<f32>() - 4.0;
            let mut x = ((w - total) / 2.0).floor();
            for (bw, label, target, active) in items {
                out.push(([x, y, bw, 20.0], label, target, active));
                x += bw + 4.0;
            }
        };
        let done = (64.0, "Done".into(), Target::Done, true);
        // The switch, top right (P1's): off only once P1 plays alone (turning it off with others
        // in drops them all at once).
        let switch = (
            [w - 110.0, 6.0, 104.0, 20.0],
            format!("Splitscreen: {}", if self.on { "On" } else { "Off" }),
            Target::Switch,
            !self.on || self.names.len() <= 1,
        );
        if !self.on || self.idle {
            let mut out = vec![switch];
            row(&mut out, h - 27.0, vec![done]);
            return out;
        }
        let (_, windows, tiles) = self.map(w, h);
        let mut order: Vec<usize> = (0..tiles.len()).collect();
        order.sort_by_key(|&i| windows.iter().position(|(n, _)| *n == tiles[i].0));
        let mut out: Vec<Button> = order
            .into_iter()
            .map(|i| {
                let name = self
                    .names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Player {}", i + 1));
                // The part no later window covers: what can be seen and pressed.
                let at = windows.iter().position(|(n, _)| *n == tiles[i].0);
                let covers = at.map_or(&[][..], |k| &windows[k + 1..]);
                let rect = uncovered(tiles[i].1, covers.iter().map(|(_, r)| *r));
                (rect, name, Target::Player(i), true)
            })
            .collect();
        // The move row: one button for the windows, which cycles (the next window after the
        // player's, by number, wrapping), then a new window and the monitors.
        let mut spots = self.spots();
        let mine = self
            .selected
            .and_then(|i| self.draft().and_then(|d| d.players.get(i).copied()))
            .unwrap_or(0);
        let next = spots
            .iter()
            .filter_map(|(_, s)| match s {
                Spot::Window(n) => Some(*n),
                _ => None,
            })
            .min_by_key(|&n| (n <= mine, n));
        spots.retain(|(_, s)| !matches!(s, Spot::Window(n) if Some(*n) != next));
        if !spots.is_empty() {
            let items = spots
                .into_iter()
                .map(|(label, spot)| (86.0, label, Target::Move(spot), true))
                .collect();
            row(&mut out, h - BOTTOM + 4.0, items);
        }
        out.push(switch);
        let mut footer = Vec::new();
        let editing = self.changed();
        footer.extend([
            (64.0, "Revert".into(), Target::Revert, editing),
            (64.0, "Apply".into(), Target::Apply, editing),
            done,
        ]);
        row(&mut out, h - 27.0, footer);
        out
    }

    fn warnings(&self) -> Vec<String> {
        let Some(draft) = self.draft() else {
            return Vec::new();
        };
        draft
            .warnings(&self.monitors, &self.main)
            .into_iter()
            .map(|w| match w {
                Warning::Empty(1) => "Nobody plays in Window 1: it hides".to_owned(),
                // One open now closes; one only in the draft never opens.
                Warning::Empty(n) if self.live.as_ref().is_some_and(|l| !l.empty(n)) => {
                    format!("Nobody plays in Window {n}: it closes on Apply")
                }
                Warning::Empty(n) => format!("Nobody plays in Window {n}: it won't open"),
                Warning::OnWindow1(n) => {
                    format!("Window {n} fills Window 1's monitor: its players play in Window 1")
                }
                Warning::Unplugged(n, _) => {
                    format!("Window {n}'s monitor is not connected: its players play in Window 1")
                }
            })
            .collect()
    }

    /// The state represented by the preview area, also available to agent controls.
    pub fn status(&self) -> &'static str {
        if !self.on {
            "off"
        } else if self.idle {
            "idle"
        } else {
            "active"
        }
    }

    fn status_lines(&self) -> Option<[&'static str; 3]> {
        if !self.on {
            Some([
                "Splitscreen is off.",
                "Turn it on, then in a world press a button on another",
                "controller to add a player.",
            ])
        } else if self.idle {
            Some([
                "Splitscreen idle",
                "No other local players.",
                "Solo controls and controller feedback are unchanged.",
            ])
        } else {
            None
        }
    }

    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32), in_world: bool) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        STYLE.title(batch, atlas, &Text::literal("Splitscreen Setup"));
        if let Some(lines) = self.status_lines() {
            let grey = rgba(170, 170, 170, 255);
            for (k, line) in lines.into_iter().enumerate() {
                let y = (h / 2.0 - 30.0 + 12.0 * k as f32).floor();
                ui::centered_text(batch, atlas, &Text::literal(line), 0.0, y, w, grey);
            }
            for b in &self.buttons(w, h) {
                self.draw_button(batch, atlas, cursor, b);
            }
            return;
        }
        let (monitors, windows, tiles) = self.map(w, h);
        let grey = rgba(160, 160, 160, 255);
        for &[x, y, mw, mh] in &monitors {
            batch.fill(atlas, x, y, mw, mh, rgba(255, 255, 255, 60));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                mw - 2.0,
                mh - 2.0,
                rgba(0, 0, 0, 170),
            );
        }
        let buttons = self.buttons(w, h);
        let yellow = rgba(255, 255, 85, 255);
        // Each window, then its players: a window over another covers it.
        let draft = self.draft();
        // A window nobody plays in any more stays until Apply: grey, under the others.
        let emptied = |n: u8| n != 1 && draft.is_some_and(|d| d.empty(n));
        for &(number, [x, y, ww, wh]) in &windows {
            let edge = match emptied(number) {
                true => rgba(160, 160, 160, 120),
                false => rgba(255, 255, 85, 200),
            };
            batch.fill(atlas, x, y, ww, wh, edge);
            if emptied(number) {
                let label = Text::literal(format!("Window {number}: empty"));
                batch.text_styled(atlas, &label, x + 3.0, y - 10.0, grey, true);
            }
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                ww - 2.0,
                wh - 2.0,
                rgba(40, 40, 40, 230),
            );
            for b in buttons
                .iter()
                .filter(|b| matches!(b.2, Target::Player(i) if tiles[i].0 == number))
            {
                self.draw_button(batch, atlas, cursor, b);
            }
        }
        // The windows' names over everything, so another window's tiles never hide them.
        for (number, [x, y, _, _]) in windows.into_iter().filter(|w| !emptied(w.0)) {
            let (label, color) = (format!("Window {number}"), yellow);
            batch.text_styled(atlas, &Text::literal(label), x + 3.0, y - 10.0, color, true);
        }
        for b in buttons.iter().filter(|b| !matches!(b.2, Target::Player(_))) {
            self.draw_button(batch, atlas, cursor, b);
        }
        // The monitors' names under them, where no window covers them.
        for (n, [x, y, _, mh]) in monitors.into_iter().enumerate() {
            let label = Text::literal(format!("Monitor {}", n + 1));
            batch.text_styled(atlas, &label, x + 1.0, y + mh + 2.0, grey, true);
        }
        self.draw_notes(batch, atlas, cursor, &buttons);
    }

    fn draw_button(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        ([x, y, bw, bh], label, target, active): &Button,
    ) {
        let (mut x, mut y, mut bw, mut bh) = (*x, *y, *bw, *bh);
        let hover = *active && ui::hovered(cursor, x, y, bw, bh);
        let selected = matches!(target, Target::Player(i) if Some(*i) == self.selected);
        if matches!(target, Target::Player(_)) {
            (x, y, bw, bh) = (x + 1.0, y + 1.0, (bw - 2.0).max(4.0), (bh - 2.0).max(4.0));
        }
        ui::button(
            batch,
            atlas,
            x,
            y,
            bw,
            bh,
            &Text::literal(label),
            *active,
            hover || selected,
        );
    }
    fn draw_notes(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        buttons: &[Button],
    ) {
        let (w, h) = (batch.width, batch.height);
        let note = match self.selected {
            None => "Click a player to choose its window.".to_owned(),
            Some(_) if self.edit.is_some() => "Apply to play this way, Revert to undo.".to_owned(),
            Some(i) => format!(
                "Where should {} play?",
                self.names.get(i).map_or("this player", String::as_str)
            ),
        };
        for (k, line) in std::iter::once(note)
            .chain(self.warnings())
            .take(2)
            .enumerate()
        {
            ui::centered_text(
                batch,
                atlas,
                &Text::literal(line),
                0.0,
                h - BOTTOM + 28.0 + k as f32 * 11.0,
                w,
                rgba(170, 170, 170, 255),
            );
        }
        if buttons.iter().any(|(r, _, t, active)| {
            !active && matches!(t, Target::Switch) && ui::hovered(cursor, r[0], r[1], r[2], r[3])
        }) {
            ui::tooltip(
                batch,
                atlas,
                &atlas.font.split(
                    &Text::literal(
                        "Splitscreen turns off once the others have left (their pause menu).",
                    ),
                    170.0,
                ),
                cursor,
            );
        }
    }
    /// A press at `(x, y)` on a screen `w` x `h`, or the mouse moving with it held (`drag`).
    pub fn click(&mut self, (x, y): (f32, f32), (w, h): (f32, f32), drag: bool) -> Click {
        if drag {
            return Click::Nothing;
        }
        // The topmost: the last drawn.
        let hit = self
            .buttons(w, h)
            .into_iter()
            .rev()
            .find(|([bx, by, bw, bh], _, _, active)| {
                *active && ui::hovered((x, y), *bx, *by, *bw, *bh)
            });
        let Some((_, _, target, _)) = hit else {
            return Click::Nothing;
        };
        match target {
            Target::Player(i) => {
                self.selected = Some(i);
            }
            Target::Move(spot) => {
                if let Some(i) = self.selected {
                    self.place(i, spot);
                }
            }
            Target::Revert => self.edit = None,
            Target::Apply => {
                // Windows emptied in the draft close now, not as players moved.
                self.apply = self.edit.take().map(|mut d| {
                    d.drop_empty(&[]);
                    d
                });
            }
            Target::Done => {
                self.reset();
                return Click::Done;
            }
            Target::Switch => {
                self.reset();
                return Click::Switch;
            }
        }
        Click::Pressed
    }

    fn changed(&self) -> bool {
        // A free window's spot moves under the draft as it's dragged around: not a change.
        let windows = |d: &Draft| -> Vec<(u8, Option<String>)> {
            let fill = |p: &Placement| match p {
                Placement::Fill(name) => Some(name.clone()),
                Placement::Free(_) => None,
            };
            d.windows
                .iter()
                .map(|w| (w.number, fill(&w.placement)))
                .collect()
        };
        // The order only counts among players sharing a window (who is first in it).
        let order = |d: &Draft| -> Vec<(u8, usize)> {
            let mut by_window: Vec<(u8, usize)> = ordered(&d.order, d.players.len())
                .into_iter()
                .filter_map(|i| d.players.get(i).map(|&n| (n, i)))
                .collect();
            by_window.sort_by_key(|&(n, _)| n);
            by_window
        };
        let same = |a: &Draft, b: &Draft| {
            windows(a) == windows(b)
                && a.players == b.players
                && (a.two, a.three) == (b.two, b.three)
                && order(a) == order(b)
        };
        match (&self.edit, &self.live) {
            (Some(edit), Some(live)) => !same(edit, live),
            (edit, _) => edit.is_some(),
        }
    }

    /// Player `i` to `spot` in the draft.
    fn place(&mut self, i: usize, spot: Spot) {
        let open = self.open_now();
        if let Some(draft) = self.editing() {
            draft.place_player(i, spot);
            draft.drop_empty(&open);
        }
    }

    /// The windows open now: emptied in a draft, they stay until Apply (a window the draft
    /// added goes as soon as nobody is in it).
    fn open_now(&self) -> Vec<u8> {
        self.live
            .as_ref()
            .map_or_else(Vec::new, |l| l.windows.iter().map(|w| w.number).collect())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f32, f32) = (427.0, 300.0);

    fn screen(players: usize) -> SplitUi {
        SplitUi {
            live: Some(Draft::everyone_here(
                players,
                Placement::Free(Some(([0, 0], [2000, 1200]))),
                0,
                0,
            )),
            monitors: vec![
                Monitor {
                    name: "Built-in".into(),
                    at: [0, 0],
                    size: [3024, 1964],
                    scale: 2.0,
                },
                Monitor {
                    name: "TV".into(),
                    at: [3024, 0],
                    size: [1920, 1080],
                    scale: 1.0,
                },
            ],
            main: "Built-in".into(),
            names: (1..=players).map(|n| format!("P{n}")).collect(),
            on: true,
            ..Default::default()
        }
    }

    fn labels(ui: &SplitUi) -> Vec<String> {
        ui.buttons(SIZE.0, SIZE.1)
            .into_iter()
            .map(|b| b.1)
            .collect()
    }

    #[test]
    fn off_only_the_switch_and_done() {
        let mut ui = screen(2);
        ui.on = false;
        assert_eq!(labels(&ui), ["Splitscreen: Off", "Done"]);
        let r = rect(&ui, "Splitscreen: Off");
        assert!(matches!(
            ui.click((r[0] + 1.0, r[1] + 1.0), SIZE, false),
            Click::Switch
        ));
    }

    fn rect(ui: &SplitUi, label: &str) -> [f32; 4] {
        ui.buttons(SIZE.0, SIZE.1)
            .into_iter()
            .find(|(_, l, _, _)| l == label)
            .unwrap_or_else(|| panic!("no {label}"))
            .0
    }

    fn press(ui: &mut SplitUi, label: &str) {
        let r = rect(ui, label);
        ui.click((r[0] + 1.0, r[1] + 1.0), SIZE, false);
    }

    #[test]
    fn move_a_player_then_apply() {
        let mut ui = screen(2);
        press(&mut ui, "P2");
        press(&mut ui, "Fill Monitor 2");
        // Nothing applied yet: the draft only.
        assert_eq!(ui.apply, None);
        assert_eq!(ui.live.as_ref().unwrap().players, [1, 1]);
        press(&mut ui, "Apply");
        let applied = ui.apply.take().unwrap();
        assert_eq!(applied.players, [1, 2]);
        assert_eq!(applied.windows[1].placement, Placement::Fill("TV".into()));
    }

    #[test]
    fn revert_drops_the_draft() {
        let mut ui = screen(2);
        press(&mut ui, "P2");
        press(&mut ui, "New window");
        assert!(ui.edit.is_some());
        press(&mut ui, "Revert");
        assert!(ui.edit.is_none());
        // Done closes and forgets the selection.
        press(&mut ui, "Done");
        assert_eq!(ui.selected, None);
    }
}
