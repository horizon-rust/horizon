//! Bounded program delivery leases. The game supplies players and connection routes.
use crate::ProgramMessage;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
pub const PAYLOAD: usize = 32768;
pub const QUEUE: usize = 64;
static EPOCH: AtomicU64 = AtomicU64::new(1);
pub fn new_epoch() -> Result<u64, String> {
    EPOCH
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map_err(|_| "message identities exhausted".into())
}
pub fn owner_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
}
pub fn channel_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.:/-".contains(&c))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub receiver: String,
    pub epoch: u64,
    pub sender: String,
    pub channel: String,
}
#[derive(Clone)]
pub struct Route {
    pub player: String,
    pub subscription: Subscription,
}
#[derive(Clone)]
pub struct Delivery {
    pub player: String,
    pub receiver: String,
    pub receiver_epoch: u64,
    pub sender: String,
    pub channel: String,
    pub payload: Arc<[u8]>,
    pub world: u64,
}
#[derive(Default)]
pub struct Broker {
    pub(crate) subscriptions: BTreeMap<u64, Vec<Subscription>>,
    inbox_counts: BTreeMap<u64, usize>,
    queued: BTreeMap<u64, Vec<String>>,
    pub routes: Vec<Route>,
    pub local_player: Option<String>,
}
pub type Shared = Arc<Mutex<Broker>>;
#[derive(Clone, Default)]
struct Live {
    subscriptions: Vec<(String, String)>,
    inbox: VecDeque<Arc<ProgramMessage>>,
    outbox: VecDeque<Delivery>,
}
pub(crate) struct Session {
    id: String,
    server: bool,
    owner: u64,
    epoch: u64,
    world: Option<u64>,
    shared: Shared,
    live: Live,
    before: Option<Live>,
    calls: u32,
    sends: u32,
}
impl Session {
    pub(crate) fn new(id: String, server: bool, shared: Shared) -> Result<Self, String> {
        let owner = new_epoch()?;
        Ok(Self {
            id,
            server,
            owner,
            epoch: new_epoch()?,
            world: None,
            shared,
            live: Live::default(),
            before: None,
            calls: 0,
            sends: 0,
        })
    }
    fn publish(&self) {
        let mut broker = self.shared.lock().unwrap();
        broker.subscriptions.insert(
            self.owner,
            if self.server {
                Vec::new()
            } else {
                self.live
                    .subscriptions
                    .iter()
                    .map(|(sender, channel)| Subscription {
                        receiver: self.id.clone(),
                        epoch: self.epoch,
                        sender: sender.clone(),
                        channel: channel.clone(),
                    })
                    .collect()
            },
        );
        broker
            .inbox_counts
            .insert(self.owner, self.live.inbox.len());
        broker.queued.insert(
            self.owner,
            self.live.outbox.iter().map(|d| d.player.clone()).collect(),
        );
    }
    pub(crate) fn begin(&mut self) {
        self.before = Some(self.live.clone());
        self.calls = 0;
        self.sends = 0;
    }
    pub(crate) fn finish(&mut self, success: bool) {
        if let Some(before) = self.before.take() {
            if !success {
                self.live = before;
            }
        }
        self.publish();
    }
    pub(crate) fn context(&mut self, world: Option<u64>) -> Result<(), String> {
        if world != self.world {
            self.live.inbox.clear();
            self.live.outbox.clear();
            self.before = None;
            self.world = world;
            self.epoch = new_epoch()?;
            self.publish();
        }
        Ok(())
    }
    fn call(&mut self) -> Result<(), String> {
        self.calls += 1;
        if self.calls > 64 {
            Err("message callback call budget exceeded".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn subscribe(&mut self, sender: String, channel: String) -> Result<(), String> {
        self.call()?;
        if self.server {
            return Err("messages.subscribe is client-only".into());
        }
        if !owner_name(&sender) || !channel_name(&channel) {
            return Err("invalid message sender/channel".into());
        }
        if self
            .live
            .subscriptions
            .contains(&(sender.clone(), channel.clone()))
        {
            return Ok(());
        }
        let broker = self.shared.lock().unwrap();
        let others: usize = broker
            .subscriptions
            .iter()
            .filter(|(owner, _)| **owner != self.owner)
            .map(|(_, v)| v.len())
            .sum();
        if self.live.subscriptions.len() >= 32 || others + self.live.subscriptions.len() >= 64 {
            return Err("message subscription limit exceeded".into());
        }
        drop(broker);
        self.live.subscriptions.push((sender, channel));
        Ok(())
    }
    pub(crate) fn poll(&mut self) -> Result<Vec<ProgramMessage>, String> {
        self.call()?;
        if self.server {
            return Err("messages.poll is client-only".into());
        }
        Ok(self.live.inbox.drain(..).map(|m| (*m).clone()).collect())
    }
    pub(crate) fn send(
        &mut self,
        player: String,
        receiver: String,
        channel: String,
        payload: Vec<u8>,
        online: bool,
    ) -> Result<(), String> {
        self.call()?;
        self.sends += 1;
        if !self.server {
            return Err("messages.send is server-only".into());
        }
        if !online {
            return Err("message target is not an authoritative online player".into());
        }
        if self.sends > 8 || payload.len() > PAYLOAD {
            return Err("message send/payload budget exceeded".into());
        }
        if !owner_name(&receiver) || !channel_name(&channel) {
            return Err("invalid message receiver/channel".into());
        }
        let world = self.world.ok_or("message world unavailable")?;
        let broker = self.shared.lock().unwrap();
        let subscription = broker
            .routes
            .iter()
            .filter(|r| r.player == player)
            .map(|r| &r.subscription)
            .chain(
                broker
                    .subscriptions
                    .values()
                    .flatten()
                    .filter(|_| broker.local_player.as_ref() == Some(&player)),
            )
            .find(|s| s.receiver == receiver && s.sender == self.id && s.channel == channel)
            .ok_or("matching receiver subscription unavailable")?;
        let queued: usize = broker
            .queued
            .iter()
            .filter(|(owner, _)| **owner != self.owner)
            .map(|(_, v)| v.iter().filter(|p| *p == &player).count())
            .sum();
        let total: usize = broker
            .queued
            .iter()
            .filter(|(owner, _)| **owner != self.owner)
            .map(|(_, v)| v.len())
            .sum();
        if queued
            + self
                .live
                .outbox
                .iter()
                .filter(|d| d.player == player)
                .count()
            >= QUEUE
            || total + self.live.outbox.len() >= 256
        {
            return Err("message delivery queue full".into());
        }
        let receiver_epoch = subscription.epoch;
        drop(broker);
        self.live.outbox.push_back(Delivery {
            player,
            receiver,
            receiver_epoch,
            sender: self.id.clone(),
            channel,
            payload: payload.into(),
            world,
        });
        Ok(())
    }
    pub(crate) fn receive(
        &mut self,
        receiver_epoch: u64,
        sender: &str,
        channel: &str,
        payload: &[u8],
    ) -> Result<bool, String> {
        if self.server
            || self.world.is_none()
            || receiver_epoch != self.epoch
            || !self
                .live
                .subscriptions
                .iter()
                .any(|(s, c)| s == sender && c == channel)
        {
            return Ok(false);
        }
        if payload.len() > PAYLOAD || !owner_name(sender) || !channel_name(channel) {
            return Err("invalid program message".into());
        }
        let others: usize = self
            .shared
            .lock()
            .unwrap()
            .inbox_counts
            .iter()
            .filter(|(owner, _)| **owner != self.owner)
            .map(|(_, n)| *n)
            .sum();
        if others + self.live.inbox.len() >= QUEUE {
            return Err("program message inbox full".into());
        }
        self.live.inbox.push_back(Arc::new(ProgramMessage {
            sender: sender.into(),
            channel: channel.into(),
            payload: payload.to_vec(),
        }));
        self.publish();
        Ok(true)
    }
    pub(crate) fn drain(&mut self) -> Vec<Delivery> {
        let out = self.live.outbox.drain(..).collect();
        self.publish();
        out
    }
    pub(crate) fn is_receiver(&self, id: &str) -> bool {
        !self.server && self.id == id
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let mut b = self.shared.lock().unwrap();
        b.subscriptions.remove(&self.owner);
        b.queued.remove(&self.owner);
        b.inbox_counts.remove(&self.owner);
    }
}
