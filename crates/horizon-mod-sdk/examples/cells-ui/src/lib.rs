//! Example Horizon client mod (`docs/mods.md`, "Network"): the client half of
//! the `cells` server mod, with the same id. The server's `/cellui` sends
//! `cells/open` and then `cells/state` messages (`x y z stored capacity in
//! out`, as text); this half draws them as a screen of its own layout
//! (capability `gui-screens`) — a machine panel of the mod's own pictures
//! (`assets/cells/textures/gui/sprites/panel/`, carried by capability
//! `registries`): the charge as text, a bar that fills with it, the redstone
//! waiting and made, and buttons and a menu — and sends `cells/action`
//! (`fill`, `drain`, `close`) back for what the player presses. The server
//! decides what each does; this half only shows and asks.
use horizon_mod_sdk::{
    Mod, Registration, gui_screens,
    gui_screens::{
        Background, ButtonWidget, FreeScreen, ImageWidget, ItemWidget, ListWidget, Point, Rect,
        TextWidget, Widget,
    },
    network,
};

/// The screen's widgets by id: the buttons and the menu the player presses.
const FILL: u32 = 1;
const DRAIN: u32 = 2;
const MENU: u32 = 3;

/// What the server last said of the cell.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    x: i32,
    y: i32,
    z: i32,
    stored: u64,
    capacity: u64,
    input: u32,
    output: u32,
}

/// `x y z stored capacity in out`; `None` for anything else.
fn parse(data: &[u8]) -> Option<Cell> {
    let text = std::str::from_utf8(data).ok()?;
    let mut f = text.split(' ');
    let cell = Cell {
        x: f.next()?.parse().ok()?,
        y: f.next()?.parse().ok()?,
        z: f.next()?.parse().ok()?,
        stored: f.next()?.parse().ok()?,
        capacity: f.next()?.parse().ok()?,
        input: f.next()?.parse().ok()?,
        output: f.next()?.parse().ok()?,
    };
    (f.next().is_none() && cell.capacity > 0).then_some(cell)
}

/// How far across the bar the charge reaches: the part of `BAR_W` it fills.
fn bar_w(stored: u64, capacity: u64) -> f32 {
    let stored = stored.min(capacity) as f32;
    BAR_W * stored / capacity as f32
}

/// Where the panel's own furniture sits, in GUI pixels from its top left.
const TITLE_Y: f32 = 7.0;
const CHARGE_Y: f32 = 19.0;
const BAR_X: f32 = 30.0;
const BAR_Y: f32 = 31.0;
const BAR_W: f32 = 116.0;
const BAR_H: f32 = 7.0;
const ROW_Y: f32 = 43.0;
const MENU_Y: f32 = 62.0;
const BUTTON_W: f32 = 78.0;
const BUTTON_H: f32 = 16.0;

/// The free screen for a cell: a 176 × 96 machine panel of the mod's own
/// background picture, the widgets where the panel wants them.
fn screen(c: &Cell) -> FreeScreen {
    let p = |x: f32, y: f32| Point { x, y };
    let r = |x: f32, y: f32, w: f32, h: f32| Rect { x, y, w, h };
    let text = |id: u32, text: String, at: Point, color: u32| {
        Widget::Text(TextWidget {
            id,
            text,
            at,
            color,
            centered: true,
            shadow: true,
        })
    };
    FreeScreen {
        width: 176.0,
        height: 96.0,
        background: Background {
            sprite: Some("cells:panel/cell".into()),
            // What shows where the picture is missing (a sandbox, a pack
            // without it): the panel's own dark fill.
            color: 0xFF26_2626,
        },
        widgets: vec![
            text(
                10,
                format!("Energy Cell {} {} {}", c.x, c.y, c.z),
                p(88.0, TITLE_Y),
                0xFFAA00,
            ),
            text(
                11,
                format!("{} / {} FE", c.stored, c.capacity),
                p(88.0, CHARGE_Y),
                0xFFFFFF,
            ),
            // The bar: its track the whole width, the fill as far as the
            // charge reaches, both the same picture tinted.
            Widget::Image(ImageWidget {
                id: 15,
                sprite: "cells:panel/bar".into(),
                rect: r(BAR_X, BAR_Y, BAR_W, BAR_H),
                color: 0xFF3A_3A3A,
            }),
            Widget::Image(ImageWidget {
                id: 12,
                sprite: "cells:panel/bar".into(),
                rect: r(BAR_X, BAR_Y, bar_w(c.stored, c.capacity), BAR_H),
                color: 0xFF70_E070,
            }),
            Widget::Item(ItemWidget {
                id: 13,
                item: "minecraft:redstone".into(),
                at: p(8.0, ROW_Y),
                scale: 1.0,
            }),
            Widget::Text(TextWidget {
                id: 14,
                text: format!("{} waiting, {} made", c.input, c.output),
                at: p(26.0, ROW_Y + 4.0),
                color: 0xA0A0A0,
                centered: false,
                shadow: true,
            }),
            // The menu does what the buttons do, for a player who prefers
            // rows; its rows are the server's three actions.
            Widget::List(ListWidget {
                id: MENU,
                rect: r(8.0, MENU_Y, BUTTON_W, 30.0),
                rows: vec!["Fill".into(), "Drain".into(), "Close".into()],
                selected: Some(0),
            }),
            Widget::Button(ButtonWidget {
                id: FILL,
                label: "Fill 1000 FE".into(),
                rect: r(90.0, MENU_Y, BUTTON_W, BUTTON_H),
            }),
            Widget::Button(ButtonWidget {
                id: DRAIN,
                label: "Drain 1000 FE".into(),
                rect: r(90.0, MENU_Y + 18.0, BUTTON_W, BUTTON_H),
            }),
        ],
    }
}

