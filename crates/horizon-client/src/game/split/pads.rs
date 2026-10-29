//! Controllers: which player each drives, joining with one, and one disconnecting (its player
//! waits for another).
use super::*;

/// Giving P1's controller away is pending until the other player enters the world.
/// Cancel, decline or timeout returns that same pad, without taking another player's pad.
pub(super) struct P1Pad<Id> {
    pub(super) owner: Option<Id>,
    released: Option<Id>,
}
impl<Id> Default for P1Pad<Id> {
    fn default() -> Self {
        Self {
            owner: None,
            released: None,
        }
    }
}
impl<Id: Copy + Eq> P1Pad<Id> {
    fn claim(&mut self, id: Id) {
        self.owner = Some(id);
        if self.released == Some(id) {
            self.released = None;
        }
    }
    fn release(&mut self) -> Option<Id> {
        let id = self.owner.take()?;
        self.released = Some(id);
        Some(id)
    }
    fn cancel(&mut self, id: Id) -> bool {
        if self.released != Some(id) {
            return false;
        }
        self.released = None;
        if self.owner.is_some() {
            return false;
        }
        self.owner = Some(id);
        true
    }
    pub(super) fn joined(&mut self, id: Id) {
        if self.released == Some(id) {
            self.released = None;
        }
    }
    fn gone(&mut self, id: Id) {
        if self.owner == Some(id) {
            self.owner = None;
        }
        self.joined(id);
    }
}

/// A player whose controller disconnected, waiting for one: its seat (1 = P1), and the
/// controller's id and kind (`PadRouter::uuid`) to know it back.
pub(super) struct Lost<Id> {
    pub(super) seat: usize,
    pub(super) id: Id,
    pub(super) uuid: [u8; 16],
}

/// What a button on a controller no player has does while players wait for one.
#[derive(Debug, PartialEq)]
pub(super) enum Claim {
    /// It goes to `lost[k]`.
    Give(usize),
    /// `lost[k]` leaves instead.
    Leave(usize),
    /// Nothing: the others wait (no offer to join meanwhile).
    Wait,
    /// Nobody but P1 waits, and P1 has the keyboard: as if nobody did.
    Pass,
}

/// The same kind of controller back (a new id on macOS) takes back its player with any button;
/// another takes over the player waiting longest with A, or lets it leave with B. P1 is only
/// given its own back (another joins as P1 through the offer to join).
pub(super) fn claim<Id>(lost: &[Lost<Id>], uuid: [u8; 16], button: Button) -> Claim {
    if uuid != [0; 16]
        && let Some(k) = lost.iter().position(|l| l.uuid == uuid)
    {
        return Claim::Give(k);
    }
    let Some(k) = lost.iter().position(|l| l.seat != 1) else {
        return Claim::Pass;
    };
    match button {
        Button::South => Claim::Give(k),
        Button::East => Claim::Leave(k),
        _ => Claim::Wait,
    }
}

impl<'a> Split<'a> {
    /// The pad's state for its player: nothing while the press that joined it is still held.
    pub(super) fn pad_state(&mut self, pad: Option<PadId>) -> PadState {
        let Some(id) = pad else {
            return Default::default();
        };
        let state = self.pads.state(id);
        if self.fresh.contains(&id) {
            if state.buttons != 0 {
                return Default::default();
            }
            self.fresh.retain(|&p| p != id);
        }
        state
    }

