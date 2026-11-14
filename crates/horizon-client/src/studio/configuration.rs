//! The session's configuration phase in a recording: the registries (biomes, dimension types,
//! chat and damage types...) and tags the server synchronized before the first play packet.
//!
//! Play packets index these by number, so a replay opened without the server it was recorded
//! on has to bring them along. They are stored as the configuration packets the server sent
//! (`RegistryData`, `UpdateTags`), the way ReplayMod keeps that phase, in the archive's
//! `configuration.tmcpr` ([`super::mcpr`]).
use super::mcpr::TimestampedPacket;
use horizon_protocol::{
    buf::Reader,
    packets::configuration::{
        Clientbound as C, CustomPayload, RegistryData, RegistryEntry, RegistryTags, TagEntry,
        UpdateTags,
    },
    registry::Dynamic,
};

/// Present (with an empty body) when the session confirmed Horizon's native component
/// registry (`Dynamic::custom_components`), which no vanilla packet carries.
const CUSTOM_COMPONENTS_CHANNEL: &str = "horizon:custom_components";

/// The configuration packets that rebuild `registries`, in the order a server sends them:
/// every registry (sorted by name, entries in id order), then the tags.
pub fn packets(registries: &Dynamic) -> Vec<TimestampedPacket> {
    // A session that never synchronized anything (a replay playing an older file) has nothing to
    // keep; an empty list would only replace vanilla's registries with none at all.
    if registries.registries.is_empty() && registries.tags.is_empty() {
        return Vec::new();
    }
    let mut sent = Vec::new();
    if registries.custom_components {
        sent.push(C::CustomPayload(CustomPayload {
            channel: CUSTOM_COMPONENTS_CHANNEL.into(),
            data: Vec::new(),
        }));
    }
    let mut names: Vec<&String> = registries.registries.keys().collect();
    names.sort();
    for name in names {
        sent.push(C::RegistryData(RegistryData {
            registry: name.clone(),
            entries: registries.registries[name]
                .iter()
                .map(|(id, data)| RegistryEntry {
                    id: id.clone(),
                    data: data.clone(),
                })
                .collect(),
        }));
    }
    let mut tag_registries: Vec<&String> = registries.tags.keys().collect();
    tag_registries.sort();
    sent.push(C::UpdateTags(UpdateTags {
        registries: tag_registries
            .into_iter()
            .map(|registry| {
                let mut tags: Vec<TagEntry> = registries.tags[registry]
                    .iter()
                    .map(|(name, ids)| TagEntry {
                        name: name.clone(),
                        ids: ids.clone(),
                    })
                    .collect();
                tags.sort_by(|a, b| a.name.cmp(&b.name));
                RegistryTags {
                    registry: registry.clone(),
                    tags,
                }
            })
            .collect(),
    }));
    sent.into_iter()
        .map(|packet| {
            let encoded = packet.encode();
            let mut reader = Reader::new(&encoded);
            // The id is the encoding's first VarInt: it was just written.
            let id = reader.varint().unwrap_or_default();
            TimestampedPacket::new(0, id, reader.rest().to_vec())
        })
        .collect()
}

