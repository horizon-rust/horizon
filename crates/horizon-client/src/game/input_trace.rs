//! Who caused an input event: a person at the keyboard (`user`), an agent on the
//! control socket (`agent`), or the game itself (`game`, made-up input no one
//! asked for). Agents read this with `input-log` and the `input` section of
//! `state` and `status`, so a script can tell that a person took over instead of
//! mistaking their keystrokes for its own. A `user-input` event also lands in
//! `events` (and so in `wait-for event user-input`) when a person steps in
//! while an agent was driving.
//!
//! The events are also what a repro recording writes to `inputs.jsonl`
//! (game/repro.rs).
use super::{Game, Screen};
use horizon_client::{Result, keybinds};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use winit::{
    event::{ElementState, MouseScrollDelta, WindowEvent},
    keyboard::PhysicalKey,
};

pub(super) const HELP: &[&str] = &[
    "input-log [--since SEQ] [--limit N] [--origin user|agent|game]  recent input and who caused it",
];

/// Input events kept for `input-log`.
const KEPT: usize = 512;
/// A person's input this soon after an agent command is an intrusion.
const INTRUSION_GAP: Duration = Duration::from_secs(5);
/// At most one `user-input` event per this long, so holding a key does not flood.
const INTRUSION_QUIET: Duration = Duration::from_secs(2);

/// Who caused an input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum InputOrigin {
    /// The window's own events: someone at this computer's keyboard, mouse or pad.
    User,
    /// The control socket: `horizon-ctl`, an MCP client, or a script.
    Agent,
    /// Neither: input the game made up itself (pad menus, UI tasks).
    Game,
}

impl InputOrigin {
    pub(super) fn name(self) -> &'static str {
        match self {
            InputOrigin::User => "user",
            InputOrigin::Agent => "agent",
            InputOrigin::Game => "game",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
    fn parse(name: &str) -> Option<Self> {
        match name {
            "user" => Some(InputOrigin::User),
            "agent" => Some(InputOrigin::Agent),
            "game" => Some(InputOrigin::Game),
            _ => None,
        }
    }
}

/// One input event, as `input-log` shows it and the recording writes it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TraceEvent {
    pub seq: u64,
    pub tick: u64,
    /// Milliseconds since the first event this run; the clock a replay keeps.
    pub ms: u64,
    pub origin: InputOrigin,
    #[serde(flatten)]
    pub kind: TraceKind,
}

/// What was pressed, clicked or typed. Key and button names are the game's own
/// (`key.keyboard.w`, `key.mouse.left`), so a recording can feed them back.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum TraceKind {
    Key {
        key: String,
        /// `down`, `up` or `repeat` (the system's auto-repeat).
        action: String,
        /// The text the key wrote, when it wrote any: chat and signs need it.
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Mouse {
        button: String,
        action: String,
    },
    Wheel {
        x: f64,
        y: f64,
        /// The window gave lines (a notched wheel) rather than pixels.
        lines: bool,
    },
    /// The pointer's screen position, kept while a screen is open: GUI clicks
    /// are replayed at it.
    Cursor {
        x: f64,
        y: f64,
    },
    /// Relative mouse motion, which turns the view.
    Motion {
        x: f64,
        y: f64,
    },
    Pad {
        button: String,
        action: String,
    },
    /// A control-socket line: `key`, `mouse`, `goto`, anything an agent ran.
    Command {
        line: String,
    },
}

impl TraceKind {
    pub(super) fn name(&self) -> &'static str {
        match self {
            TraceKind::Key { .. } => "key",
            TraceKind::Mouse { .. } => "mouse",
            TraceKind::Wheel { .. } => "wheel",
            TraceKind::Cursor { .. } => "cursor",
            TraceKind::Motion { .. } => "motion",
            TraceKind::Pad { .. } => "pad",
            TraceKind::Command { .. } => "command",
        }
    }
}

/// The recent input, the counters and the recording's line writer.
#[derive(Default)]
pub(super) struct InputTrace {
    seq: u64,
    log: VecDeque<TraceEvent>,
    counts: [u64; 3],
    last: Option<InputOrigin>,
    last_user: Option<Instant>,
    last_agent: Option<Instant>,
    /// The person's events since the last agent command.
    user_since_agent: u64,
    quiet_until: Option<Instant>,
    /// The first event's wall clock and tick.
    started: Option<(Instant, u64)>,
    /// The recording's `inputs.jsonl` writer; `None` when nothing records.
    writer: Option<Sender<String>>,
    written: u64,
    /// A replay feeds events back: they are not new input and are not recorded.
    suppress: bool,
}

