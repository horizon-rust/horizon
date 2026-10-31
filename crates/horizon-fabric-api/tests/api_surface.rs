//! Tests for the Rust-side Fabric API surface.

use horizon_fabric_api::events::{BlockBreakEvent, EventBus, TickEvent};
use horizon_fabric_api::networking::{ChannelDirection, ChannelRegistry};
use horizon_fabric_api::registry::{Identifier, Registry};

#[test]
fn identifiers_parse_with_and_without_namespace() {
    let plain = Identifier::parse("stone").unwrap();
    assert_eq!(plain.namespace, "minecraft");
    assert_eq!(plain.path, "stone");

    let namespaced = Identifier::parse("mymod:ruby").unwrap();
    assert_eq!(namespaced.to_string(), "mymod:ruby");
}

#[test]
fn identifiers_reject_bad_names() {
    assert!(Identifier::parse("MyMod:Ruby").is_err());
    assert!(Identifier::parse("mymod:").is_err());
    assert!(Identifier::parse("").is_err());
}

#[test]
fn registry_rejects_duplicates() {
    let mut registry: Registry<u32> = Registry::new("item");
    let id = Identifier::parse("mymod:thing").unwrap();

    registry.register(id.clone(), 1).unwrap();
    assert!(registry.register(id.clone(), 2).is_err());
    assert_eq!(registry.get(&id), Some(&1));
    assert_eq!(registry.len(), 1);
}

#[test]
fn event_bus_dispatches_to_type_and_unsubscribes() {
    let mut bus = EventBus::new();
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = ticks.clone();

    let id = bus.subscribe::<TickEvent, _>(move |event| {
        assert!(event.tick > 0);
        seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });

    assert_eq!(bus.fire(&TickEvent { tick: 1 }), 1);
    assert_eq!(bus.fire(&TickEvent { tick: 2 }), 1);
    // A different event type does not reach the tick listener.
    assert_eq!(
        bus.fire(&BlockBreakEvent {
            dimension: "minecraft:overworld".into(),
            position: [0, 64, 0],
            block: "minecraft:stone".into(),
            cancelled: false,
        }),
        0
    );
    assert_eq!(ticks.load(std::sync::atomic::Ordering::Relaxed), 2);

    assert!(bus.unsubscribe(id));
    assert_eq!(bus.fire(&TickEvent { tick: 3 }), 0);
}

#[test]
fn channel_registry_widens_direction() {
    let mut channels = ChannelRegistry::new();
    let id = Identifier::parse("mymod:sync").unwrap();

    channels.register(id.clone(), ChannelDirection::ClientToServer).unwrap();
    assert!(channels.accepts(&id, ChannelDirection::ClientToServer));
    assert!(!channels.accepts(&id, ChannelDirection::ServerToClient));

    channels.register(id.clone(), ChannelDirection::ServerToClient).unwrap();
    assert!(channels.accepts(&id, ChannelDirection::ServerToClient));
    assert_eq!(channels.direction(&id), Some(ChannelDirection::Both));
}
