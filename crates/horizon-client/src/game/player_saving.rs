//! The player traversal shared by forced saves and snapshot preparation.
use super::stats::{Stats, player_json};
use horizon_client::{
    Result,
    advancements::Tree,
    lan::{guest::Guest, playerdata},
    save::{Level, WorldPath, write_json},
};

pub(super) fn save_records(
    root: &WorldPath,
    tree: &Tree,
    guest: &Guest,
    stats: Option<&Stats>,
    uuid: [i32; 4],
) -> Result<()> {
    if !guest.placed {
        return Ok(());
    }
    let mut error = None;
    if let Some(stats) = stats.filter(|s| s.loaded)
        && let Err(next) = stats.write(&player_json(root, "stats", uuid))
    {
        error = Some(next);
    }
    if let Err(next) = write_json(
        &player_json(root, "advancements", uuid),
        &guest
            .advancements
            .to_json(tree, horizon_world::DATA_VERSION),
    ) {
        error.get_or_insert(next);
    }
    error.map_or(Ok(()), Err)
}
pub(super) fn save_online<'a>(
    root: &WorldPath,
    level: &mut Level,
    owner: Option<u128>,
    tree: &Tree,
    players: impl IntoIterator<Item = (&'a Guest, Option<&'a Stats>)>,
) -> Result<()> {
    let mut error = None;
    for (guest, stats) in players {
        if !guest.placed {
            continue;
        }
        let uuid = if owner == Some(guest.uuid) {
            playerdata::store(guest, level);
            level
                .player_uuid
                .ok_or("integrated owner has no saved UUID")?
        } else {
            playerdata::uuid_ints(guest.uuid)
        };
        if let Err(next) = playerdata::save_as(guest, root, uuid) {
            error.get_or_insert(next);
        }
        if let Err(next) = save_records(root, tree, guest, stats, uuid) {
            error.get_or_insert(next);
        }
    }
    error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{
        items::ItemStack,
        lan::{Joined, PlayerSender},
        save::{read_dat, read_string},
    };
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicI32, AtomicU64, Ordering},
            mpsc,
        },
    };

    struct Fixture(WorldPath);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "horizon-save-players-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(WorldPath::new(path))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn guest(id: u32) -> Guest {
        let (send, _) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: id,
                uuid: u128::from(id),
                name: format!("Player{id}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(send, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
            },
            id as i32,
            [f64::from(id), 72.0, -12.0],
            8,
        );
        guest.placed = true;
        guest.dimension = "minecraft:the_nether".into();
        guest.loaded_body = Some(horizon_client::lan::guest::SavedBody {
            fire_ticks: 0,
            air: 300,
            fall_distance: 2.0,
            velocity: [0.25, -0.5, 0.75],
        });
        guest.effects = horizon_client::effects::PlayerEffects::from_saved(&[(
            "minecraft:speed".into(),
            400,
            1,
            false,
            true,
        )]);
        guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", id));
        guest.armor[0] = Some(ItemStack::new("minecraft:diamond_helmet", 1));
        guest.offhand = Some(ItemStack::new("minecraft:shield", 1));
        guest.advancements.grant(
            "minecraft:story/root",
            "crafting_table",
            "2026-10-25 00:00:00 +0000",
        );
        guest
    }
    fn stats(id: i32) -> Stats {
        let mut stats = Stats::default();
        stats.loaded = true;
        stats.custom("jump", id);
        stats
    }
    fn read_player(root: &WorldPath, id: [i32; 4]) -> Level {
        let tag = read_dat(
            &root
                .join("players/data")
                .join(format!("{}.dat", horizon_client::save::uuid_text(id))),
        )
        .unwrap();
        let mut level = Level::default();
        level.read_player(&tag);
        level
    }
    fn read_json(root: &WorldPath, kind: &str, id: [i32; 4]) -> serde_json::Value {
        serde_json::from_str(&read_string(&player_json(root, kind, id)).unwrap()).unwrap()
    }

    #[test]
    fn save_online_persists_every_placed_player_with_empty_advancement_tree() {
        let f = Fixture::new();
        let (one, two) = (guest(1), guest(2));
        let (one_stats, two_stats) = (stats(13), stats(27));
        let mut unplaced = guest(3);
        unplaced.placed = false;
        save_online(
            &f.0,
            &mut Level::default(),
            None,
            &Tree::default(),
            [
                (&one, Some(&one_stats)),
                (&two, Some(&two_stats)),
                (&unplaced, None),
            ],
        )
        .unwrap();
        for (guest, jumps) in [(&one, 13), (&two, 27)] {
            let id = playerdata::uuid_ints(guest.uuid);
            let level = read_player(&f.0, id);
            assert_eq!(level.position, Some(guest.position));
            assert_eq!(level.dimension, guest.dimension);
            assert_eq!(level.motion, [0.25, -0.5, 0.75]);
            assert_eq!(level.active_effects, guest.effects.saved());
            assert!(
                level
                    .inventory
                    .iter()
                    .any(|s| s.0 == 0 && s.1 == "minecraft:diamond" && s.2 == guest.uuid as u32)
            );
            assert!(
                level
                    .inventory
                    .iter()
                    .any(|s| s.0 == 103 && s.1 == "minecraft:diamond_helmet")
            );
            assert!(
                level
                    .inventory
                    .iter()
                    .any(|s| s.0 == 150 && s.1 == "minecraft:shield")
            );
            assert_eq!(
                read_json(&f.0, "stats", id)["stats"]["minecraft:custom"]["minecraft:jump"],
                jumps
            );
            assert!(read_json(&f.0, "advancements", id)["minecraft:story/root"]["criteria"]["crafting_table"].is_string());
        }
        assert!(!playerdata::path(&f.0, unplaced.uuid).exists());
    }

    #[test]
    fn integrated_owner_keeps_the_saved_uuid_for_every_record() {
        let f = Fixture::new();
        let owner = guest(9);
        let stats = stats(6);
        let id = [7, 8, 9, 10];
        let mut level = Level {
            player_uuid: Some(id),
            ..Default::default()
        };
        save_online(
            &f.0,
            &mut level,
            Some(owner.uuid),
            &Tree::default(),
            [(&owner, Some(&stats))],
        )
        .unwrap();
        assert_eq!(read_player(&f.0, id).position, Some(owner.position));
        assert_eq!(level.position, Some(owner.position));
        assert!(player_json(&f.0, "stats", id).exists());
        assert!(player_json(&f.0, "advancements", id).exists());
        assert!(!playerdata::path(&f.0, owner.uuid).exists());
    }

    #[test]
    fn held_departure_reconnect_reads_live_inventory_stats_and_advancements() {
        let f = Fixture::new();
        let mut player = guest(4);
        let mut stats = stats(1);
        let id = playerdata::uuid_ints(player.uuid);
        save_online(
            &f.0,
            &mut Level::default(),
            None,
            &Tree::default(),
            [(&player, Some(&stats))],
        )
        .unwrap();
        let file = playerdata::path(&f.0, player.uuid);
        let before = fs::read(&file).unwrap();
        f.0.persistence().unwrap().begin("reconnect").unwrap();
        player.position[0] = 900.0;
        player.inventory[0] = Some(ItemStack::new("minecraft:emerald", 32));
        player
            .advancements
            .grant("test:new", "visited", "2026-10-25 01:00:00 +0000");
        stats.custom("jump", 8);
        playerdata::save(&player, &f.0).unwrap();
        save_records(&f.0, &Tree::default(), &player, Some(&stats), id).unwrap();
        let mut reconnect = guest(4);
        assert!(playerdata::load(&mut reconnect, &f.0));
        assert_eq!(reconnect.position, player.position);
        assert_eq!(reconnect.inventory[0], player.inventory[0]);
        assert_eq!(
            Stats::read(&player_json(&f.0, "stats", id)).get("custom", "minecraft:jump"),
            9
        );
        assert!(read_json(&f.0, "advancements", id)["test:new"]["criteria"]["visited"].is_string());
        assert_eq!(fs::read(&file).unwrap(), before);
        f.0.persistence().unwrap().end("reconnect").unwrap();
        assert_ne!(fs::read(&file).unwrap(), before);
        assert_eq!(read_player(&f.0, id).position, Some(player.position));
    }

    #[test]
    fn player_and_record_io_failures_reject_the_actual_online_save() {
        for kind in ["data", "stats", "advancements"] {
            let f = Fixture::new();
            fs::create_dir_all(f.0.join("players")).unwrap();
            fs::write(f.0.join("players").join(kind), b"not a directory").unwrap();
            let player = guest(1);
            let stats = stats(1);
            let error = save_online(
                &f.0,
                &mut Level::default(),
                None,
                &Tree::default(),
                [(&player, Some(&stats))],
            )
            .unwrap_err();
            assert!(
                error.to_string().contains(&format!("players/{kind}/")),
                "{error}"
            );
            assert!(f.0.persistence().unwrap().begin("failed").is_err());
        }
    }

    #[test]
    fn one_failed_record_does_not_skip_other_players_or_records() {
        let f = Fixture::new();
        fs::create_dir_all(f.0.join("players/stats")).unwrap();
        let first = guest(1);
        let second = guest(2);
        let first_id = playerdata::uuid_ints(first.uuid);
        let second_id = playerdata::uuid_ints(second.uuid);
        fs::create_dir(player_json(&f.0, "stats", first_id)).unwrap();
        let stats = stats(3);
        assert!(
            save_online(
                &f.0,
                &mut Level::default(),
                None,
                &Tree::default(),
                [(&first, Some(&stats)), (&second, Some(&stats))]
            )
            .is_err()
        );
        assert!(player_json(&f.0, "advancements", first_id).is_file());
        assert!(player_json(&f.0, "advancements", second_id).is_file());
        assert!(player_json(&f.0, "stats", second_id).is_file());
        assert!(playerdata::path(&f.0, first.uuid).is_file());
        assert!(playerdata::path(&f.0, second.uuid).is_file());
    }

    #[test]
    fn uninitialized_records_are_not_overwritten() {
        let f = Fixture::new();
        let mut player = guest(2);
        let stats = stats(12);
        let id = playerdata::uuid_ints(player.uuid);
        save_records(&f.0, &Tree::default(), &player, Some(&stats), id).unwrap();
        let before = fs::read(player_json(&f.0, "stats", id)).unwrap();
        let unloaded = Stats::default();
        save_records(&f.0, &Tree::default(), &player, Some(&unloaded), id).unwrap();
        assert_eq!(fs::read(player_json(&f.0, "stats", id)).unwrap(), before);
        player.placed = false;
        save_online(
            &f.0,
            &mut Level::default(),
            None,
            &Tree::default(),
            [(&player, Some(&stats))],
        )
        .unwrap();
        assert!(!playerdata::path(&f.0, player.uuid).exists());
    }
}
