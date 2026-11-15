//! The hook a dedicated server's anticheat fills (docs/anticheat.md): the hub tells a [`Watch`] what
//! players send (moves, attacks, swings, the ends of their client ticks) and the movement replay's
//! results, and the watch may drop an action, send the player back, kick or ban. The engine has no
//! checks of its own; the dedicated server sets the platform's (`horizon-anticheat`, its `hub` feature)
//! from `server.toml`. Without a watch (the default, and every LAN world) the
//! hub calls nothing and does exactly what vanilla's server does.
//!
//! The engine keeps the movement record the replay plays again (`recording.rs`): it records while the
//! watch asks for it ([`Watch::records`]).
use super::hub::{HostEvent, Hub, TickCtx};
use crate::lan::prediction::Over;

/// The anticheat's checks, called by the hub with itself (the watch taken out of it for the call).
/// `i` is the guest's index in `hub.guests`.
pub trait Watch: Send + Sync {
    /// Whether the hub records the players' client ticks for the movement replay.
    fn records(&self) -> bool;
    /// How far, in blocks, a replayed tick may land from where the client said it went.
    fn tolerance(&self) -> f64;
    /// The start of a server tick, at `tick_rate` ticks a second.
    fn tick(&mut self, hub: &mut Hub, tick_rate: f32);
    /// ServerboundClientTickEndPacket: one client tick spent (after the hub recorded it).
    fn tick_end(&mut self, hub: &mut Hub, i: usize, events: &mut Vec<HostEvent>);
    /// ServerboundAttackPacket on entity `target`: whether the attack is dropped.
    fn attack(&mut self, hub: &mut Hub, i: usize, target: i32, events: &mut Vec<HostEvent>)
    -> bool;
    /// ServerboundPunchPacket: the arm swung.
    fn swing(&mut self, hub: &mut Hub, i: usize);
    /// A move that says it ends on the ground at `to` (`stands`: something below stopped it): None
    /// when the claim stands or isn't judged, else whether the move is refused (the player was
    /// kicked); `Some(false)` drops the claim.
    #[allow(clippy::too_many_arguments)]
    fn ground(
        &mut self,
        hub: &mut Hub,
        i: usize,
        to: [f64; 3],
        stands: bool,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> Option<bool>;
    /// The replay found guest `conn`'s ticks that landed off by more than [`Watch::tolerance`]; a
    /// setback sends the player to `agreed`, where its moves last agreed. What to log.
    fn prediction(
        &mut self,
        hub: &mut Hub,
        conn: u32,
        over: &Over,
        agreed: [f64; 3],
    ) -> Vec<HostEvent>;
}

impl Hub {
    /// Turn the anticheat on (`Some`) or off.
    pub fn set_anticheat(&mut self, watch: Option<Box<dyn Watch>>) {
        self.anticheat = watch;
    }

    /// Calls the watch with the hub, if there is one; `off` without.
    fn watch<R>(&mut self, off: R, call: impl FnOnce(&mut dyn Watch, &mut Hub) -> R) -> R {
        let Some(mut watch) = self.anticheat.take() else {
            return off;
        };
        let result = call(watch.as_mut(), self);
        self.anticheat = Some(watch);
        result
    }

    /// The start of a server tick.
    pub(super) fn anticheat_tick(&mut self, tick_rate: f32) {
        self.watch((), |w, hub| w.tick(hub, tick_rate));
    }

    /// ServerboundClientTickEndPacket: the record's tick ends, then the watch hears it.
    pub(super) fn anticheat_tick_end(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        if self.anticheat.is_none() {
            return;
        }
        if self.recording() {
            let g = &mut self.guests[i];
            let (input, sprinting) = (g.input, g.client_sprinting);
            g.record.get_or_insert_default().tick_end(input, sprinting);
        }
        self.watch((), |w, hub| w.tick_end(hub, i, events));
    }

    /// ServerboundAttackPacket on entity `target`: whether the watch drops the attack.
    pub(super) fn anticheat_attack(
        &mut self,
        i: usize,
        target: i32,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        self.watch(false, |w, hub| w.attack(hub, i, target, events))
    }

    /// ServerboundPunchPacket: the arm swung.
    pub(super) fn anticheat_swing(&mut self, i: usize) {
        self.watch((), |w, hub| w.swing(hub, i));
    }

    /// [`Watch::ground`] for a move from guest `i` claiming the ground at `to`.
    pub(super) fn anticheat_ground(
        &mut self,
        i: usize,
        to: [f64; 3],
        stands: bool,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> Option<bool> {
        self.watch(None, |w, hub| w.ground(hub, i, to, stands, ctx, events))
    }

    /// How far a replayed tick may land from the client's; infinite without a watch.
    pub fn prediction_tolerance(&self) -> f64 {
        self.anticheat
            .as_deref()
            .map_or(f64::INFINITY, |w| w.tolerance())
    }

    /// The replay's ticks of guest `conn` that landed off: the watch flags them.
    pub fn anticheat_prediction(
        &mut self,
        conn: u32,
        over: &Over,
        agreed: [f64; 3],
    ) -> Vec<HostEvent> {
        self.watch(Vec::new(), |w, hub| w.prediction(hub, conn, over, agreed))
    }

    /// Disconnects `conn` for the anticheat.
    pub fn anticheat_kick(&mut self, conn: u32) {
        let reason = horizon_world::nbt::Tag::compound([
            (
                "text",
                horizon_world::nbt::Tag::String("Kicked by the server's anticheat".into()),
            ),
            ("color", horizon_world::nbt::Tag::String("white".into())),
        ]);
        self.kick(conn, reason);
    }
}
