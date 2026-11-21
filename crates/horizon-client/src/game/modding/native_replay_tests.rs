//! A client mod reads and drives replay playback, and the person's gameplay input stays off the
//! recording while it plays (#19247).
use super::native_replay::recording_path;
use horizon_client::packet_source::PacketSource;
use horizon_client::studio::mcpr::TimestampedPacket;
use mod_api::{ClientHost, Hooks, ModEntry, ReplayError};
use mod_platform::testing::with_mods;
use std::any::Any;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn a_recording_id_names_one_file_in_the_recordings_folder() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("recordings");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("my_trip_1.mcpr"), b"x").unwrap();
    std::fs::write(root.path().join("outside.mcpr"), b"x").unwrap();
    std::fs::create_dir(dir.join("folder.mcpr")).unwrap();

    assert_eq!(
        recording_path(&dir, "my_trip_1"),
        Ok(dir.join("my_trip_1.mcpr"))
    );
    // Refused as text, before any file is looked at.
    for id in [
        "",
        ".",
        "..",
        "../outside",
        "a/../my_trip_1",
        "sub/my_trip_1",
        "sub\\my_trip_1",
        "/etc/passwd",
        "\\\\server\\share\\x",
        "C:my_trip_1",
        "C:\\Windows\\x",
        "my_trip_1:stream",
        "my\0trip",
        "my\ntrip",
        &"a".repeat(251),
    ] {
        assert_eq!(
            recording_path(&dir, id),
            Err(ReplayError::InvalidId),
            "{id:?}"
        );
    }
    // A missing file, a folder, and the id with its extension: no such recording.
    for id in ["nothing_here", "folder", "my_trip_1.mcpr"] {
        assert_eq!(
            recording_path(&dir, id),
            Err(ReplayError::NotFound),
            "{id:?}"
        );
    }
}

#[test]
fn a_link_out_of_the_recordings_folder_is_no_recording() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("recordings");
    std::fs::create_dir(&dir).unwrap();
    let outside = root.path().join("secret.mcpr");
    std::fs::write(&outside, b"x").unwrap();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, dir.join("escape.mcpr"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, dir.join("escape.mcpr"));
    if let Err(error) = linked {
        // Windows makes links only with Developer Mode or the privilege.
        eprintln!("skipped: no symbolic link could be made: {error}");
        return;
    }
    assert_eq!(recording_path(&dir, "escape"), Err(ReplayError::NotFound));
}

/// A packet source standing in for a recording: its clock moves by the frame's time at its
/// speed unless paused, and it carries no packets.
struct Clock {
    position: f64,
    duration: u32,
    speed: f64,
    paused: bool,
}

impl Clock {
    fn new(duration: u32) -> Self {
        Self {
            position: 0.0,
            duration,
            speed: 1.0,
            paused: false,
        }
    }
}