/// The registries a recording's configuration packets describe, or `None` when it has none
/// (a recording made before they were kept) so the caller keeps vanilla's. Packets that don't
/// decode, and ones this doesn't use, are skipped.
pub fn registries(packets: &[TimestampedPacket]) -> Option<Dynamic> {
    let mut dynamic = Dynamic::default();
    let mut any = false;
    for packet in packets {
        match C::decode(packet.id, &packet.payload) {
            Ok(C::RegistryData(data)) => {
                dynamic.add(data);
                any = true;
            }
            Ok(C::UpdateTags(tags)) => {
                dynamic.add_tags(&tags);
                any = true;
            }
            Ok(C::CustomPayload(payload)) if payload.channel == CUSTOM_COMPONENTS_CHANNEL => {
                dynamic.custom_components = true;
            }
            Ok(_) => {}
            Err(error) => log::debug!(
                "Studio replay skipped configuration packet {:#x}: {error}",
                packet.id
            ),
        }
    }
    any.then_some(dynamic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::mcpr::{McprReader, McprWriter, Metadata};
    use horizon_world::nbt::Tag;

    fn entry(name: &str, data: Option<Tag>) -> (String, Option<Tag>) {
        (name.to_owned(), data)
    }

    /// A server whose biome registry is not vanilla's order: the second entry here is where
    /// vanilla has `minecraft:plains`. Ids in the play stream only mean something with this list.
    fn server_registries() -> Dynamic {
        let mut dynamic = Dynamic::default();
        dynamic.add(RegistryData {
            registry: "minecraft:worldgen/biome".into(),
            entries: vec![
                RegistryEntry {
                    id: "custom:lush_flats".into(),
                    data: Some(Tag::compound([(
                        "temperature",
                        Tag::Float(0.8f32.to_bits()),
                    )])),
                },
                RegistryEntry {
                    id: "minecraft:desert".into(),
                    data: Some(Tag::compound([("downfall", Tag::Float(0.0f32.to_bits()))])),
                },
                RegistryEntry {
                    id: "minecraft:plains".into(),
                    data: None,
                },
            ],
        });
        dynamic.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![RegistryEntry {
                id: "minecraft:the_nether".into(),
                data: Some(Tag::compound([("min_y", Tag::Int(0))])),
            }],
        });
        dynamic.add_tags(&UpdateTags {
            registries: vec![RegistryTags {
                registry: "minecraft:block".into(),
                tags: vec![TagEntry {
                    name: "minecraft:logs".into(),
                    ids: vec![5, 9, 300],
                }],
            }],
        });
        dynamic.custom_components = true;
        dynamic
    }

    #[test]
    fn a_recording_hands_back_the_registries_it_was_made_with() {
        let dir = std::env::temp_dir().join(format!("studio_config_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.mcpr");

        let recorded = server_registries();
        let mut writer = McprWriter::new(path.clone(), Metadata::default()).unwrap();
        writer.set_configuration(&packets(&recorded)).unwrap();
        writer.write_packet(0, 0x2B, &[1, 2, 3]).unwrap();
        writer.finish(10).unwrap();

        let reader = McprReader::open(&path).unwrap();
        let played = registries(&reader.configuration).expect("the recording kept its registries");
        let _ = std::fs::remove_dir_all(&dir);

        // Entry order is the id order the play stream uses.
        assert_eq!(played.registries, recorded.registries);
        assert_eq!(
            played.id("minecraft:worldgen/biome", "minecraft:plains"),
            Some(2)
        );
        assert_eq!(
            played.name("minecraft:worldgen/biome", 0),
            Some("custom:lush_flats")
        );
        assert_eq!(
            played.tag("minecraft:block", "minecraft:logs"),
            Some(&[5, 9, 300][..])
        );
        assert!(played.custom_components);
    }

    #[test]
    fn a_recording_without_configuration_keeps_vanillas_registries() {
        let dir = std::env::temp_dir().join(format!("studio_config_old_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.mcpr");
        let mut writer = McprWriter::new(path.clone(), Metadata::default()).unwrap();
        writer.write_packet(0, 0x2B, &[1]).unwrap();
        writer.finish(0).unwrap();

        let reader = McprReader::open(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(reader.configuration.is_empty());
        assert!(registries(&reader.configuration).is_none());
    }

    #[test]
    fn a_packet_that_does_not_decode_is_skipped() {
        let mut stream = packets(&server_registries());
        stream.insert(
            0,
            TimestampedPacket::new(
                0,
                horizon_protocol::ids::configuration::clientbound::REGISTRY_DATA,
                vec![0xFF, 0xFF],
            ),
        );
        let played = registries(&stream).unwrap();
        assert_eq!(played.len("minecraft:worldgen/biome"), 3);
    }
}
