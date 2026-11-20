//! Making a rename durable: the folder that holds the renamed file synced.
use std::io;
use std::path::Path;

/// `folder` synced, so the renames into it are on the disk before what
/// depends on them is written. A no-op on Windows: a directory cannot be
/// opened and synced there the way POSIX allows, so a rename there stays
/// only as durable as NTFS's journal makes it.
pub fn sync_dir(folder: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(folder)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}
