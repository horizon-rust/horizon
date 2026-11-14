//! Several games at once (#15496), when Settings' "Run several instances at once" is on
//! (`Settings::several_games`, off by default: one game at a time, as before).
//!
//! The newest game stays in front (`Launcher::client`: its log, its crash report, Stop while its
//! instance is selected). Starting another instance moves the game in front to `background`,
//! where it keeps running until it ends; its play time and post-exit command then follow as the
//! front game's do. The same instance never runs twice, and a start under way always finishes
//! before the next one begins.
use super::*;

/// A game moved out of the front by a newer one.
pub(super) struct BackgroundGame {
    running: Running,
    /// Its instance and when it started, for its play time.
    run: Option<(String, Instant)>,
    /// Its instance variables, for its post-exit command.
    vars: horizon_launcher::launch::Vars,
}

/// Whether a game running or starting keeps the selected instance from starting: always while
/// a start is under way, and while any game runs unless `several` lets another instance run
/// beside it. `running` holds the instances whose games run; `selected` none (a pack's or no
/// instance) never runs beside another game.
pub(super) fn game_blocks(
    several: bool,
    starting: bool,
    any_running: bool,
    running: &[&str],
    selected: Option<&str>,
) -> bool {
    if starting {
        return true;
    }
    if !any_running {
        return false;
    }
    !several || selected.is_none_or(|id| running.contains(&id))
}

/// Whether a game's process has ended.
fn exited(game: &Running) -> bool {
    matches!(game.child.lock().unwrap().try_wait(), Ok(Some(_)))
}

impl Launcher {
    /// The instances whose games run, front and background.
    fn running_ids(&self) -> Vec<&str> {
        self.client_run
            .iter()
            .chain(self.background.iter().filter_map(|b| b.run.as_ref()))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Whether a game running or starting keeps the selected instance from starting now
    /// ([`game_blocks`]).
    pub(crate) fn game_blocks_start(&self) -> bool {
        let running = self.running_ids();
        game_blocks(
            self.settings.several_games,
            self.starting_client.is_some(),
            self.client.is_some() || !self.background.is_empty(),
            &running,
            self.selected_instance().map(|i| i.id.as_str()),
        )
    }

    /// Whether `id`'s game runs. With one game at a time, any running game counts as the
    /// selected instance's, as before.
    pub(crate) fn instance_running(&self, id: &str) -> bool {
        if self.settings.several_games {
            self.running_ids().contains(&id)
        } else {
            self.client.is_some()
        }
    }

    /// Whether `tag` can start now ([`starts_now`]): prepared, no preparation under way, no game
    /// starting, and none running unless several may ([`game_blocks`]).
    pub(crate) fn can_start(&self, tag: &str) -> bool {
        let blocked = self.game_blocks_start();
        starts_now(backend::is_prepared(tag), self.preparing, blocked)
    }

    /// Whether a server's Join starts the selected instance now: a Java instance too, as Play
    /// would (no game running or starting).
    pub(crate) fn can_join_selected(&self) -> bool {
        match self.selected_instance() {
            Some(i) if i.java.is_some() => !self.game_blocks_start(),
            _ => self.can_start_selected(),
        }
    }

    /// Whether the selected instance's game runs (Play becomes Stop).
    pub(crate) fn selected_running(&self) -> bool {
        match self.selected_instance() {
            Some(i) => self.instance_running(&i.id),
            None => self.client.is_some(),
        }
    }

    /// Stop the selected instance's game: the front game, or the background one it plays.
    pub(crate) fn stop_selected(&self) {
        let id = self.selected_instance().map(|i| i.id.as_str());
        let behind = self
            .background
            .iter()
            .find(|b| b.run.as_ref().map(|(r, _)| r.as_str()) == id);
        match behind {
            Some(b) if self.client_run.as_ref().map(|(r, _)| r.as_str()) != id => b.running.kill(),
            _ => {
                if let Some(client) = &self.client {
                    client.kill();
                }
            }
        }
    }

    /// A new game is about to take the front: the one there keeps running behind it.
    pub(super) fn move_front_game_back(&mut self) {
        if let Some(running) = self.client.take() {
            self.background.push(BackgroundGame {
                running,
                run: self.client_run.take(),
                vars: std::mem::take(&mut self.client_vars),
            });
        }
    }

    /// A game's process ended (`Msg::Exit` of `Job::Client`). When it was a background game (the
    /// front one still runs, or there is none), its play time and post-exit command are kept here
    /// and `true` says the front game's end handling must not run. One call may keep several
    /// ended background games; their later messages find the front game running and end here too.
    pub(super) fn background_game_ended(&mut self) -> bool {
        if self.client.as_ref().is_some_and(exited) {
            return false;
        }
        let (ended, still): (Vec<_>, Vec<_>) = std::mem::take(&mut self.background)
            .into_iter()
            .partition(|b| exited(&b.running));
        self.background = still;
        let front_runs = self.client.is_some();
        if ended.is_empty() {
            return front_runs;
        }
        for game in ended {
            let Some((id, started)) = game.run else {
                continue;
            };
            if let Err(e) = instances::record_play(&id, started.elapsed()) {
                eprintln!("Play time not saved: {e}");
            }
            if let Some(i) = instances::list().into_iter().find(|i| i.id == id) {
                horizon_launcher::launch::run_after_exit(&i.launch, &i.home, &game.vars);
            }
        }
        self.worlds.stale();
        self.screenshots.stale();
        self.reload_instances();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::game_blocks;

    #[test]
    fn one_game_at_a_time_unless_several_are_on() {
        // Nothing running: any instance starts.
        assert!(!game_blocks(false, false, false, &[], Some("a")));
        // Off (the default): any running game blocks every start, as before.
        assert!(game_blocks(false, false, true, &["a"], Some("b")));
        // On: another instance starts beside it, the same one never runs twice.
        assert!(!game_blocks(true, false, true, &["a"], Some("b")));
        assert!(game_blocks(true, false, true, &["a"], Some("a")));
        // A start under way, or a game with no instance selected, still waits.
        assert!(game_blocks(true, true, false, &[], Some("b")));
        assert!(game_blocks(true, false, true, &["a"], None));
    }
}
