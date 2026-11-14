//! A page drawn as its own screen (`horizon_hooks::pages::Place::PauseFriends`, what the pause
//! menu's vanilla Friends icon opens), the way a mod adds a screen to vanilla: the menu
//! background over the paused world, the title centred at the top, the rows centred down the
//! middle, footer buttons beside Done at the bottom (`HeaderAndFooterLayout`).
use super::{Game, Screen};
use horizon_client::{
    hud::{HudAtlas, HudBatch, rgba},
    multiplayer::wrap,
    text::Text,
    ui,
};
use horizon_hooks::pages::{self, Act, Page, Place, Row};

/// What a click on the screen hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    /// The page's row with this index.
    Row(usize),
    Done,
}

struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: &'static str,
    hit: Hit,
    active: bool,
}

fn grey() -> u32 {
    rgba(160, 160, 160, 255)
}
const WHITE: u32 = u32::MAX;

/// The centred lines (text, colour, y) and the buttons.
fn layout(
    atlas: &HudAtlas,
    (w, h): (f32, f32),
    rows: &[Row],
) -> (Vec<(String, u32, f32)>, Vec<Button>) {
    let width = (w - 40.0).min(310.0);
    let cx = (w / 2.0).floor();
    let mut lines = Vec::new();
    let mut buttons = Vec::new();
    let mut footer = Vec::new();
    let mut y = 40.0;
    // A half-width button waiting for the next to sit beside it.
    let mut half_open = false;
    for (r, row) in rows.iter().enumerate() {
        let beside = std::mem::take(&mut half_open);
        let (text, quiet, groups) = match row {
            Row::Text {
                text,
                quiet,
                groups,
                ..
            } => (text.clone(), *quiet, *groups),
            Row::Note(text) => (text.clone(), true, false),
            Row::Heading(key) => (horizon_client::lang::get(key), true, false),
            Row::Space(px) => {
                y += f32::from(*px);
                continue;
            }
            Row::Button {
                label,
                active,
                footer: true,
                ..
            } => {
                footer.push((r, *label, *active));
                continue;
            }
            Row::Button {
                label,
                active,
                half,
                ..
            } => {
                let (x, w) = match (*half, beside) {
                    (true, true) => {
                        y -= 30.0;
                        (cx + 4.0, 150.0)
                    }
                    (true, false) => {
                        half_open = true;
                        (cx - 154.0, 150.0)
                    }
                    (false, _) => (cx - 100.0, 200.0),
                };
                buttons.push(Button {
                    x,
                    y,
                    w,
                    label,
                    hit: Hit::Row(r),
                    active: *active,
                });
                y += 30.0;
                continue;
            }
            // Settings rows belong in the options shell.
            _ => continue,
        };
        let wrapped = if groups {
            groups_lines(atlas, &text, width)
        } else {
            wrap(&text, width, atlas)
        };
        // Text that would run into the footer is left out.
        if y + 10.0 * wrapped.len() as f32 > h - 33.0 {
            continue;
        }
        let color = if quiet { grey() } else { WHITE };
        for line in wrapped {
            lines.push((line, color, y));
            y += 10.0;
        }
    }
    // The footer: the page's buttons, then Done; Done alone is wider.
    footer.push((usize::MAX, "gui.done", true));
    let (each, gap) = if footer.len() == 1 {
        (200.0, 0.0)
    } else {
        (150.0, 8.0)
    };
    let total = footer.len() as f32 * (each + gap) - gap;
    let mut x = cx - total / 2.0;
    for (r, label, active) in footer {
        let hit = if r == usize::MAX {
            Hit::Done
        } else {
            Hit::Row(r)
        };
        buttons.push(Button {
            x,
            y: h - 27.0,
            w: each,
            label,
            hit,
            active,
        });
        x += each + gap;
    }
    (lines, buttons)
}

/// `-`-separated groups packed into lines of at most `width`, never broken inside a group.
fn groups_lines(atlas: &HudAtlas, text: &str, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for group in text.split('-') {
        match lines.last_mut() {
            Some(l) if HudBatch::text_width(atlas, &format!("{l}-{group}")) <= width => {
                l.push('-');
                l.push_str(group);
            }
            _ => lines.push(group.to_owned()),
        }
    }
    lines
}

fn draw(batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32), title: &str, rows: &[Row]) {
    let (w, h) = (batch.width, batch.height);
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
    let title = Text::translate(title, &[]);
    let tw = atlas.font.width_text(&title);
    batch.text_styled(atlas, &title, ((w - tw) / 2.0).floor(), 15.0, WHITE, true);
    let (lines, buttons) = layout(atlas, (w, h), rows);
    for (line, color, y) in lines {
        let lw = HudBatch::text_width(atlas, &line);
        batch.text(atlas, &line, ((w - lw) / 2.0).floor(), y, color, true);
    }
    for b in buttons {
        let hover = b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0);
        if b.hit == Hit::Done {
            horizon_client::widgets::declare_action(
                horizon_client::widgets::Action::Done,
                [b.x, b.y, b.w, 20.0],
                b.active,
            );
        }
        let label = Text::translate(b.label, &[]);
        ui::button(batch, atlas, b.x, b.y, b.w, 20.0, &label, b.active, hover);
    }
}