impl InputTrace {
    /// Record one event; whether it is a person intruding on an agent's run.
    fn record(&mut self, tick: u64, origin: InputOrigin, kind: TraceKind) -> bool {
        if self.suppress {
            return false;
        }
        let now = Instant::now();
        let started = self.started.get_or_insert((now, tick)).0;
        let ms = now.duration_since(started).as_millis() as u64;

        let intrusion = origin == InputOrigin::User
            && self
                .last_agent
                .is_some_and(|t| now.duration_since(t) < INTRUSION_GAP)
            && self.quiet_until.is_none_or(|t| now >= t);
        if intrusion {
            self.quiet_until = Some(now + INTRUSION_QUIET);
        }

        self.seq += 1;
        let event = TraceEvent {
            seq: self.seq,
            tick,
            ms,
            origin,
            kind,
        };
        if self.writer.is_some() {
            let mut failed = false;
            match serde_json::to_string(&event) {
                Ok(line) => {
                    if let Some(writer) = &self.writer {
                        failed = writer.send(line).is_err();
                    }
                }
                Err(e) => log::warn!("Horizon: input trace: {e}"),
            }
            if failed {
                self.writer = None;
                log::warn!("Horizon: input trace stopped: its writer is gone");
            } else {
                self.written += 1;
            }
        }
        if self.log.len() == KEPT {
            self.log.pop_front();
        }
        self.log.push_back(event);
        self.counts[origin.index()] += 1;
        self.last = Some(origin);
        match origin {
            InputOrigin::User => {
                self.last_user = Some(now);
                self.user_since_agent += 1;
            }
            InputOrigin::Agent => {
                self.last_agent = Some(now);
                self.user_since_agent = 0;
            }
            InputOrigin::Game => {}
        }
        intrusion
    }

    /// The `input` section of `state` and `status`.
    pub(super) fn summary(&self) -> Value {
        json!({
            "last_origin": self.last.map(InputOrigin::name),
            "counts": {
                "user": self.counts[InputOrigin::User.index()],
                "agent": self.counts[InputOrigin::Agent.index()],
                "game": self.counts[InputOrigin::Game.index()],
            },
            "user_since_agent": self.user_since_agent,
            "last_user_age_s": self
                .last_user
                .map(|t| (t.elapsed().as_secs_f64() * 10.0).round() / 10.0),
            "events": self.seq,
        })
    }

    /// Start writing the events to a recording.
    pub(super) fn start_file(&mut self, writer: Sender<String>) {
        self.writer = Some(writer);
        self.written = 0;
    }
    /// Stop writing; how many events the recording received.
    pub(super) fn stop_file(&mut self) -> u64 {
        self.writer = None;
        std::mem::take(&mut self.written)
    }
    /// How many events this run has seen.
    pub(super) fn events(&self) -> u64 {
        self.seq
    }
    /// A replay's own events are not new input.
    pub(super) fn set_suppress(&mut self, suppress: bool) {
        self.suppress = suppress;
    }

    /// `input-log`.
    pub(super) fn command(&self, words: &[&str]) -> Result<Option<Value>> {
        if words[0] != "input-log" {
            return Ok(None);
        }
        let mut since = 0;
        let mut limit = usize::MAX;
        let mut origin = None;
        let mut i = 1;
        while i < words.len() {
            let value = words
                .get(i + 1)
                .ok_or_else(|| format!("{} takes a value", words[i]))?;
            match words[i] {
                "--since" => since = value.parse().map_err(|_| "--since takes a number")?,
                "--limit" => limit = value.parse().map_err(|_| "--limit takes a number")?,
                "--origin" => {
                    origin = Some(
                        InputOrigin::parse(value).ok_or("--origin takes user, agent or game")?,
                    )
                }
                other => return Err(format!("input-log: unknown argument {other}").into()),
            }
            i += 2;
        }
        let mut events: Vec<&TraceEvent> = self
            .log
            .iter()
            .filter(|e| e.seq > since && origin.is_none_or(|o| e.origin == o))
            .collect();
        if events.len() > limit {
            events.drain(..events.len() - limit);
        }
        Ok(Some(json!({
            "events": events,
            "last": self.seq,
            "input": self.summary(),
        })))
    }
}

