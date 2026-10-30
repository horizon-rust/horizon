//! The running program's path. std has no `current_exe` on the Nintendo Switch (Horizon), where
//! the homebrew loader passes the NRO's path as `argv[0]` instead.
use std::{io, path::PathBuf};

pub fn current_exe() -> io::Result<PathBuf> {
    std::env::current_exe().or_else(|error| {
        std::env::args_os()
            .next()
            .filter(|arg| !arg.is_empty())
            .map(PathBuf::from)
            .ok_or(error)
    })
}