    /// Controllers: P1's is the first used in a menu; in a world another one is offered to join
    /// (A), or to drive P1 (Y) while P1 has none. The pad whose player should join now.
    pub(super) fn route_pads(&mut self) -> Vec<PadId> {
        let now = Instant::now();
        let owner_before = self.p1_pad.owner;
        let offers_before = self.offers.len();
        let mut joins = Vec::new();
        let mut presses = Vec::new();
        for event in self.pads.events() {
            match event {
                PadEvent::Press(id, button) => presses.push((id, button)),
                PadEvent::Gone(id) => self.pad_gone(id),
                PadEvent::Back(id) => {
                    if let Some(k) = self.lost.iter().position(|l| l.id == id) {
                        let seat = self.lost.remove(k).seat;
                        self.give_pad(seat, id);
                    }
                }
            }
        }
        self.forget_lost();
        for (id, button) in presses {
            if Some(id) == self.p1_pad.owner || self.guests.iter().any(|g| g.pad == Some(id)) {
                continue;
            }
            // Pressed, then gone in the same frame.
            if !self.pads.connected(id) {
                continue;
            }
            // In a menu, or alone in a world, the first controller pressed is P1's (alone it
            // drives P1 anyway); one P1 gave up is offered to join instead.
            let offered = self.offers.iter().any(|(o, _)| *o == id);
            if !self.in_world()
                || (self.guests.is_empty() && self.p1_pad.owner.is_none() && !offered)
            {
                if self.p1_pad.owner.is_none() {
                    self.p1_pad.claim(id);
                    self.fresh.push(id);
                }
                continue;
            }
            if self.claim_for_lost(id, button) {
                continue;
            }
            match self.offers.iter().position(|(o, _)| *o == id) {
                Some(k) => match button {
                    // Two pressing A together both join, in the order they pressed.
                    Button::South => {
                        self.offers.remove(k);
                        joins.push(id);
                    }
                    Button::North if self.p1_pad.owner.is_none() => {
                        self.offers.remove(k);
                        self.p1_pad.claim(id);
                        self.fresh.push(id);
                    }
                    Button::East => {
                        self.offers.remove(k);
                        self.restore_given_up_pad(id);
                    }
                    _ => {}
                },
                // An offer for each controller while seats are left.
                None if self.players() + joins.len() + self.offers.len() < MAX_PLAYERS => {
                    self.offers.push((id, now));
                    self.ask_to_join();
                }
                None => {}
            }
        }
        let in_world = self.in_world();
        let expired: Vec<_> = self
            .offers
            .iter()
            .filter(|(_, since)| now - *since > OFFER || !in_world)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.restore_given_up_pad(id);
        }
        self.offers
            .retain(|(_, since)| now - *since <= OFFER && in_world);
        if self.offers.is_empty() {
            self.prompt_join(None);
        } else if self.offers.len() != offers_before
            || self.p1_pad.owner != owner_before
            || !joins.is_empty()
        {
            self.ask_to_join();
        }
        // Each player's glyphs follow its own controller (its game never reads the pads itself).
        if let Some(id) = self.p1_pad.owner {
            self.host.pad.style = self.pads.style(id);
        }
        for g in &mut self.guests {
            if let Some(id) = g.pad {
                g.game.pad.style = self.pads.style(id);
            }
        }
        // P1 keeps controller output while solo, using the router's assigned physical pad.
        // The router lends its backend for P1's turn; no second reader can claim other pads.
        if self.guests.is_empty() {
            self.host.pad.feed = Some(self.pad_state(self.p1_pad.owner));
            if self.p1_pad.owner.is_none() {
                self.host.pad.release_hardware();
            }
            return joins;
        }
        self.host.pad.release_hardware();
        self.host.pad.feed = Some(self.pad_state(self.p1_pad.owner));
        for i in 0..self.guests.len() {
            let state = self.pad_state(self.guests[i].pad);
            self.guests[i].game.pad.feed = Some(state);
        }
        joins
    }

    /// P1's Give Up Pad: its controller stops driving P1 and is offered to join, for a friend
    /// to press A (P1 plays on with the keyboard and mouse).
    pub(super) fn give_up_pad(&mut self) {
        let Some(id) = self.p1_pad.release() else {
            return;
        };
        self.offers.retain(|(o, _)| *o != id);
        self.offers.push((id, Instant::now()));
        self.host.pad.release_hardware();
        self.host.pad.feed = Some(PadState::default());
        self.ask_to_join();
    }

    pub(super) fn restore_given_up_pad(&mut self, id: PadId) {
        if self.pads.connected(id) && self.p1_pad.cancel(id) {
            // The Cancel/No press belongs to the transfer, not P1's menu.
            self.fresh.push(id);
        }
    }

    /// A controller disconnected: its offer and fresh press go; its player waits for one
    /// (P1 keeps the keyboard and mouse; a player picking a profile leaves).
    pub(super) fn pad_gone(&mut self, id: PadId) {
        self.fresh.retain(|&p| p != id);
        let offered = self.offers.len();
        self.offers.retain(|(p, _)| *p != id);
        if self.offers.len() != offered && !self.offers.is_empty() {
            self.ask_to_join();
        }
        let uuid = self.pads.uuid(id);
        let owned = self.p1_pad.owner == Some(id);
        self.p1_pad.gone(id);
        if owned {
            if self.in_world() {
                self.lost.push(Lost { seat: 1, id, uuid });
                self.host.split_hud.notice = Some(Notice::new(
                    "Controller disconnected",
                    "Reconnect it, or press a button on another",
                ));
            }
            return;
        }
        let Some(g) = self.guests.iter_mut().find(|g| g.pad == Some(id)) else {
            return;
        };
        g.pad = None;
        if g.name.is_none() {
            // Still picking a profile: as if it pressed Cancel (`pick_profiles`).
            g.game.screen = crate::game::Screen::Playing;
            return;
        }
        self.lost.push(Lost {
            seat: g.seat,
            id,
            uuid,
        });
        let name = g.name.as_deref().unwrap_or("New player");
        g.game.split_hud.prompt = Some(lost_prompt(name));
    }

    /// Player in `seat` plays with controller `id` (back, or another): its question goes.
    pub(super) fn give_pad(&mut self, seat: usize, id: PadId) {
        self.fresh.push(id);
        let hud = if seat == 1 {
            self.p1_pad.claim(id);
            &mut self.host.split_hud
        } else {
            let Some(g) = self.guests.iter_mut().find(|g| g.seat == seat) else {
                return;
            };
            g.pad = Some(id);
            &mut g.game.split_hud
        };
        if hud.prompt.as_ref().is_some_and(|p| p.ask == Ask::Lost) {
            hud.prompt = None;
        }
        hud.notice = Some(Notice::new("Controller connected", ""));
    }

    /// A button on a controller nobody has while players wait for one: true when it answered
    /// them (`claim`).
    pub(super) fn claim_for_lost(&mut self, id: PadId, button: Button) -> bool {
        match claim(&self.lost, self.pads.uuid(id), button) {
            Claim::Give(k) => {
                let seat = self.lost.remove(k).seat;
                self.give_pad(seat, id);
            }
            Claim::Leave(k) => {
                let seat = self.lost.remove(k).seat;
                if let Some(g) = self.guests.iter_mut().find(|g| g.seat == seat) {
                    g.game.split_hud.prompt = None;
                    if let Err(e) = g.game.leave_server(None) {
                        log::warn!("Horizon: splitscreen: {e}");
                    }
                }
            }
            Claim::Wait => {}
            Claim::Pass => return false,
        }
        true
    }

    /// Nobody waits for a player who left or has a controller again (another way), nor for P1
    /// out of a world.
    pub(super) fn forget_lost(&mut self) {
        let p1 = self.in_world() && self.p1_pad.owner.is_none();
        let guests = &self.guests;
        self.lost.retain(|l| match l.seat {
            1 => p1,
            seat => guests.iter().any(|g| g.seat == seat && g.pad.is_none()),
        });
    }

    /// Every player sees the controllers waiting to join, whatever it is doing.
    pub(super) fn ask_to_join(&mut self) {
        let title = match self.offers.as_slice() {
            [(id, _)] => format!("{} wants to join", self.pads.name(*id)),
            offers => format!("{} controllers want to join", offers.len()),
        };
        let mut choices = vec![(Button::South, "Join")];
        if self.p1_pad.owner.is_none() {
            choices.push((Button::North, "Play as P1"));
        }
        choices.push((Button::East, "No"));
        self.prompt_join(Some(Prompt {
            ask: Ask::Join,
            title,
            choices,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_transfer_can_be_canceled_without_claiming_an_unrelated_pad() {
        let mut p1 = P1Pad::default();
        p1.claim(7);
        assert_eq!(p1.release(), Some(7));
        assert_eq!(p1.owner, None);
        assert!(!p1.cancel(8));
        assert!(p1.cancel(7));
        assert_eq!(p1.owner, Some(7));
        assert!(!p1.cancel(7));
    }

    #[test]
    fn entering_the_world_commits_the_transfer_and_reclaim_does_not_steal_a_pad() {
        let mut p1 = P1Pad::default();
        p1.claim(7);
        p1.release();
        p1.joined(7);
        assert!(!p1.cancel(7));
        assert_eq!(p1.owner, None);
        p1.claim(7);
        p1.release();
        p1.claim(8);
        assert!(!p1.cancel(7));
        assert_eq!(p1.owner, Some(8));
        p1.release();
        p1.gone(8);
        assert!(!p1.cancel(8));
    }

    const PAD: [u8; 16] = [7; 16];
    const OTHER: [u8; 16] = [9; 16];

    fn lost(seat: usize, uuid: [u8; 16]) -> Lost<usize> {
        Lost {
            seat,
            id: seat,
            uuid,
        }
    }

    #[test]
    fn the_same_controller_back_takes_its_player_with_any_button() {
        let waiting = [lost(2, OTHER), lost(3, PAD)];
        assert_eq!(claim(&waiting, PAD, Button::DPadUp), Claim::Give(1));
        // P1's own comes back too.
        assert_eq!(claim(&[lost(1, PAD)], PAD, Button::West), Claim::Give(0));
    }

    #[test]
    fn another_controller_takes_over_or_lets_go_the_longest_waiting() {
        let waiting = [lost(1, OTHER), lost(3, OTHER), lost(2, OTHER)];
        assert_eq!(claim(&waiting, PAD, Button::South), Claim::Give(1));
        assert_eq!(claim(&waiting, PAD, Button::East), Claim::Leave(1));
        // Any other button: the others wait, nobody is offered to join meanwhile.
        assert_eq!(claim(&waiting, PAD, Button::North), Claim::Wait);
    }

    #[test]
    fn only_p1_waiting_leaves_the_offer_to_join() {
        assert_eq!(claim(&[lost(1, OTHER)], PAD, Button::South), Claim::Pass);
        assert_eq!(claim::<usize>(&[], PAD, Button::South), Claim::Pass);
    }

    #[test]
    fn a_controller_the_system_does_not_describe_is_not_known_back() {
        let unknown = [0; 16];
        assert_eq!(
            claim(&[lost(2, unknown)], unknown, Button::West),
            Claim::Wait
        );
    }
}
