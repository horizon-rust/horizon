//! Account and unsigned-developer policy for CLI Play.
use super::*;

/// [`client`], playing as `who` for this launch only (`play --account`).
pub fn client_as(tag: &str, who: Option<&str>) -> Result<Command, String> {
    if let Some(error) = game_architecture_launch_error(tag) {
        return Err(error);
    }
    play_step("reading the sign-in");
    let session = if crate::developer_offline::enabled(tag) && who.is_none() {
        crate::developer_offline::require_capability(tag)?;
        None
    } else {
        Some(match who {
            None => account::session_for_game(&game_home()),
            // An account that isn't signed in is the only `--account` refusal left: anything
            // the services can't answer plays offline instead.
            Some(who) => account::session_for_game_as(&game_home(), who)
                .map_err(|e| format!("{e} {}", crate::i18n::tr(SIGN_IN_HINT)))?,
        })
    };
    if let Err(e) = need_prepared(tag) {
        account::discard_unread_sessions();
        return Err(e);
    }
    play_step("checking the game's assets");
    complete_assets(tag);
    crate::browser::horizon::before_play(&game_home());
    let mut cmd = client_command(tag);
    match session {
        Some(session) => {
            if session.file.is_none() {
                set_local_name(&mut cmd, &Settings::load());
            }
            if let Some(file) = session.file {
                cmd.arg("--session-file").arg(file);
            }
            cmd.envs(session.env);
            if let Some(e) = session.offline {
                eprintln!("Playing offline: {e}");
            }
            if let Some(e) = account::take_unsaved() {
                eprintln!("{e}");
            }
        }
        None => {
            crate::developer_offline::mark(&mut cmd);
            eprintln!("{}", crate::i18n::tr(crate::developer_offline::NOTICE));
        }
    }
    append_game_args(&mut cmd);
    Ok(cmd)
}

/// The name to play under when no account's session goes with the game: the player's own
/// (`Settings::player_name`), or the game's own kept name when that is empty. The game reads it
/// as `HORIZON_PLAYER_NAME` and keeps the offline UUID of whatever name it plays under.
pub fn set_local_name(cmd: &mut Command, settings: &Settings) {
    if let Some(name) = settings.player_name.as_deref() {
        if !name.is_empty() {
            cmd.env("HORIZON_PLAYER_NAME", name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_name_goes_to_the_game_only_when_configured() {
        let named = |settings: &Settings| {
            let mut cmd = Command::new("game");
            set_local_name(&mut cmd, settings);
            cmd.get_envs()
                .find(|(key, _)| *key == "HORIZON_PLAYER_NAME")
                .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };
        let with = |name: Option<&str>| Settings {
            player_name: name.map(str::to_owned),
            ..Settings::default()
        };
        assert_eq!(named(&with(Some("Notch"))).as_deref(), Some("Notch"));
        // Nothing configured, or configured empty: the game keeps its own kept name.
        assert_eq!(named(&with(None)), None);
        assert_eq!(named(&with(Some(""))), None);
    }
}
