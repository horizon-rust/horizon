//! Worlds and the multiplayer server list from the control socket: what the Select World, Edit
//! World and Multiplayer screens do, by world folder. Changing a world needs the title screen
//! (no world open), as those screens do; deleting one needs `--confirm`.
use super::Game;
use horizon_client::{Result, title};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc::Sender, time::UNIX_EPOCH};

pub(super) const HELP: &[&str] = &[
    "worlds                         the saved worlds, newest first: folder, name, mode, last played, version",
    "world-rename FOLDER NAME       rename a world (Edit World; its folder stays)",
    "world-backup FOLDER            zip a world into backups/ (Edit World > Make Backup)",
    "world-delete FOLDER --confirm  delete a world for good (Select World > Delete)",
    "servers                        the multiplayer server list (servers.dat): name and address",
];

pub(super) fn saves() -> Result<PathBuf> {
    Ok(std::env::current_dir()?.join("saves"))
}

/// The world folder that `rest` (the line after the command's name) starts with, and the
/// words after it. A folder may hold spaces ("New World" is vanilla's default) while the
/// socket's commands are words, and horizon-ctl joins its arguments with spaces: a folder
/// in double quotes is taken whole; else the longest run of words that names a folder for
/// which `exists` is true; else the first word (a world `open` is about to create).
pub(super) fn split_folder(
    rest: &str,
    exists: impl Fn(&str) -> bool,
) -> Option<(String, Vec<&str>)> {
    let rest = rest.trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        let (folder, after) = quoted.split_once('"')?;
        return Some((folder.to_owned(), after.split_whitespace().collect()));
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    let taken = (2..=words.len())
        .rev()
        .find(|&n| exists(&words[..n].join(" ")))
        .unwrap_or(1);
    let first = words.first()?;
    let folder = if taken == 1 {
        (*first).to_owned()
    } else {
        words[..taken].join(" ")
    };
    Some((folder, words[taken..].to_vec()))
}

impl Game<'_> {
    /// Some(reply) for these commands (Some(None): the reply comes later), None for others.
    pub(super) fn saves_control(
        &mut self,
        words: &[&str],
        line: &str,
        reply: &Sender<String>,
    ) -> Result<Option<Option<Value>>> {
        // The folder and the words after it (world-rename's name, world-delete's --confirm).
        let folder = || -> Result<(String, Vec<&str>)> {
            let rest = line.trim_start().strip_prefix(words[0]).unwrap_or("");
            let saves = saves()?;
            split_folder(rest, |f| saves.join(f).is_dir())
                .ok_or_else(|| format!("{} FOLDER: the folder from worlds", words[0]).into())
        };
        let on_title = || {
            if self.title.is_some() {
                Ok(())
            } else {
                Err(format!(
                    "{} needs the title screen: save and leave the world first (pause, then Save and Quit to Title)",
                    words[0]
                ))
            }
        };
        let value = match words[0] {
            "worlds" => {
                let worlds: Vec<Value> = title::list_worlds(&saves()?)
                    .iter()
                    .map(|w| {
                        json!({
                            "folder": w.folder,
                            "name": w.name,
                            "mode": if w.creative { "creative" } else { "survival" },
                            "last_played": w.modified.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
                            "commands": w.commands,
                            "version": w.version,
                        })
                    })
                    .collect();
                json!({ "worlds": worlds })
            }
            "world-rename" => {
                on_title()?;
                let (folder, after) = folder()?;
                let name = after.join(" ");
                if name.is_empty() {
                    return Err("world-rename FOLDER NAME: missing the new name".into());
                }
                title::world_management::rename(&saves()?, &folder, &name)?;
                if let Some(title) = self.title.as_mut() {
                    title.worlds_changed();
                }
                json!({ "ok": true, "folder": folder, "name": name })
            }
            "world-delete" => {
                on_title()?;
                let (folder, after) = folder()?;
                if !after.contains(&"--confirm") {
                    return Err(format!(
                        "world-delete deletes {folder} for good: ask the player, then add --confirm"
                    )
                    .into());
                }
                title::world_management::delete(&saves()?, &folder)?;
                if let Some(title) = self.title.as_mut() {
                    title.worlds_changed();
                }
                json!({ "ok": true, "deleted": folder })
            }
            "world-backup" => {
                on_title()?;
                let (folder, _) = folder()?;
                let saves = saves()?;
                let backups = title::world_management::backups_path(&saves);
                // A big world takes a while to zip: off the game thread, as Make Backup does.
                let reply = reply.clone();
                std::thread::spawn(move || {
                    let answer = match title::world_management::backup(&saves, &folder) {
                        Ok(bytes) => {
                            json!({ "ok": true, "folder": folder, "backups": backups, "bytes": bytes })
                        }
                        Err(e) => json!({ "error": format!("backup of {folder} failed: {e}") }),
                    };
                    let _ = reply.send(answer.to_string());
                });
                return Ok(Some(None));
            }
            "servers" => {
                let file = horizon_client::multiplayer::servers_file(&saves()?);
                let servers: Vec<Value> = horizon_protocol::servers::load(&file)
                    .iter()
                    .map(|s| json!({ "name": s.name, "address": s.ip }))
                    .collect();
                json!({ "servers": servers })
            }
            _ => return Ok(None),
        };
        Ok(Some(Some(value)))
    }
}

#[cfg(test)]
mod tests {
    use super::split_folder;

    fn split(rest: &str, folders: &[&str]) -> Option<(String, Vec<String>)> {
        split_folder(rest, |f| folders.contains(&f))
            .map(|(folder, after)| (folder, after.iter().map(|w| (*w).to_owned()).collect()))
    }

    fn owned(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn a_folder_with_spaces_that_exists() {
        let worlds = ["New World", "New"];
        assert_eq!(
            split(" New World", &worlds),
            Some(("New World".into(), vec![]))
        );
        assert_eq!(
            split(" New World 42 creative", &worlds),
            Some(("New World".into(), owned(&["42", "creative"])))
        );
        assert_eq!(
            split(" New World My Base", &worlds),
            Some(("New World".into(), owned(&["My", "Base"])))
        );
        assert_eq!(
            split(" New --confirm", &worlds),
            Some(("New".into(), owned(&["--confirm"])))
        );
    }

    #[test]
    fn quoted_folders_and_new_ones() {
        assert_eq!(
            split(r#" "My New World" 7 creative"#, &[]),
            Some(("My New World".into(), owned(&["7", "creative"])))
        );
        assert_eq!(split(r#" "unclosed"#, &[]), None);
        assert_eq!(split(" Test 3", &[]), Some(("Test".into(), owned(&["3"]))));
        assert_eq!(split("  ", &[]), None);
    }
}
