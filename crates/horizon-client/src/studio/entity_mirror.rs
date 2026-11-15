//! The entities the server has shown the client, as the last packets said them, so a recording
//! that starts mid-session can open with them. The client turns an `AddEntity` into its own mob,
//! object and player state and keeps neither the entity's UUID, its raw synched data nor its
//! equipment slots, so they cannot be rebuilt from that state; this keeps the packets' content
//! instead. It is cheap: one small record per entity, updated by the packets the session already
//! handles, with no work per tick.
use horizon_protocol::{
    buf::{pack_degrees, unpack_degrees},
    entity_data::Value,
    item::Stack,
    packets::play::{
        self, AddEntity, Clientbound as C, SetEntityData, SetEntityLink, SetEquipment,
        SetPassengers,
    },
};
use std::collections::HashMap;

/// One entity as the server last described it.
#[derive(Clone, Debug)]
struct Entity {
    /// The `AddEntity` that created it, with the position, rotation, head rotation and motion
    /// kept at their latest values.
    add: AddEntity,
    /// Its synched data by entry index.
    data: Vec<(u8, i32, Value)>,
    /// Its worn and held items by `EquipmentSlot` ordinal.
    equipment: Vec<(u8, Option<Stack>)>,
    /// Who holds its lead.
    leash_holder: Option<i32>,
    /// What rides it.
    passengers: Vec<i32>,
}

/// The entities of one session's level.
#[derive(Clone, Debug, Default)]
pub struct EntityMirror {
    entities: HashMap<i32, Entity>,
}

