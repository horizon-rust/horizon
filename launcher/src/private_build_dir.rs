//! The native build folder for a data folder whose path isn't ASCII (Windows, #6009): a private,
//! unguessable folder of ProgramData's ([`crate::backend::build_dir`]).
use crate::backend::tool_safe;
use std::fs;
use std::path::{Path, PathBuf};

/// Takes `dir` from ProgramData's inherited permissions: full control for this account only.
#[cfg(windows)]
pub(crate) fn lock_to_this_account(dir: &Path) -> bool {
    let Ok(user) = std::env::var("USERNAME") else {
        return false;
    };
    let account = match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    };
    let icacls = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join(r"System32\icacls.exe"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\icacls.exe"));
    crate::process::command(icacls)
        .arg(dir)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{account}:(OI)(CI)F"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `%ProgramData%`, or `C:\ProgramData` when it is unset, relative or not [`tool_safe`](crate::backend).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn program_data(program_data: Option<PathBuf>) -> PathBuf {
    program_data
        .filter(|p| p.is_absolute() && tool_safe(p))
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

/// The build folder for a data folder `real` whose path isn't `ascii`: a folder of `public`
/// (ProgramData) with a random name, made new and handed to `lock` to shut other accounts out,
/// and remembered in `real` (which only this account can write). ProgramData lets every account
/// create folders, and add files below the folders others made, so a name anyone could work out
/// (it was a hash of `real`) let another account plant the toolchain or a Cargo configuration
/// there before the player's first build. None when no such folder can be made.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn private_build_dir(
    real: &Path,
    public: &Path,
    lock: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    const PREFIX: &str = "horizon-launcher-";
    let ours = |name: &str| {
        name.strip_prefix(PREFIX)
            .is_some_and(|key| key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let remembered = real.join("build-folder.txt");
    if let Some(dir) = fs::read_to_string(&remembered)
        .ok()
        .filter(|name| ours(name.trim()))
        .map(|name| public.join(name.trim()))
        .filter(|dir| dir.is_dir())
    {
        return Some(dir);
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).ok()?;
    let name: String = std::iter::once(PREFIX.to_owned())
        .chain(random.iter().map(|b| format!("{b:02x}")))
        .collect();
    let dir = public.join(&name);
    // `create_dir`, not `create_dir_all`: a folder that is already there is someone else's.
    fs::create_dir(&dir).ok()?;
    if lock(&dir) && fs::write(&remembered, &name).is_ok() {
        return Some(dir);
    }
    let _ = fs::remove_dir(&dir);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ascii;

    /// [#6009](https://github.com/horizon-rust/horizon/issues/6009): with a Cyrillic user name the
    /// build folder is a real ASCII folder of ProgramData's (not a short name of the data folder,
    /// which rustc resolves back to the Cyrillic one). Its name can't be worked out by another
    /// account, it is made new and locked, and the same folder is used next time.
    #[test]
    fn a_cyrillic_data_folder_builds_in_a_private_ascii_folder() {
        let root = std::env::temp_dir().join(format!("horizon-build-dir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (pavlo, olena, public) = (
            root.join("Кириченко Павло"),
            root.join("Олена"),
            root.join("ProgramData"),
        );
        for dir in [&pavlo, &olena, &public] {
            fs::create_dir_all(dir).unwrap();
        }
        let locked = std::cell::RefCell::new(Vec::new());
        let lock = |dir: &Path| {
            locked.borrow_mut().push(dir.to_path_buf());
            true
        };
        let dir = private_build_dir(&pavlo, &public, lock).unwrap();
        assert!(ascii(&dir), "{}", dir.display());
        assert_eq!(dir.parent(), Some(public.as_path()));
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("horizon-launcher-") && name.len() == 49,
            "{name}"
        );
        assert_eq!(
            *locked.borrow(),
            std::slice::from_ref(&dir),
            "locked when made"
        );
        assert_eq!(
            private_build_dir(&pavlo, &public, lock),
            Some(dir.clone()),
            "remembered"
        );
        assert_eq!(locked.borrow().len(), 1, "not made again");
        let other = private_build_dir(&olena, &public, lock).unwrap();
        assert_ne!(other, dir, "one per user");
        // A folder that can't be locked is not used, and not left behind.
        let unlockable = root.join("Дмитро");
        fs::create_dir_all(&unlockable).unwrap();
        assert_eq!(private_build_dir(&unlockable, &public, |_| false), None);
        assert_eq!(fs::read_dir(&public).unwrap().count(), 2);
        // A remembered name that isn't one of ours is not followed.
        fs::write(pavlo.join("build-folder.txt"), r"..\..\Windows").unwrap();
        let fresh = private_build_dir(&pavlo, &public, lock).unwrap();
        assert_ne!(fresh, dir);
        assert_eq!(fresh.parent(), Some(public.as_path()));
        // %ProgramData% unset, relative or itself not ASCII: C:\ProgramData.
        let default = PathBuf::from(r"C:\ProgramData");
        for pd in [None, Some("ProgramData".into()), Some(r"C:\Дані".into())] {
            assert_eq!(program_data(pd), default);
        }
        let _ = fs::remove_dir_all(&root);
    }
}
