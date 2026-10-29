//! P1's Setup screen's side in the game: the arrangement as a draft, and Apply.
use super::*;

impl<'a> Split<'a> {
    /// The arrangement now, for the Setup screen: free windows where they are.
    pub(super) fn draft(&self) -> layout::Draft {
        let windows = self
            .windows
            .iter()
            .map(|w| {
                let here = self.surfaces.iter().find(|s| s.key == w.number);
                let placement = match (&w.placement, here) {
                    (Placement::Free(_), Some(s)) => match s.window.outer_position() {
                        Ok(at) => {
                            // On the desktop as windows are placed: macOS in points.
                            let k = if cfg!(target_os = "macos") {
                                s.window.scale_factor()
                            } else {
                                1.0
                            };
                            let at = [(f64::from(at.x) / k) as i32, (f64::from(at.y) / k) as i32];
                            Placement::Free(Some((at, [s.size.width, s.size.height])))
                        }
                        Err(_) => w.placement.clone(),
                    },
                    (p, _) => p.clone(),
                };
                layout::Window {
                    number: w.number,
                    placement,
                }
            })
            .collect();
        let e = &self.host.settings.extras;
        layout::Draft {
            windows,
            players: (0..self.players()).map(|i| self.window_number(i)).collect(),
            two: e.splitscreen_two,
            three: e.splitscreen_three,
            order: self.player_order(),
        }
    }

    /// The monitors for the Setup screen, left to right (as `monitor_label` numbers them), and
    /// the one Window 1 is on.
    pub(super) fn setup_monitors(&self) -> (Vec<layout::Monitor>, String) {
        let monitors = self
            .monitors()
            .iter()
            .map(|(name, m)| layout::Monitor {
                name: name.clone(),
                at: [m.position().x, m.position().y],
                size: [m.size().width, m.size().height],
                scale: m.scale_factor(),
            })
            .collect();
        (monitors, self.main_monitor().cloned().unwrap_or_default())
    }

    /// The Setup screen applied: its windows, and every player in its window. Windows nobody
    /// plays in close (`forget_empty_windows`).
    pub(super) fn apply(&mut self, draft: layout::Draft) {
        let mut windows: Vec<layout::Window> = draft
            .windows
            .into_iter()
            .map(|w| {
                // A free window stays where it is; a new one opens where the system puts it.
                let placement = match (&w.placement, self.placement(w.number)) {
                    (Placement::Free(_), Some(Placement::Free(p))) => Placement::Free(*p),
                    (Placement::Free(_), _) => Placement::Free(None),
                    (p, _) => p.clone(),
                };
                layout::Window {
                    number: w.number,
                    placement,
                }
            })
            .collect();
        if !windows.iter().any(|w| w.number == 1) {
            windows.insert(
                0,
                layout::Window {
                    number: 1,
                    placement: Placement::Free(None),
                },
            );
        }
        self.windows = windows;
        // Players who joined while it was open stay where they are.
        for (i, &number) in draft.players.iter().enumerate().take(self.players()) {
            let number = if self.placement(number).is_some() {
                number
            } else {
                1
            };
            self.set_window(i, number);
        }
        // Who is first in a shared window, by seat (players are by index in the draft).
        let n = self.players();
        self.order = layout::ordered(&draft.order, n)
            .into_iter()
            .map(|i| self.who(i).0)
            .collect();
        // P1's options file: its turn.
        self.turn(0);
        let e = &mut self.host.settings.extras;
        if (e.splitscreen_two, e.splitscreen_three) != (draft.two, draft.three) {
            e.splitscreen_two = draft.two;
            e.splitscreen_three = draft.three;
            self.host.settings.save(&Game::options_path());
        }
    }

    /// P1's Setup screen: the arrangement now, the monitors and the players' names; Apply
    /// arranges them. True when it did.
    pub(super) fn sync_setup(&mut self) -> bool {
        use horizon_client::options_ui::Page;
        if self.host.ui.options.page() != Page::SplitSetup {
            return false;
        }
        let live = self.draft();
        let (monitors, main) = self.setup_monitors();
        let names: Vec<String> = (0..self.players())
            .map(|i| {
                let (seat, who) = self.who(i);
                if self.lost.iter().any(|l| l.seat == seat) {
                    format!("{who} (no controller)")
                } else {
                    who
                }
            })
            .collect();
        let seats: Vec<usize> = (0..self.players()).map(|i| self.who(i).0).collect();
        let moved = std::mem::replace(&mut self.setup_seats, seats) != self.setup_seats;
        let setup = &mut self.host.ui.options.split_setup;
        // Someone joined or left: the draft would put the players in the wrong places.
        if moved {
            setup.reset();
        }
        let applied = setup.apply.take();
        let identify = std::mem::take(&mut setup.identify);
        if std::mem::take(&mut setup.toggle_names) {
            let e = &mut self.host.settings.extras;
            e.splitscreen_names = !e.splitscreen_names;
            self.host.settings.save(&Game::options_path());
        }
        let setup = &mut self.host.ui.options.split_setup;
        setup.names_shown = self.host.settings.extras.splitscreen_names;
        setup.live = Some(live);
        setup.monitors = monitors;
        setup.main = main;
        setup.names = names;
        let changed = applied.is_some();
        if let Some(draft) = applied {
            self.apply(draft);
        }
        // Every view says where it is: after a change, or asked.
        if changed || identify {
            for i in 0..self.players() {
                self.tag(i, None);
            }
        }
        changed
    }
}
