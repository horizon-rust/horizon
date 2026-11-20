use crate::game::{Progress, WorldRestLoader, native_entity_refs::tests::with_client_game};
use horizon_client::net::Event;
use horizon_protocol::connection::Pipe;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[test]
#[ignore = "daily"]
fn a_deferred_replacement_keeps_the_previous_login_until_entry_can_proceed() {
    with_client_game(|game| {
        let (client, server) = Pipe::pair();
        game.connect_local(client, "PreviousLogin").unwrap();
        let (finish, release) = mpsc::channel();
        game.rest_loader = Some(WorldRestLoader {
            progress: Progress::new(),
            handle: thread::spawn(move || {
                let _ = release.recv();
                Err("world resource fixture failed".into())
            }),
        });

        game.connect_server("127.0.0.1:1", "ReplacementLogin")
            .unwrap();
        assert!(game.pending_entry.is_some());
        finish.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !game.rest_loader.as_ref().unwrap().handle.is_finished() {
            assert!(Instant::now() < deadline, "world loader did not finish");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            game.resume_world_entry().unwrap_err().to_string(),
            "world resource fixture failed"
        );

        // The old session should still report its own disconnect. A login cancelled
        // during deferral suppresses this event even though its Remote stays installed.
        drop(server);
        let previous = &game.remote.as_ref().unwrap().session;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if previous
                .poll()
                .into_iter()
                .any(|receipt| matches!(receipt.into_parts().0, Event::Disconnected { .. }))
            {
                break;
            }
            assert!(Instant::now() < deadline, "previous login was cancelled");
            thread::sleep(Duration::from_millis(1));
        }
    });
}
