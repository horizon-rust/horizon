//! A small typed event bus, mirroring Fabric API's event callbacks.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Identifier for a subscription, used to unsubscribe later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubscriptionId(u64);

/// A value that can be dispatched on the [`EventBus`].
pub trait Event: Any + Send + Sync + 'static {}

impl<T: Any + Send + Sync + 'static> Event for T {}

type Listener = Box<dyn Fn(&dyn Any) + Send + Sync + 'static>;

/// A typed event bus.
#[derive(Default)]
pub struct EventBus {
    next_id: AtomicU64,
    listeners: HashMap<TypeId, Vec<(SubscriptionId, Listener)>>,
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("event_types", &self.listeners.len())
            .finish_non_exhaustive()
    }
}

impl EventBus {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to events of type `T`.
    pub fn subscribe<T, F>(&mut self, listener: F) -> SubscriptionId
    where
        T: Event,
        F: Fn(&T) + Send + Sync + 'static,
    {
        let id = SubscriptionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let boxed: Listener = Box::new(move |event| {
            if let Some(event) = event.downcast_ref::<T>() {
                listener(event);
            }
        });
        self.listeners
            .entry(TypeId::of::<T>())
            .or_default()
            .push((id, boxed));
        id
    }

    /// Remove a subscription.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        for listeners in self.listeners.values_mut() {
            if let Some(index) = listeners.iter().position(|(listener_id, _)| *listener_id == id) {
                listeners.remove(index);
                return true;
            }
        }
        false
    }

    /// Dispatch an event to every listener registered for its type.
    ///
    /// Returns the number of listeners that ran.
    pub fn fire<T: Event>(&self, event: &T) -> usize {
        let Some(listeners) = self.listeners.get(&TypeId::of::<T>()) else {
            return 0;
        };
        for (_, listener) in listeners {
            listener(event);
        }
        listeners.len()
    }

    /// Number of listeners registered for `T`.
    pub fn listener_count<T: Event>(&self) -> usize {
        self.listeners
            .get(&TypeId::of::<T>())
            .map_or(0, Vec::len)
    }

    /// Total number of listeners across all event types.
    pub fn total_listeners(&self) -> usize {
        self.listeners.values().map(Vec::len).sum()
    }
}

/// Fired once per game tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickEvent {
    /// The tick counter.
    pub tick: u64,
}

/// Fired when a player breaks a block.
#[derive(Debug, Clone)]
pub struct BlockBreakEvent {
    /// Dimension identifier.
    pub dimension: String,
    /// Block position.
    pub position: [i32; 3],
    /// Block identifier.
    pub block: String,
    /// Whether another listener cancelled the break.
    pub cancelled: bool,
}

/// Fired when a player places a block.
#[derive(Debug, Clone)]
pub struct BlockPlaceEvent {
    /// Dimension identifier.
    pub dimension: String,
    /// Block position.
    pub position: [i32; 3],
    /// Block identifier.
    pub block: String,
    /// Whether another listener cancelled the placement.
    pub cancelled: bool,
}

/// Fired when an entity takes damage.
#[derive(Debug, Clone)]
pub struct EntityDamageEvent {
    /// Entity id.
    pub entity_id: u32,
    /// Damage amount.
    pub amount: f32,
    /// Whether another listener cancelled the damage.
    pub cancelled: bool,
}

/// Fired when a player joins.
#[derive(Debug, Clone)]
pub struct PlayerJoinEvent {
    /// Player name.
    pub name: String,
}

/// Fired when a player leaves.
#[derive(Debug, Clone)]
pub struct PlayerLeaveEvent {
    /// Player name.
    pub name: String,
}

/// Fired when a chat message is sent.
#[derive(Debug, Clone)]
pub struct ChatEvent {
    /// Sender name.
    pub sender: String,
    /// Message body.
    pub message: String,
    /// Whether another listener cancelled the message.
    pub cancelled: bool,
}
