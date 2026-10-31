//! Controllers while several play: one reader for every player's game (`PadRouter`), each pad
//! driving one player. Alone, P1 reads the controllers itself, as without splitscreen.
use super::*;
use crate::game::pad::{PadEvent, PadId};
use horizon_client::gamepad::PadState;

/// The player a controller nobody has goes to when pressed: the first without one (P1 first).
pub(super) fn first_without<Id>(pads: &[Option<Id>]) -> Option<usize> {
    pads.iter().position(Option::is_none)
}

impl Split<'_> {
    /// Each player's pad, P1's first.
    fn pads_of(&self) -> Vec<Option<PadId>> {
        std::iter::once(self.p1_pad)
            .chain(self.guests.iter().map(|g| g.pad))
            .collect()
    }

    fn set_pad(&mut self, i: usize, pad: Option<PadId>) {
        match i {
            0 => self.p1_pad = pad,
            _ => self.guests[i - 1].pad = pad,
        }
    }

    /// The pad's state for its player: nothing while the press that gave it is still held.
    fn pad_state(&mut self, pad: Option<PadId>) -> PadState {
        let Some(id) = pad else {
            return Default::default();
        };
        let state = self.pads.state(id);
        if self.fresh.contains(&id) {
            if pressing(&state) {
                return Default::default();
            }
            self.fresh.retain(|&p| p != id);
        }
        state
    }

    /// Several players: the controllers' events once, a pressed controller nobody has to the
    /// first player without one, a disconnected one's player without, and each player's game
    /// fed its own pad's state and glyphs.
    pub(super) fn route_pads(&mut self) {
        // P1's own reader closes: every pad goes through the split's now.
        self.host.pad.release_hardware();
        for event in self.pads.events() {
            match event {
                PadEvent::Press(id) => {
                    let pads = self.pads_of();
                    if pads.contains(&Some(id)) || !self.pads.connected(id) {
                        continue;
                    }
                    if let Some(i) = first_without(&pads) {
                        self.set_pad(i, Some(id));
                        self.fresh.push(id);
                    }
                }
                PadEvent::Gone(id) => {
                    if let Some(i) = self.pads_of().iter().position(|&p| p == Some(id)) {
                        self.set_pad(i, None);
                    }
                    self.fresh.retain(|&p| p != id);
                }
            }
        }
        let state = self.pad_state(self.p1_pad);
        self.host.pad.feed = Some(state);
        if let Some(id) = self.p1_pad {
            self.host.pad.style = self.pads.style(id);
        }
        for i in 0..self.guests.len() {
            let state = self.pad_state(self.guests[i].pad);
            let style = self.guests[i].pad.map(|id| self.pads.style(id));
            let game = &mut self.guests[i].game;
            game.pad.feed = Some(state);
            if let Some(style) = style {
                game.pad.style = style;
            }
        }
    }

    /// Alone again: P1 reads its own controllers; the split's reader closes.
    pub(super) fn stop_routing(&mut self) {
        self.host.pad.feed = None;
        self.pads = Default::default();
        self.p1_pad = None;
        self.fresh.clear();
    }
}

/// Still holding the press that gave the pad its player: a button, or a trigger off its rest
/// (gilrs reports a trigger press as a button, but `PadState` keeps it in `triggers`).
fn pressing(state: &PadState) -> bool {
    state.buttons != 0 || state.triggers.iter().any(|&t| t > 0.05)
}

#[cfg(test)]
mod tests {
    use super::{first_without, pressing};
    use horizon_client::gamepad::PadState;

    #[test]
    fn the_trigger_that_gave_a_pad_is_held_back_too() {
        assert!(!pressing(&PadState::default()));
        let trigger = PadState {
            triggers: [0.0, 0.8],
            ..Default::default()
        };
        assert!(pressing(&trigger), "a held trigger would attack or use");
        let button = PadState {
            buttons: 1,
            ..Default::default()
        };
        assert!(pressing(&button));
    }

    #[test]
    fn a_new_controller_goes_to_the_first_player_without_one() {
        assert_eq!(first_without::<u8>(&[None, None]), Some(0));
        assert_eq!(first_without(&[Some(1), None, None]), Some(1));
        assert_eq!(first_without(&[Some(1), Some(2), None]), Some(2));
        // P2 lost its controller: the next new one is P2's, not P3's.
        assert_eq!(first_without(&[Some(1), None, Some(3), None]), Some(1));
        assert_eq!(first_without(&[Some(1), Some(2)]), None);
    }
}
