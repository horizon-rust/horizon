//! Bounded local sound requests. Only the native client supplies event IDs and context.
use crate::AudioCategory;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

pub struct Request {
    pub owner: String,
    pub owner_generation: u64,
    pub event: String,
    pub position: Option<[f64; 3]>,
    pub category: AudioCategory,
    pub volume: f32,
    pub pitch: f32,
}
struct Context {
    session: u64,
    available: bool,
    events: Arc<HashSet<String>>,
}
#[derive(Default)]
pub(crate) struct Broker {
    context: Option<Context>,
    queue: VecDeque<Request>,
}
pub(crate) type Shared = Arc<Mutex<Broker>>;
impl Broker {
    pub(crate) fn context(
        &mut self,
        session: Option<u64>,
        available: bool,
        events: Arc<HashSet<String>>,
    ) {
        if self.context.as_ref().map(|c| c.session) != session
            || !available
            || self
                .context
                .as_ref()
                .is_some_and(|c| !Arc::ptr_eq(&c.events, &events))
        {
            self.queue.clear();
        }
        self.context = session.map(|session| Context {
            session,
            available,
            events,
        });
    }
    pub(crate) fn play(&mut self, request: Request) -> Result<(), String> {
        let event = &request.event;
        let valid_id = event.split_once(':').is_some_and(|(namespace, path)| {
            !namespace.is_empty()
                && !path.is_empty()
                && namespace
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
                && path
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/_.-".contains(&b))
                && path
                    .split('/')
                    .all(|p| !p.is_empty() && p != "." && p != "..")
        });
        if event.len() > 128
            || !valid_id
            || !request.volume.is_finite()
            || !(0.0..=1.0).contains(&request.volume)
            || !request.pitch.is_finite()
            || !(0.5..=2.0).contains(&request.pitch)
            || request.position.is_some_and(|p| {
                p.into_iter()
                    .any(|v| !v.is_finite() || v.abs() > 30_000_000.0)
            })
        {
            return Err("invalid audio event or bounded playback parameters".into());
        }
        let context = self
            .context
            .as_ref()
            .ok_or("audio requires an active client world")?;
        if !context.available {
            return Err("native audio output is disabled or unavailable".into());
        }
        if !context.events.contains(event) {
            return Err("sound event is not registered in active resources".into());
        }
        if self.queue.len() >= 64 {
            return Err("client program sound queue is full".into());
        }
        self.queue.push_back(request);
        Ok(())
    }
    pub(crate) fn drain(&mut self) -> Vec<Request> {
        self.queue.drain(..).collect()
    }
}