#[derive(Default)]
struct CellsUi {
    /// The cell on the screen, while one is open.
    cell: Option<Cell>,
}

impl Mod for CellsUi {
    fn init() -> Result<(Self, Registration), String> {
        Ok((CellsUi::default(), Registration::new()))
    }

    fn network_received(&mut self, channel: &str, data: &[u8]) {
        match channel {
            // The server opened a screen, or its state changed: a second
            // `open` while one shows replaces it, so one call serves both,
            // with the new numbers.
            "cells/open" | "cells/state" => {
                if channel == "cells/state" && self.cell.is_none() {
                    // A late update after the player left the screen.
                    return;
                }
                let Some(cell) = parse(data) else { return };
                gui_screens::open(&screen(&cell));
                self.cell = Some(cell);
            }
            // The cell the screen showed is gone.
            "cells/gone" => {
                self.cell = None;
                gui_screens::close();
            }
            _ => {}
        }
    }

    /// A button: asks the server, which answers with the new state.
    fn gui_clicked(&mut self, id: u32) {
        let action: &[u8] = match id {
            FILL => b"fill",
            DRAIN => b"drain",
            _ => return,
        };
        let _ = network::send("cells/action", action);
    }

    /// A row of the menu: the same actions, by their place in it.
    fn gui_picked(&mut self, id: u32, row: u32) {
        let action: &[u8] = match (id, row) {
            (MENU, 0) => b"fill",
            (MENU, 1) => b"drain",
            (MENU, 2) => b"close",
            _ => return,
        };
        let _ = network::send("cells/action", action);
    }

    /// Esc: the server stops sending states.
    fn gui_closed(&mut self) {
        self.cell = None;
        let _ = network::send("cells/action", b"close");
    }
}

horizon_mod_sdk::export_mod!(CellsUi);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_parse_and_nothing_else_does() {
        let c = parse(b"-38 64 -8 5000 100000 2 3").unwrap();
        assert_eq!(
            (c.x, c.y, c.z, c.stored, c.input, c.output),
            (-38, 64, -8, 5000, 2, 3)
        );
        for bad in [
            "",
            "1 2 3",
            "1 2 3 4 0 0 0",
            "1 2 3 4 5 6 7 8",
            "a b c d e f g",
        ] {
            assert_eq!(parse(bad.as_bytes()), None, "{bad:?}");
        }
        assert_eq!(parse(&[0xFF, 0xFE]), None);
    }

    #[test]
    fn the_bar_fills_by_the_charge() {
        assert_eq!(bar_w(0, 100), 0.0);
        assert_eq!(bar_w(25, 100), BAR_W / 4.0);
        assert_eq!(bar_w(100, 100), BAR_W);
        assert_eq!(bar_w(200, 100), BAR_W, "over capacity stays full");
    }

    #[test]
    fn the_panel_lays_out_from_a_cell() {
        let c = parse(b"0 64 0 25000 100000 1 0").unwrap();
        let s = screen(&c);
        assert_eq!((s.width, s.height), (176.0, 96.0));
        assert_eq!(s.background.sprite.as_deref(), Some("cells:panel/cell"));
        // The title and the charge, centred on the panel.
        let Widget::Text(title) = &s.widgets[0] else {
            panic!("{:?}", s.widgets[0])
        };
        assert_eq!(title.text, "Energy Cell 0 64 0");
        assert!(title.centered);
        let Widget::Text(charge) = &s.widgets[1] else {
            panic!("{:?}", s.widgets[1])
        };
        assert_eq!(charge.text, "25000 / 100000 FE");
        // The bar's track the whole width, its fill a quarter at 25%.
        let images: Vec<&ImageWidget> = s
            .widgets
            .iter()
            .filter_map(|w| match w {
                Widget::Image(i) => Some(i),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].rect.w, BAR_W);
        assert_eq!(images[1].rect.w, BAR_W / 4.0);
        // Two buttons beside a three-row menu, all inside the panel.
        let buttons: Vec<&ButtonWidget> = s
            .widgets
            .iter()
            .filter_map(|w| match w {
                Widget::Button(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(
            buttons.iter().map(|b| b.id).collect::<Vec<_>>(),
            [FILL, DRAIN]
        );
        let Widget::List(menu) = &s.widgets[6] else {
            panic!("{:?}", s.widgets[6])
        };
        assert_eq!(menu.rows, ["Fill", "Drain", "Close"]);
        for w in &s.widgets {
            let r = match w {
                Widget::Text(t) => Rect {
                    x: t.at.x,
                    y: t.at.y,
                    w: 1.0,
                    h: 1.0,
                },
                Widget::Button(b) => b.rect,
                Widget::Item(i) => Rect {
                    x: i.at.x,
                    y: i.at.y,
                    w: 1.0,
                    h: 1.0,
                },
                Widget::Image(i) => i.rect,
                Widget::List(l) => l.rect,
            };
            assert!(r.x >= 0.0 && r.y >= 0.0, "{r:?} outside the panel");
            assert!(
                r.x + r.w <= s.width && r.y + r.h <= s.height,
                "{r:?} outside the panel"
            );
        }
    }
}
