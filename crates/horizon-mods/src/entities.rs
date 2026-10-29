//! Bounded entity leases and transactions. The game owns one `Level` per level.
use crate::{
    EntityContext, EntityEvent, EntityMovementDelta, EntityMovementResult, EntityReason,
    EntityState, EntityTransform, EntityType,
};
use std::{
    cell::Cell,
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

pub const TYPES_PER_OWNER: usize = 8;
pub const LIVE_PER_OWNER: usize = 64;
pub const LIVE_PER_WORLD: usize = 256;
pub const ATTEMPTS_PER_CALLBACK: usize = 64;
pub const PUBLIC_BYTES: usize = 1024;
const EVENTS: usize = 256;
static IDENTITY: AtomicU64 = AtomicU64::new(1);

fn advance(counter: &AtomicU64) -> Result<u64, String> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map_err(|_| "entity identity exhausted".into())
}

/// A fresh host world session. Never derived from a guest or a save name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldId(u64);
impl WorldId {
    pub fn new() -> Result<Self, String> {
        advance(&IDENTITY).map(Self)
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

struct Owner {
    tag: u64,
    epoch: u64,
    serial: AtomicU64,
    events: Mutex<VecDeque<EntityEvent>>,
    levels: Mutex<Vec<Weak<Mutex<Bucket>>>>,
}
impl Owner {
    fn handle(&self, kind: &str, world: u64, slot: usize) -> Result<String, String> {
        let generation = advance(&self.serial)?;
        Ok(format!(
            "{kind}:{:x}:{:x}:{world:x}:{slot:x}:{generation:x}",
            self.tag, self.epoch
        ))
    }
    fn event(&self, event: EntityEvent) {
        let mut events = self.events.lock().unwrap();
        if events.len() == EVENTS {
            events.pop_front();
        }
        events.push_back(event);
    }
    fn track(&self, level: &Arc<Mutex<Bucket>>) {
        let mut levels = self.levels.lock().unwrap();
        levels.retain(|l| l.strong_count() != 0);
        let weak = Arc::downgrade(level);
        if !levels.iter().any(|l| l.ptr_eq(&weak)) {
            levels.push(weak);
        }
    }
    fn revoke(&self) {
        for level in self
            .levels
            .lock()
            .unwrap()
            .drain(..)
            .filter_map(|l| l.upgrade())
        {
            level
                .lock()
                .unwrap()
                .records
                .retain(|r| r.owner_tag != self.tag);
        }
        self.events.lock().unwrap().clear();
    }
}

/// An applied authoritative record. No vanilla behavior or persistence is attached.
#[derive(Clone, Debug)]
pub struct Record {
    pub handle: String,
    pub engine_id: i32,
    pub kind: EntityType,
    pub state: EntityState,
    pub revision: u64,
    owner_tag: u64,
    world: WorldId,
    owner: Weak<Owner>,
}
impl Record {
    fn notify(&self, reason: EntityReason) {
        if let Some(owner) = self.owner.upgrade() {
            owner.event(EntityEvent {
                entity: self.handle.clone(),
                reason,
            });
        }
    }
}

#[derive(Default)]
struct Bucket {
    records: Vec<Record>,
}

/// Lazy per-level storage. Dropping it unloads its records without a guest callback.
#[derive(Default)]
pub struct Level {
    table: Option<Arc<Mutex<Bucket>>>,
}
impl Level {
    pub fn records(&self) -> Vec<Record> {
        self.table
            .as_ref()
            .map_or_else(Vec::new, |t| t.lock().unwrap().records.clone())
    }
    pub fn allocated(&self) -> bool {
        self.table.is_some()
    }
    pub fn prune(&mut self) {
        if self
            .table
            .as_ref()
            .is_some_and(|t| t.lock().unwrap().records.is_empty())
        {
            self.table = None;
        }
    }
    pub fn clear(&mut self, reason: EntityReason) {
        if let Some(table) = self.table.take() {
            for r in table.lock().unwrap().records.drain(..) {
                r.notify(reason);
            }
        }
    }
    pub fn unload_chunks(&mut self, chunks: &HashSet<(i32, i32)>) {
        if let Some(table) = &self.table {
            table.lock().unwrap().records.retain(|r| {
                let p = &r.state.transform;
                let gone = chunks.contains(&(chunk(p.x), chunk(p.z)));
                if gone {
                    r.notify(EntityReason::ChunkUnloaded);
                }
                !gone
            });
        }
        self.prune();
    }
    /// Only resident chunks and this dimension's real vertical bounds are lent.
    pub fn loaded(
        &mut self,
        dimension: &str,
        min_y: i32,
        max_y: i32,
        chunks: impl IntoIterator<Item = (i32, i32)>,
    ) -> LoadedLevel {
        let table = self
            .table
            .get_or_insert_with(|| Arc::new(Mutex::new(Bucket::default())))
            .clone();
        LoadedLevel {
            dimension: dimension.into(),
            min_y,
            max_y,
            chunks: chunks.into_iter().collect(),
            table,
        }
    }
}
impl Drop for Level {
    fn drop(&mut self) {
        self.clear(EntityReason::DimensionUnloaded);
    }
}

pub struct LoadedLevel {
    dimension: String,
    min_y: i32,
    max_y: i32,
    chunks: HashSet<(i32, i32)>,
    table: Arc<Mutex<Bucket>>,
}

/// The host lends all loaded levels and its shared entity ID cursor for callbacks.
pub struct World {
    id: WorldId,
    last_id: u32,
    levels: Vec<LoadedLevel>,
}
impl World {
    pub fn new(id: WorldId, last_id: u32, levels: Vec<LoadedLevel>) -> Self {
        for level in &levels {
            level.table.lock().unwrap().records.retain(|r| {
                if r.world != id {
                    r.notify(EntityReason::WorldUnloaded);
                }
                r.world == id
            });
        }
        Self {
            id,
            last_id,
            levels,
        }
    }
    pub fn last_id(&self) -> u32 {
        self.last_id
    }
    fn context(&self) -> EntityContext {
        let mut dimensions: Vec<_> = self.levels.iter().map(|l| l.dimension.clone()).collect();
        dimensions.sort();
        EntityContext {
            world: self.id.get(),
            dimensions,
        }
    }
    fn records(&self) -> Vec<Record> {
        self.levels
            .iter()
            .flat_map(|l| l.table.lock().unwrap().records.clone())
            .collect()
    }
    fn placement(&self, state: &EntityState) -> Result<(), String> {
        let p = &state.transform;
        if ![p.x, p.y, p.z].iter().all(|v| v.is_finite())
            || !p.yaw.is_finite()
            || !p.pitch.is_finite()
        {
            return Err("entity transform must be finite".into());
        }
        if p.x.abs() > 29_999_984.0
            || p.z.abs() > 29_999_984.0
            || p.yaw.abs() > 360.0
            || p.pitch.abs() > 90.0
        {
            return Err("entity transform out of range".into());
        }
        if state.public_state.len() > PUBLIC_BYTES {
            return Err("entity public state exceeds 1024 bytes".into());
        }
        let level = self
            .levels
            .iter()
            .find(|l| l.dimension == state.dimension)
            .ok_or("entity dimension is not loaded")?;
        if p.y < f64::from(level.min_y) || p.y >= f64::from(level.max_y) {
            return Err("entity outside dimension height".into());
        }
        if !level.chunks.contains(&(chunk(p.x), chunk(p.z))) {
            return Err("entity chunk is not loaded".into());
        }
        Ok(())
    }
    fn commit(&mut self, owner: &Arc<Owner>, tx: &Transaction) -> Result<(), String> {
        if tx.world != Some(self.id) {
            return Err("entity world session changed".into());
        }
        let other = self
            .records()
            .iter()
            .filter(|r| r.owner_tag != owner.tag)
            .count();
        if tx.records.len() > LIVE_PER_OWNER || other + tx.records.len() > LIVE_PER_WORLD {
            return Err("entity live quota exhausted".into());
        }
        for r in &tx.records {
            self.placement(&r.state)?;
        }
        for level in &self.levels {
            let mut table = level.table.lock().unwrap();
            table.records.retain(|r| r.owner_tag != owner.tag);
            table.records.extend(
                tx.records
                    .iter()
                    .filter(|r| r.state.dimension == level.dimension)
                    .cloned(),
            );
            drop(table);
            if tx
                .records
                .iter()
                .any(|r| r.state.dimension == level.dimension)
            {
                owner.track(&level.table);
            }
        }
        self.last_id = tx.last_id;
        Ok(())
    }
}
fn chunk(v: f64) -> i32 {
    (v.floor() as i32).div_euclid(16)
}

pub(crate) type Slot = Arc<Mutex<Option<World>>>;
#[derive(Clone)]
struct Registered {
    handle: String,
    definition: EntityType,
}
struct Transaction {
    world: Option<WorldId>,
    last_id: u32,
    other_count: usize,
    types: Vec<Registered>,
    records: Vec<Record>,
    events: Vec<EntityEvent>,
    polled: usize,
}

/// One running owner's registration and callback staging area.
pub(crate) struct Session {
    owner: Arc<Owner>,
    namespaces: Vec<String>,
    slot: Slot,
    types: Vec<Registered>,
    attempts: usize,
    collision_attempts: Cell<usize>,
    transaction: Option<Transaction>,
}
impl Session {
    pub(crate) fn new(namespaces: Vec<String>, slot: Slot) -> Result<Self, String> {
        Ok(Self {
            owner: Arc::new(Owner {
                tag: advance(&IDENTITY)?,
                epoch: advance(&IDENTITY)?,
                serial: AtomicU64::new(1),
                events: Mutex::new(VecDeque::new()),
                levels: Mutex::new(Vec::new()),
            }),
            namespaces,
            slot,
            types: Vec::new(),
            attempts: 0,
            collision_attempts: Cell::new(0),
            transaction: None,
        })
    }
    pub(crate) fn owns_handle(&self, handle: &str) -> bool {
        self.slot.lock().unwrap().as_ref().is_some_and(|world| {
            world.records().iter().any(|record| {
                record.owner_tag == self.owner.tag
                    && record.world == world.id
                    && record.handle == handle
            })
        })
    }
    pub(crate) fn begin(&mut self) {
        self.attempts = 0;
        self.collision_attempts.set(0);
        self.transaction = None;
    }
    fn attempt<T>(
        &mut self,
        f: impl FnOnce(&Self, &mut Transaction, Option<&World>) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.attempts == ATTEMPTS_PER_CALLBACK {
            return Err("entity callback attempt budget exhausted".into());
        }
        self.attempts += 1;
        let slot = self.slot.lock().unwrap();
        let world = slot.as_ref();
        let mut tx = self.transaction.take().unwrap_or_else(|| {
            let all = world.map_or_else(Vec::new, World::records);
            let records: Vec<_> = all
                .iter()
                .filter(|r| r.owner_tag == self.owner.tag)
                .cloned()
                .collect();
            Transaction {
                world: world.map(|w| w.id),
                last_id: world.map_or(0, |w| w.last_id),
                other_count: all.len() - records.len(),
                types: self.types.clone(),
                records,
                events: Vec::new(),
                polled: 0,
            }
        });
        let result = f(self, &mut tx, world);
        self.transaction = Some(tx);
        result
    }
    pub(crate) fn finish(&mut self, success: bool) -> Result<(), String> {
        let Some(tx) = self.transaction.take() else {
            return Ok(());
        };
        if !success {
            return Ok(());
        }
        if tx.world.is_some() {
            let mut slot = self.slot.lock().unwrap();
            slot.as_mut()
                .ok_or("entity world unavailable")?
                .commit(&self.owner, &tx)?;
        }
        self.types = tx.types;
        let mut queue = self.owner.events.lock().unwrap();
        for _ in 0..tx.polled {
            queue.pop_front();
        }
        drop(queue);
        for event in tx.events {
            self.owner.event(event);
        }
        Ok(())
    }
    pub(crate) fn register(&mut self, definitions: Vec<EntityType>) -> Result<Vec<String>, String> {
        self.attempt(|s, tx, _| {
            if definitions.is_empty() || tx.types.len() + definitions.len() > TYPES_PER_OWNER {
                return Err("entity type quota is eight per owner".into());
            }
            let mut added = Vec::<Registered>::new();
            for d in definitions {
                if !key(&d.key)
                    || !s
                        .namespaces
                        .iter()
                        .any(|ns| d.key.starts_with(&format!("{ns}:")))
                {
                    return Err("entity type key is not in an owned namespace".into());
                }
                if tx
                    .types
                    .iter()
                    .chain(&added)
                    .any(|t| t.definition.key == d.key)
                {
                    return Err("entity type key is already registered".into());
                }
                if !d.width.is_finite()
                    || !d.height.is_finite()
                    || !d.tracking_range.is_finite()
                    || d.width <= 0.0
                    || d.width > 64.0
                    || d.height <= 0.0
                    || d.height > 64.0
                    || d.tracking_range <= 0.0
                    || d.tracking_range > 256.0
                    || !key(&d.renderer)
                    || !(1..=65535).contains(&d.public_version)
                {
                    return Err("invalid entity dimensions, tracking or renderer contract".into());
                }
                added.push(Registered {
                    handle: s.owner.handle("t", 0, tx.types.len() + added.len())?,
                    definition: d,
                });
            }
            let handles = added.iter().map(|t| t.handle.clone()).collect();
            tx.types.extend(added);
            Ok(handles)
        })
    }
    pub(crate) fn context(&mut self) -> Result<Option<EntityContext>, String> {
        self.attempt(|_, _, w| Ok(w.map(World::context)))
    }
    pub(crate) fn spawn(&mut self, kind: String, state: EntityState) -> Result<String, String> {
        self.attempt(|s, tx, w| {
            let w = w.ok_or("entity world unavailable")?;
            let kind = tx
                .types
                .iter()
                .find(|t| t.handle == kind)
                .ok_or("unknown, foreign or stale entity type handle")?;
            w.placement(&state)?;
            if tx.records.len() == LIVE_PER_OWNER
                || tx.other_count + tx.records.len() == LIVE_PER_WORLD
            {
                return Err("entity live quota exhausted".into());
            }
            let id = tx
                .last_id
                .checked_add(1)
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("host entity ID exhausted")?;
            let slot = (0..LIVE_PER_OWNER)
                .find(|i| {
                    !tx.records
                        .iter()
                        .any(|r| r.handle.split(':').nth(4) == Some(format!("{i:x}").as_str()))
                })
                .ok_or("entity slots exhausted")?;
            let handle = s.owner.handle("e", w.id.get(), slot)?;
            tx.records.push(Record {
                handle: handle.clone(),
                engine_id: id,
                kind: kind.definition.clone(),
                state,
                revision: 1,
                owner_tag: s.owner.tag,
                world: w.id,
                owner: Arc::downgrade(&s.owner),
            });
            tx.last_id = id as u32;
            tx.events.push(EntityEvent {
                entity: handle.clone(),
                reason: EntityReason::Spawned,
            });
            Ok(handle)
        })
    }
    pub(crate) fn set_state(&mut self, entity: String, state: EntityState) -> Result<(), String> {
        self.attempt(|_, tx, w| {
            let w = w.ok_or("entity world unavailable")?;
            let i = tx
                .records
                .iter()
                .position(|r| r.handle == entity)
                .ok_or("unknown, foreign or stale entity handle")?;
            w.placement(&state)?;
            let revision = tx.records[i]
                .revision
                .checked_add(1)
                .ok_or("entity revision exhausted")?;
            tx.records[i].state = state;
            tx.records[i].revision = revision;
            tx.events.push(EntityEvent {
                entity,
                reason: EntityReason::Updated,
            });
            Ok(())
        })
    }
    pub(crate) fn move_colliding(
        &mut self,
        entity: String,
        delta: EntityMovementDelta,
        resolve: impl FnOnce(
            &EntityState,
            &EntityType,
            &EntityMovementDelta,
        ) -> Result<EntityMovementResult, String>,
    ) -> Result<EntityMovementResult, String> {
        self.attempt(|s, tx, world| {
            let attempts = s.collision_attempts.get();
            if attempts >= 8 {
                return Err("entity collision attempt budget exhausted".into());
            }
            s.collision_attempts.set(attempts + 1);
            if [delta.x, delta.y, delta.z]
                .into_iter()
                .any(|v| !v.is_finite() || v.abs() > 4.0)
            {
                return Err(
                    "entity collision delta must be finite and at most four blocks per axis".into(),
                );
            }
            let world = world.ok_or("entity world unavailable")?;
            let record = tx
                .records
                .iter_mut()
                .find(|r| r.handle == entity && r.world == world.id)
                .ok_or("unknown, foreign or stale entity handle")?;
            world.placement(&record.state)?;
            let moved = resolve(&record.state, &record.kind, &delta)?;
            let mut state = record.state.clone();
            state.transform.x = moved.x;
            state.transform.y = moved.y;
            state.transform.z = moved.z;
            world.placement(&state)?;
            let revision = record
                .revision
                .checked_add(1)
                .ok_or("entity revision exhausted")?;
            record.state = state;
            record.revision = revision;
            tx.events.push(EntityEvent {
                entity,
                reason: EntityReason::Updated,
            });
            Ok(moved)
        })
    }
    pub(crate) fn probe_colliding(
        &mut self,
        entity: String,
        dimension: String,
        start: EntityTransform,
        delta: EntityMovementDelta,
        resolve: impl FnOnce(
            &EntityState,
            &EntityType,
            &EntityMovementDelta,
        ) -> Result<EntityMovementResult, String>,
    ) -> Result<EntityMovementResult, String> {
        if self.attempts >= ATTEMPTS_PER_CALLBACK {
            return Err("entity callback attempt budget exhausted".into());
        }
        self.attempts += 1;
        let attempts = self.collision_attempts.get();
        if attempts >= 8 {
            return Err("entity collision attempt budget exhausted".into());
        }
        self.collision_attempts.set(attempts + 1);
        if [delta.x, delta.y, delta.z].into_iter()
            .any(|v| !v.is_finite() || v.abs() > 4.0)
        {
            return Err("entity collision delta must be finite and at most four blocks per axis".into());
        }
        if dimension.len() > 128
            || ![start.x, start.y, start.z].into_iter().all(f64::is_finite)
            || !start.yaw.is_finite()
            || !start.pitch.is_finite()
        {
            return Err("entity collision expected baseline is invalid".into());
        }
        let slot = self.slot.lock().unwrap();
        let world = slot.as_ref().ok_or("entity world unavailable")?;
        let owned = |r: &Record| {
            r.handle == entity && r.world == world.id && r.owner_tag == self.owner.tag
        };
        let record = match self.transaction.as_ref() {
            Some(tx) if tx.world != Some(world.id) => {
                return Err("entity transaction belongs to another world".into());
            }
            Some(tx) => tx.records.iter().find(|r| owned(r)).cloned(),
            None => world.records().into_iter().find(owned),
        }.ok_or("unknown, foreign or stale entity handle")?;
        let current = &record.state.transform;
        if record.state.dimension != dimension
            || current.x != start.x || current.y != start.y || current.z != start.z
            || current.yaw != start.yaw || current.pitch != start.pitch
        {
            return Err("entity collision baseline differs from the expected durable transform".into());
        }
        world.placement(&record.state)?;
        let moved = resolve(&record.state, &record.kind, &delta)?;
        let mut state = record.state.clone();
        state.transform.x = moved.x;
        state.transform.y = moved.y;
        state.transform.z = moved.z;
        world.placement(&state)?;
        Ok(moved)
    }
    pub(crate) fn despawn(&mut self, entity: String) -> Result<(), String> {
        self.attempt(|_, tx, w| {
            w.ok_or("entity world unavailable")?;
            let i = tx
                .records
                .iter()
                .position(|r| r.handle == entity)
                .ok_or("unknown, foreign or stale entity handle")?;
            tx.records.remove(i);
            tx.events.push(EntityEvent {
                entity,
                reason: EntityReason::Despawned,
            });
            Ok(())
        })
    }
    pub(crate) fn poll(&mut self) -> Result<Vec<EntityEvent>, String> {
        self.attempt(|s, tx, _| {
            let events: Vec<_> = s
                .owner
                .events
                .lock()
                .unwrap()
                .iter()
                .skip(tx.polled)
                .cloned()
                .collect();
            tx.polled += events.len();
            Ok(events)
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.owner.revoke();
    }
}
fn key(value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    let Some((namespace, path)) = value.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && namespace
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        && !path.is_empty()
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
}