impl PacketSource for Clock {
    fn position_ms(&self) -> u32 {
        self.position as u32
    }
    fn duration_ms(&self) -> u32 {
        self.duration
    }
    fn packet_count(&self) -> usize {
        0
    }
    fn progress(&self) -> f32 {
        (self.position / f64::from(self.duration)) as f32
    }
    fn speed(&self) -> f64 {
        self.speed
    }
    fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }
    fn is_paused(&self) -> bool {
        self.paused
    }
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }
    fn set_looping(&mut self, _: bool) {}
    fn is_finished(&self) -> bool {
        self.position_ms() >= self.duration
    }
    fn advance(&mut self, delta: Duration) -> io::Result<Vec<TimestampedPacket>> {
        if !self.paused {
            self.position = (self.position + delta.as_secs_f64() * 1000.0 * self.speed)
                .min(f64::from(self.duration));
        }
        Ok(Vec::new())
    }
    fn seek(&mut self, target_ms: u32) -> io::Result<()> {
        self.position = f64::from(target_ms.min(self.duration));
        Ok(())
    }
    fn seek_and_collect(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>> {
        self.seek(target_ms)?;
        Ok(Vec::new())
    }
    fn seek_forward(&mut self, target_ms: u32) -> io::Result<Vec<TimestampedPacket>> {
        self.seek(target_ms)?;
        Ok(Vec::new())
    }
    fn restart(&mut self) -> io::Result<()> {
        self.seek(0)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// What a replay editor mod hears.
struct Heard(Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn replay_opened(&self, _: &mut dyn ClientHost, event: &mut mod_api::ReplayOpenedEvent<'_>) {
        let line = format!(
            "opened {} {} {}",
            event.recording, event.duration_ms, event.standalone
        );
        self.0.lock().unwrap().push(line);
    }
    fn replay_advanced(&self, _: &mut dyn ClientHost, event: &mut mod_api::ReplayAdvancedEvent) {
        let line = format!("advanced {}", event.position_ms);
        self.0.lock().unwrap().push(line);
    }
    fn replay_closed(&self, _: &mut dyn ClientHost, event: &mut mod_api::ReplayClosedEvent<'_>) {
        let line = format!("closed {}", event.recording);
        self.0.lock().unwrap().push(line);
    }
    fn mouse_scrolling(
        &self,
        _: &mut dyn ClientHost,
        _: &mut mod_api::MouseScrollingEvent,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().push("wheel".into());
        mod_api::Flow::Continue
    }
}

fn with_heard(test: impl FnOnce(&mut crate::game::Game<'_>)) -> Vec<String> {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods =
        vec![ModEntry::new("editor", Box::new(Heard(Arc::clone(&heard))))].into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(test);
    });
    heard.lock().unwrap().clone()
}

/// A replay opened over a world (not standalone), as `open_studio_replay` leaves it.
fn play(game: &mut crate::game::Game<'_>, duration: u32) {
    game.replay.driver = Some(Box::new(Clock::new(duration)));
    game.replay.recording = "demo".into();
    game.replay.generation += 1;
    game.replay.standalone = false;
    game.replay.reported_ms = Some(0);
}

/// Pause, resume, speed and the state read act at once; seek and close wait for the frame's end.
/// The clock's moves are heard once a frame they happen, never on a paused frame, and a seek is
/// heard on the next frame.
#[test]
#[ignore = "daily"]
fn a_client_mod_drives_the_replay_and_hears_its_clock_once_a_move() {
    let heard = with_heard(|game| {
        let host: &mut dyn ClientHost = game;
        assert_eq!(host.replay_state(), None);
        assert_eq!(host.replay_pause(), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_resume(), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_set_speed(1.0), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_seek(0), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_close(), Err(ReplayError::NotOpen));
        assert_eq!(host.replay_open("../x"), Err(ReplayError::InvalidId));

        play(game, 10_000);
        let host: &mut dyn ClientHost = game;
        let state = host.replay_state().unwrap();
        assert_eq!(
            (
                state.recording.as_str(),
                state.standalone,
                state.duration_ms
            ),
            ("demo", false, 10_000)
        );
        for speed in [f64::NAN, f64::INFINITY, 0.0, -1.0, 20.5] {
            assert_eq!(host.replay_set_speed(speed), Err(ReplayError::InvalidSpeed));
        }
        assert_eq!(host.replay_state().unwrap().speed, 1.0);
        assert_eq!(host.replay_set_speed(2.0), Ok(()));
        assert_eq!(host.replay_pause(), Ok(()));
        assert!(host.replay_state().unwrap().paused);

        game.advance_studio_replay().unwrap();
        game.advance_studio_replay().unwrap();
        assert_eq!(game.replay_resume(), Ok(()));
        // A frame of 100 ms at twice the speed.
        game.replay.last_tick = Some(std::time::Instant::now() - Duration::from_millis(100));
        game.advance_studio_replay().unwrap();
        let moved = game.replay_state().unwrap().position_ms;
        assert!(moved > 0);

        assert_eq!(game.replay_seek(5_000), Ok(()));
        assert_eq!(game.replay_state().unwrap().position_ms, moved, "not yet");
        game.native_replay_requests();
        assert_eq!(game.replay_state().unwrap().position_ms, 5_000);
        assert_eq!(game.replay_seek(100), Err(ReplayError::BackwardSeek));
        game.replay_pause().unwrap();
        game.advance_studio_replay().unwrap();
        game.advance_studio_replay().unwrap();

        // A close asked of a replay that is replaced before the frame ends reaches nothing.
        game.replay_close().unwrap();
        game.replay.generation += 1;
        game.native_replay_requests();
        assert!(game.replay.playing());

        game.replay_close().unwrap();
        assert!(game.replay.playing(), "open until the frame ends");
        game.native_replay_requests();
        assert_eq!(game.replay_state(), None);
    });
    assert_eq!(heard.len(), 3, "{heard:?}");
    assert!(heard[0].starts_with("advanced "), "{heard:?}");
    assert_eq!(&heard[1..], ["advanced 5000", "closed demo"]);
}

/// While a replay plays with no free camera (a mod's camera draws it), the person's movement
/// keys and the focus pause stay off the recording, while a mod still hears the wheel.
#[test]
#[ignore = "daily"]
fn a_replay_keeps_gameplay_input_off_and_leaves_mods_the_wheel() {
    use horizon_client::held::Source;
    use winit::keyboard::KeyCode;
    let heard = with_heard(|game| {
        game.title = None;
        game.screen = crate::game::Screen::Playing;
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            )
            .unwrap(),
        );
        game.focus_lost = Some(std::time::Instant::now() - Duration::from_secs(1));
        game.pause_if_inactive();
        assert_eq!(game.screen, crate::game::Screen::Paused, "without a replay");

        game.screen = crate::game::Screen::Playing;
        game.mapped_key(KeyCode::KeyW, true, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        assert!(game.keys.raw(&KeyCode::KeyW), "walking without a replay");
        game.mapped_key(KeyCode::KeyW, false, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        play(game, 10_000);
        assert!(!game.studio.flying());
        game.mapped_key(KeyCode::KeyW, true, false, KeyCode::KeyW, Source::Keyboard)
            .unwrap();
        assert!(!game.keys.raw(&KeyCode::KeyW), "no walking in a replay");
        game.pause_if_inactive();
        assert_eq!(game.screen, crate::game::Screen::Playing, "no focus pause");
        game.replay_wheel(winit::event::MouseScrollDelta::LineDelta(0.0, 1.0));
    });
    assert_eq!(heard, ["wheel"]);
}

/// A recording `/studio record` could have written: the Play login, then nothing for `duration` ms.
fn write_recording(path: &std::path::Path, duration: u32) {
    use horizon_client::studio::{McprWriter, Metadata};
    use horizon_protocol::{buf::Reader, packets::play};
    let encoded = play::Clientbound::Login(play::Login {
        player_id: 1,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 8,
        simulation_distance: 8,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        online_mode: false,
        enforces_secure_chat: false,
    })
    .encode();
    let mut reader = Reader::new(&encoded);
    let id = reader.varint().unwrap();
    let meta = Metadata {
        duration,
        ..Default::default()
    };
    let mut writer = McprWriter::new(path.to_path_buf(), meta).unwrap();
    writer.write_packet(0, id, reader.rest()).unwrap();
    writer.finish(duration).unwrap();
}

/// A mod opens recordings by id from the title screen: the open waits for the frame's end and is
/// heard once it is done, a second one over it is heard after the first one's close, and a damaged
/// recording is refused at the call with nothing opened.
#[test]
#[ignore = "daily"]
fn a_client_mod_opens_recordings_by_id_at_the_frames_end() {
    let root = tempfile::tempdir().unwrap();
    write_recording(&root.path().join("trip.mcpr"), 1_000);
    write_recording(&root.path().join("trip2.mcpr"), 2_000);
    std::fs::write(root.path().join("damaged.mcpr"), b"not a zip").unwrap();
    let heard = with_heard(|game| {
        game.replay.recordings_dir = Some(root.path().to_path_buf());
        let host: &mut dyn ClientHost = game;
        assert_eq!(host.replay_open("damaged"), Err(ReplayError::Unreadable));
        assert_eq!(host.replay_open("missing"), Err(ReplayError::NotFound));
        assert_eq!(host.replay_open("trip"), Ok(()));
        assert_eq!(host.replay_state(), None, "opens at the frame's end");
        game.native_replay_requests();
        let state = game.replay_state().expect("open");
        assert_eq!(
            (state.recording.as_str(), state.duration_ms),
            ("trip", 1_000)
        );

        assert_eq!(game.replay_open("trip2"), Ok(()));
        game.native_replay_requests();
        assert_eq!(game.replay_state().unwrap().recording, "trip2");
        game.replay_close().unwrap();
        game.native_replay_requests();
        assert_eq!(game.replay_state(), None);
    });
    let standalone = heard[0].rsplit(' ').next().unwrap().to_owned();
    assert_eq!(
        heard,
        [
            format!("opened trip 1000 {standalone}"),
            "closed trip".to_owned(),
            format!("opened trip2 2000 {standalone}"),
            "closed trip2".to_owned(),
        ]
    );
}