impl EntityMirror {
    /// Takes what `packet` says about the entities in view. The session [`clear`](Self::clear)s
    /// the mirror when the client drops its level: a respawn in the same level keeps its
    /// entities, so it must not.
    pub fn apply(&mut self, packet: &C) {
        match packet {
            C::AddEntity(p) => {
                self.entities.insert(
                    p.id,
                    Entity {
                        add: p.clone(),
                        data: Vec::new(),
                        equipment: Vec::new(),
                        leash_holder: None,
                        passengers: Vec::new(),
                    },
                );
            }
            C::RemoveEntities(p) => {
                for id in &p.ids {
                    self.entities.remove(id);
                }
            }
            C::SetEntityData(SetEntityData { id, values }) => {
                if let Some(entity) = self.entities.get_mut(id) {
                    for value in values {
                        match entity.data.iter_mut().find(|(index, ..)| *index == value.0) {
                            Some(held) => *held = value.clone(),
                            None => entity.data.push(value.clone()),
                        }
                    }
                }
            }
            C::SetEquipment(SetEquipment { id, slots }) => {
                if let Some(entity) = self.entities.get_mut(id) {
                    for slot in slots {
                        match entity.equipment.iter_mut().find(|(s, _)| *s == slot.0) {
                            Some(held) => *held = slot.clone(),
                            None => entity.equipment.push(slot.clone()),
                        }
                    }
                }
            }
            C::SetEntityMotion(p) => {
                if let Some(entity) = self.entities.get_mut(&p.id) {
                    entity.add.velocity = p.velocity;
                }
            }
            C::RotateHead(p) => {
                if let Some(entity) = self.entities.get_mut(&p.id) {
                    entity.add.head_yaw = p.head_yaw;
                }
            }
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                if let Some(entity) = self.entities.get_mut(&m.id) {
                    if let Some(delta) = &m.delta {
                        entity.add.position = delta.end(entity.add.position);
                    }
                    if let Some((yaw, pitch)) = m.rotation {
                        entity.add.yaw = yaw;
                        entity.add.pitch = pitch;
                    }
                }
            }
            C::EntityPositionSync(p) => {
                if let Some(entity) = self.entities.get_mut(&p.id) {
                    entity.add.position = p.path.end();
                    entity.add.yaw = pack_degrees(p.yaw);
                    entity.add.pitch = pack_degrees(p.pitch);
                }
            }
            C::TeleportEntity(p) => {
                if let Some(entity) = self.entities.get_mut(&p.id) {
                    let at = entity.add.position;
                    let axis = |bit: i32, i: usize| {
                        if p.relatives & bit != 0 {
                            at[i] + p.position[i]
                        } else {
                            p.position[i]
                        }
                    };
                    entity.add.position = [
                        axis(play::relative::X, 0),
                        axis(play::relative::Y, 1),
                        axis(play::relative::Z, 2),
                    ];
                    let angle = |bit: i32, now: i8, new: f32| {
                        if p.relatives & bit != 0 {
                            pack_degrees(unpack_degrees(now) + new)
                        } else {
                            pack_degrees(new)
                        }
                    };
                    let motion = |bit: i32, now: f64, new: f64| {
                        if p.relatives & bit != 0 {
                            now + new
                        } else {
                            new
                        }
                    };
                    let now = entity.add.velocity;
                    entity.add.velocity = [
                        motion(play::relative::DELTA_X, now[0], p.delta[0]),
                        motion(play::relative::DELTA_Y, now[1], p.delta[1]),
                        motion(play::relative::DELTA_Z, now[2], p.delta[2]),
                    ];
                    entity.add.yaw = angle(play::relative::Y_ROT, entity.add.yaw, p.yaw);
                    entity.add.pitch = angle(play::relative::X_ROT, entity.add.pitch, p.pitch);
                }
            }
            C::SetPassengers(SetPassengers {
                vehicle,
                passengers,
            }) => {
                if let Some(entity) = self.entities.get_mut(vehicle) {
                    entity.passengers.clone_from(passengers);
                }
            }
            C::SetEntityLink(SetEntityLink { source, dest }) => {
                if let Some(entity) = self.entities.get_mut(source) {
                    entity.leash_holder = (*dest != 0).then_some(*dest);
                }
            }
            _ => {}
        }
    }

    /// Forgets every entity: the client's level is gone and the server shows its entities again.
    pub fn clear(&mut self) {
        self.entities.clear();
    }

    /// How many entities it holds.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The packets that show the entities as they stand: every `AddEntity` first (by id), then
    /// their data, equipment, leads and riders, which need the entities to exist.
    pub fn packets(&self) -> Vec<C> {
        let mut ids: Vec<i32> = self.entities.keys().copied().collect();
        ids.sort_unstable();
        let mut out: Vec<C> = ids
            .iter()
            .map(|id| C::AddEntity(self.entities[id].add.clone()))
            .collect();
        for id in &ids {
            let entity = &self.entities[id];
            if !entity.data.is_empty() {
                out.push(C::SetEntityData(SetEntityData {
                    id: *id,
                    values: entity.data.clone(),
                }));
            }
            if !entity.equipment.is_empty() {
                out.push(C::SetEquipment(SetEquipment {
                    id: *id,
                    slots: entity.equipment.clone(),
                }));
            }
        }
        for id in &ids {
            let entity = &self.entities[id];
            if let Some(dest) = entity.leash_holder {
                out.push(C::SetEntityLink(SetEntityLink { source: *id, dest }));
            }
            if !entity.passengers.is_empty() {
                out.push(C::SetPassengers(SetPassengers {
                    vehicle: *id,
                    passengers: entity.passengers.clone(),
                }));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::packets::play::{
        EntityPositionSync, MoveEntity, PositionPath, RemoveEntities, RotateHead, SetEntityMotion,
        TeleportEntity, VecDelta,
    };

    fn add(id: i32, position: [f64; 3]) -> C {
        C::AddEntity(AddEntity {
            id,
            uuid: 0x1000 + id as u128,
            kind: 7,
            position,
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        })
    }

    fn added(packets: &[C], id: i32) -> AddEntity {
        packets
            .iter()
            .find_map(|p| match p {
                C::AddEntity(a) if a.id == id => Some(a.clone()),
                _ => None,
            })
            .expect("entity in the snapshot")
    }

    #[test]
    fn data_and_equipment_merge_by_index_and_slot() {
        let mut mirror = EntityMirror::default();
        mirror.apply(&add(5, [0.0; 3]));
        mirror.apply(&C::SetEntityData(SetEntityData {
            id: 5,
            values: vec![(0, 0, Value::Byte(1)), (8, 3, Value::Float(20.0))],
        }));
        mirror.apply(&C::SetEntityData(SetEntityData {
            id: 5,
            values: vec![(0, 0, Value::Byte(0x20)), (9, 1, Value::Int(4))],
        }));
        mirror.apply(&C::SetEquipment(SetEquipment {
            id: 5,
            slots: vec![(0, Some(Stack::new(700, 1))), (5, Some(Stack::new(701, 1)))],
        }));
        mirror.apply(&C::SetEquipment(SetEquipment {
            id: 5,
            slots: vec![(0, None), (4, Some(Stack::new(702, 1)))],
        }));
        let packets = mirror.packets();
        let data = packets.iter().find_map(|p| match p {
            C::SetEntityData(d) => Some(d.clone()),
            _ => None,
        });
        let mut values = data.expect("data").values;
        values.sort_by_key(|v| v.0);
        assert_eq!(
            values,
            [
                (0, 0, Value::Byte(0x20)),
                (8, 3, Value::Float(20.0)),
                (9, 1, Value::Int(4)),
            ]
        );
        let equipment = packets.iter().find_map(|p| match p {
            C::SetEquipment(e) => Some(e.clone()),
            _ => None,
        });
        let mut slots = equipment.expect("equipment").slots;
        slots.sort_by_key(|s| s.0);
        assert_eq!(
            slots,
            [
                (0, None),
                (4, Some(Stack::new(702, 1))),
                (5, Some(Stack::new(701, 1))),
            ]
        );
    }

    #[test]
    fn an_entity_is_where_the_last_packets_left_it() {
        let mut mirror = EntityMirror::default();
        mirror.apply(&add(1, [10.0, 64.0, 10.0]));
        mirror.apply(&add(2, [0.0, 70.0, 0.0]));
        mirror.apply(&add(3, [5.0, 5.0, 5.0]));
        // 4096 units are one block.
        mirror.apply(&C::MoveEntityPosRot(play::MoveEntityPosRot(MoveEntity {
            id: 1,
            delta: Some(VecDelta::Linear([4096, 0, -8192])),
            rotation: Some((64, 16)),
            on_ground: true,
        })));
        mirror.apply(&C::RotateHead(RotateHead {
            id: 1,
            head_yaw: 32,
        }));
        mirror.apply(&C::SetEntityMotion(SetEntityMotion {
            id: 1,
            velocity: [0.5, 0.0, 0.0],
        }));
        mirror.apply(&C::EntityPositionSync(EntityPositionSync {
            id: 2,
            path: PositionPath::Linear([1.0, 2.0, 3.0]),
            yaw: 90.0,
            pitch: 45.0,
            on_ground: false,
        }));
        // Relative to where it stood: x and z, with an absolute y; the yaw turns by 90 more.
        mirror.apply(&C::TeleportEntity(TeleportEntity {
            id: 3,
            position: [1.0, 100.0, -2.0],
            delta: [0.0, 0.25, 0.0],
            yaw: 90.0,
            pitch: 10.0,
            relatives: play::relative::X | play::relative::Z | play::relative::Y_ROT,
            on_ground: false,
        }));
        mirror.apply(&C::TeleportEntity(TeleportEntity {
            id: 3,
            position: [0.0, 0.0, 0.0],
            delta: [0.0, 0.5, 0.0],
            yaw: 90.0,
            pitch: 10.0,
            relatives: play::relative::X
                | play::relative::Y
                | play::relative::Z
                | play::relative::DELTA_Y
                | play::relative::Y_ROT,
            on_ground: false,
        }));
        let packets = mirror.packets();
        let one = added(&packets, 1);
        assert_eq!(one.position, [11.0, 64.0, 8.0]);
        assert_eq!((one.yaw, one.pitch, one.head_yaw), (64, 16, 32));
        assert_eq!(one.velocity, [0.5, 0.0, 0.0]);
        let two = added(&packets, 2);
        assert_eq!(two.position, [1.0, 2.0, 3.0]);
        assert_eq!(
            (two.yaw, two.pitch),
            (pack_degrees(90.0), pack_degrees(45.0))
        );
        let three = added(&packets, 3);
        assert_eq!(three.position, [6.0, 100.0, 3.0]);
        assert_eq!(three.yaw, pack_degrees(180.0));
        assert_eq!(three.pitch, pack_degrees(10.0));
        // The first teleport set the motion; the second added to it.
        assert_eq!(three.velocity, [0.0, 0.75, 0.0]);
    }

    #[test]
    fn removed_entities_and_a_cleared_level_are_forgotten() {
        let mut mirror = EntityMirror::default();
        for id in 1..=3 {
            mirror.apply(&add(id, [0.0; 3]));
        }
        mirror.apply(&C::SetEntityData(SetEntityData {
            id: 2,
            values: vec![(0, 0, Value::Byte(1))],
        }));
        mirror.apply(&C::RemoveEntities(RemoveEntities { ids: vec![2, 9] }));
        let packets = mirror.packets();
        let mut ids: Vec<i32> = packets
            .iter()
            .filter_map(|p| match p {
                C::AddEntity(a) => Some(a.id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 3]);
        // Nothing of the removed entity's data survives.
        assert!(!packets.iter().any(|p| matches!(p, C::SetEntityData(_))));
        // An update for an entity the server never added is dropped, not kept for later.
        mirror.apply(&C::SetEntityData(SetEntityData {
            id: 2,
            values: vec![(0, 0, Value::Byte(1))],
        }));
        mirror.apply(&add(2, [0.0; 3]));
        assert!(
            !mirror
                .packets()
                .iter()
                .any(|p| matches!(p, C::SetEntityData(_)))
        );
        mirror.clear();
        assert!(mirror.is_empty());
    }

    #[test]
    fn leads_and_riders_follow_the_server_and_come_after_the_entities() {
        let mut mirror = EntityMirror::default();
        mirror.apply(&add(1, [0.0; 3]));
        mirror.apply(&add(2, [0.0; 3]));
        mirror.apply(&C::SetEntityLink(SetEntityLink { source: 1, dest: 2 }));
        mirror.apply(&C::SetPassengers(SetPassengers {
            vehicle: 2,
            passengers: vec![1, 77],
        }));
        let packets = mirror.packets();
        let last_add = packets
            .iter()
            .rposition(|p| matches!(p, C::AddEntity(_)))
            .unwrap();
        let link = packets
            .iter()
            .position(|p| matches!(p, C::SetEntityLink(_)))
            .unwrap();
        let riders = packets
            .iter()
            .position(|p| matches!(p, C::SetPassengers(_)))
            .unwrap();
        assert!(link > last_add && riders > last_add);
        assert!(packets.contains(&C::SetEntityLink(SetEntityLink { source: 1, dest: 2 })));
        assert!(packets.contains(&C::SetPassengers(SetPassengers {
            vehicle: 2,
            passengers: vec![1, 77]
        })));
        // The lead is cut and the rider gets off.
        mirror.apply(&C::SetEntityLink(SetEntityLink { source: 1, dest: 0 }));
        mirror.apply(&C::SetPassengers(SetPassengers {
            vehicle: 2,
            passengers: vec![],
        }));
        let packets = mirror.packets();
        assert!(
            !packets
                .iter()
                .any(|p| matches!(p, C::SetEntityLink(_) | C::SetPassengers(_)))
        );
    }
}