impl Game<'_> {
    /// Record an input event and, when a person steps into an agent's run, tell
    /// every watcher once.
    pub(super) fn note_input(&mut self, origin: InputOrigin, kind: TraceKind) {
        let kind_name = kind.name();
        if self.input_trace.record(self.ticks, origin, kind) {
            let event = json!({ "event": "user-input", "input": kind_name });
            self.repro.note_event(self.ticks, &event);
            self.events.note(self.ticks, event);
            log::info!(
                "Horizon: {kind_name} input from a person while an agent was driving \
                 (input-log has the run)"
            );
        }
    }

    /// Keep the window events that are input. Agent-made window events are left
    /// out: the control-socket line that caused them is recorded instead.
    pub(super) fn trace_window_event(&mut self, origin: InputOrigin, event: &WindowEvent) {
        if origin == InputOrigin::Agent {
            return;
        }
        match event {
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let Some(name) = keybinds::key_name(code) else {
                    return;
                };
                let action = match (event.state, event.repeat) {
                    (ElementState::Released, _) => "up",
                    (_, true) => "repeat",
                    (ElementState::Pressed, false) => "down",
                };
                let text = event
                    .text
                    .as_ref()
                    .map(ToString::to_string)
                    .filter(|t| !t.is_empty());
                // The platform made this one up (an IME's dead key): not a person.
                let origin = if *is_synthetic {
                    InputOrigin::Game
                } else {
                    origin
                };
                self.note_input(
                    origin,
                    TraceKind::Key {
                        key: name.to_owned(),
                        action: action.to_owned(),
                        text,
                    },
                );
            }
            WindowEvent::MouseInput { state, button, .. } => self.note_input(
                origin,
                TraceKind::Mouse {
                    button: keybinds::mouse_name(*button),
                    action: match state {
                        ElementState::Pressed => "down".to_owned(),
                        ElementState::Released => "up".to_owned(),
                    },
                },
            ),
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y, lines) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(*x), f64::from(*y), true),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y, false),
                };
                self.note_input(origin, TraceKind::Wheel { x, y, lines });
            }
            // GUI clicks land where the pointer is; in the world the pointer is
            // grabbed and turns the view through `device_event` instead.
            WindowEvent::CursorMoved { position, .. } if self.screen != Screen::Playing => {
                self.note_input(
                    origin,
                    TraceKind::Cursor {
                        x: position.x,
                        y: position.y,
                    },
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> TraceKind {
        TraceKind::Key {
            key: name.to_owned(),
            action: "down".to_owned(),
            text: None,
        }
    }

    #[test]
    fn origins_round_trip() {
        for origin in [InputOrigin::User, InputOrigin::Agent, InputOrigin::Game] {
            assert_eq!(InputOrigin::parse(origin.name()), Some(origin));
        }
        assert_eq!(InputOrigin::parse("nobody"), None);
    }

    #[test]
    fn the_ring_keeps_the_newest_and_counts_every_origin() {
        let mut trace = InputTrace::default();
        for i in 0..=(KEPT as u64 + 10) {
            trace.record(i, InputOrigin::User, key("key.keyboard.w"));
        }
        assert_eq!(trace.log.len(), KEPT);
        assert_eq!(trace.seq, KEPT as u64 + 11);
        assert_eq!(trace.log.front().unwrap().seq, 12);
        assert_eq!(trace.counts[InputOrigin::User.index()], KEPT as u64 + 11);
    }

    #[test]
    fn a_person_after_an_agent_is_an_intrusion_once() {
        let mut trace = InputTrace::default();
        trace.record(1, InputOrigin::Agent, key("key.keyboard.w"));
        assert!(trace.record(2, InputOrigin::User, key("key.keyboard.a")));
        // The quiet period keeps a held key from flooding the event log.
        assert!(!trace.record(3, InputOrigin::User, key("key.keyboard.a")));
        assert_eq!(trace.user_since_agent, 2);
        // An agent command resets the count.
        trace.record(4, InputOrigin::Agent, key("key.keyboard.w"));
        assert_eq!(trace.user_since_agent, 0);
    }

    #[test]
    fn no_intrusion_without_an_agent() {
        let mut trace = InputTrace::default();
        assert!(!trace.record(1, InputOrigin::User, key("key.keyboard.w")));
        // The game's own input is not a person either.
        trace.record(2, InputOrigin::Agent, key("key"));
        assert!(!trace.record(3, InputOrigin::Game, key("key.keyboard.w")));
    }

    #[test]
    fn input_log_filters_since_origin_and_limit() {
        let mut trace = InputTrace::default();
        for (i, origin) in [
            InputOrigin::Agent,
            InputOrigin::User,
            InputOrigin::Game,
            InputOrigin::User,
        ]
        .into_iter()
        .enumerate()
        {
            trace.record(i as u64, origin, key("key.keyboard.w"));
        }
        let all = trace.command(&["input-log"]).unwrap().unwrap();
        assert_eq!(all["events"].as_array().unwrap().len(), 4);
        assert_eq!(all["last"], 4);

        let users = trace
            .command(&["input-log", "--origin", "user"])
            .unwrap()
            .unwrap();
        assert_eq!(users["events"].as_array().unwrap().len(), 2);
        assert!(
            users["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["origin"] == "user")
        );

        let newest = trace
            .command(&["input-log", "--since", "2", "--limit", "1"])
            .unwrap()
            .unwrap();
        let events = newest["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["seq"], 4);
    }

    #[test]
    fn the_recording_writer_gets_one_json_line_per_event() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut trace = InputTrace::default();
        trace.start_file(tx);
        trace.record(7, InputOrigin::User, key("key.keyboard.w"));
        trace.record(8, InputOrigin::Agent, key("key.keyboard.s"));
        assert_eq!(trace.stop_file(), 2);
        let lines: Vec<String> = rx.try_iter().collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(first["tick"], 7);
        assert_eq!(first["origin"], "user");
        assert_eq!(first["kind"], "key");
        assert_eq!(first["key"], "key.keyboard.w");
    }

    #[test]
    fn a_suppressed_replay_records_nothing() {
        let mut trace = InputTrace::default();
        trace.set_suppress(true);
        assert!(!trace.record(1, InputOrigin::User, key("key.keyboard.w")));
        assert_eq!(trace.seq, 0);
        assert!(trace.summary()["last_origin"].is_null());
    }
}