/// The active button under a click.
fn click(atlas: &HudAtlas, size: (f32, f32), cursor: (f32, f32), rows: &[Row]) -> Option<Hit> {
    layout(atlas, size, rows)
        .1
        .into_iter()
        .find(|b| b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0))
        .map(|b| b.hit)
}

/// The page the Friends icon opens: the first registered there.
fn friends_page() -> Option<&'static dyn Page> {
    pages::engine::all()
        .iter()
        .copied()
        .find(|page| page.place() == Place::PauseFriends)
}

/// What the Friends icon and its screen are named, for its tooltip, the narrator, controller
/// focus and the control socket's widget tree: the page's title, else vanilla's tooltip.
pub(super) fn friends_name() -> String {
    friends_page().map_or_else(
        || horizon_client::lang::get("gui.friends.open"),
        |page| horizon_client::lang::get(page.title()),
    )
}

impl Game<'_> {
    /// Whether the pause menu's Friends icon is active: a page is registered for it and this
    /// window plays a singleplayer world (another server's world can't be shared).
    pub(super) fn friends_shown(&self) -> bool {
        self.can_share() && friends_page().is_some()
    }

    /// The Friends icon's page. Its rows are asked each time it is drawn.
    pub(super) fn draw_added_page(&mut self, batch: &mut HudBatch) {
        let Some(page) = friends_page() else {
            // Only reached by naming the screen (`horizon-ctl`): back to the pause menu.
            self.screen = Screen::Paused;
            return;
        };
        let mut rows = Vec::new();
        page.rows(&mut rows);
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("friends", friends_name())
        });
        draw(batch, &self.gui.atlas, self.cursor, page.title(), &rows);
    }

    pub(super) fn added_page_click(&mut self) {
        let Some(page) = friends_page() else {
            return;
        };
        let mut rows = Vec::new();
        page.rows(&mut rows);
        let Some(hit) = click(&self.gui.atlas, self.gui_size(), self.cursor, &rows) else {
            return;
        };
        self.audio.play_button_click();
        match hit {
            Hit::Row(r) => page.act(r, Act::Press),
            Hit::Done => self.screen = Screen::Paused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap()
    }

    fn at(buttons: &[Button], hit: Hit) -> &Button {
        buttons.iter().find(|b| b.hit == hit).unwrap()
    }

    #[test]
    fn halves_sit_side_by_side_and_footer_buttons_beside_done() {
        let atlas = atlas();
        let size = (427.0, 240.0);
        let rows = [
            Row::text("Invite Code").quiet(),
            Row::Space(2),
            Row::text("0B33K-Q405Q-1Z5CV-X6NHY-R12N0-VK8T1-60N00-RA134-952PE-1HA08-910P0-K5M01-ST1NB-JDTB6-K4W1P-0").groups(),
            Row::Space(6),
            Row::button("copy").half().active(false),
            Row::button("stop").half(),
            Row::text("Friends on your network or VPN can join. For friends on the internet: turn on UPnP on your router, or forward TCP port 25580 to 10.0.0.2, or use a relay, or a VPN such as Tailscale or ZeroTier (the code already holds your VPN address)."),
            Row::button("help").footer(),
        ];
        let (lines, buttons) = layout(&atlas, size, &rows);
        let (copy, stop) = (at(&buttons, Hit::Row(4)), at(&buttons, Hit::Row(5)));
        // 213 is the middle: two 150-wide halves 8 apart around it, as vanilla's pairs.
        assert_eq!(
            (copy.x, copy.w, stop.x, stop.w),
            (59.0, 150.0, 217.0, 150.0)
        );
        assert_eq!(copy.y, stop.y);
        let (help, done) = (at(&buttons, Hit::Row(7)), at(&buttons, Hit::Done));
        assert_eq!(
            (help.x, done.x, help.y, done.y),
            (59.0, 217.0, 213.0, 213.0)
        );
        // The code breaks between its groups only.
        let code: Vec<_> = lines
            .iter()
            .filter(|(_, c, _)| *c == WHITE)
            .map(|(l, ..)| l)
            .collect();
        assert!(code[0].starts_with("0B33K-") && code.iter().all(|l| !l.ends_with('-')));
        assert!(
            lines.iter().all(|(_, _, y)| y + 9.0 <= size.1 - 27.0),
            "above the footer"
        );
        // An inactive button takes no click; Done alone is 200 wide.
        assert_eq!(
            click(&atlas, size, (copy.x + 1.0, copy.y + 1.0), &rows),
            None
        );
        assert_eq!(
            click(&atlas, size, (stop.x + 1.0, stop.y + 1.0), &rows),
            Some(Hit::Row(5))
        );
        let (_, alone) = layout(&atlas, size, &[Row::button("share")]);
        assert_eq!(at(&alone, Hit::Done).w, 200.0);
        assert_eq!(at(&alone, Hit::Row(0)).x, 113.0);
    }

    #[test]
    fn text_that_would_reach_the_footer_is_left_out() {
        let atlas = atlas();
        let rows = [
            Row::Space(150),
            Row::text("first"),
            Row::text("too low").quiet(),
        ];
        let (lines, _) = layout(&atlas, (427.0, 240.0), &rows);
        // 40 + 150 = 190: one line fits above 207, the next (200..210) doesn't.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0, "first");
    }
}
