//! The error that ends a game, kept until the loop can report it.
//!
//! `Game` moves to other threads (a region's tick), so the error it holds has to be `Send`: it keeps the
//! message of the error it was made from, and the debug crash (F3+C held) as itself, so `main` still reports
//! it as vanilla's manually triggered crash.
use std::fmt;

use horizon_client::crash::DebugCrash;

#[derive(Debug)]
pub(super) enum HeldError {
    /// The held debug crash keys: reported as vanilla's own crash.
    Debug(DebugCrash),
    /// Any other error, as its message.
    Message(String),
}

impl HeldError {
    /// The error `main` gets back: the debug crash is still one, anything else keeps its message.
    pub(super) fn boxed(self) -> Box<dyn std::error::Error> {
        match self {
            Self::Debug(crash) => Box::new(crash),
            Self::Message(message) => message.into(),
        }
    }
}

impl fmt::Display for HeldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Debug(crash) => crash.fmt(f),
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for HeldError {}

impl From<Box<dyn std::error::Error>> for HeldError {
    fn from(e: Box<dyn std::error::Error>) -> Self {
        match e.downcast::<DebugCrash>() {
            Ok(crash) => Self::Debug(*crash),
            Err(e) => Self::Message(e.to_string()),
        }
    }
}

impl From<&str> for HeldError {
    fn from(e: &str) -> Self {
        Self::Message(e.to_owned())
    }
}

impl From<String> for HeldError {
    fn from(e: String) -> Self {
        Self::Message(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_debug_crash_is_still_one_when_main_gets_it() {
        let held: HeldError = Box::<dyn std::error::Error>::from(DebugCrash::new()).into();
        assert!(held.boxed().downcast_ref::<DebugCrash>().is_some());
        let other: HeldError = Box::<dyn std::error::Error>::from("broken").into();
        let other = other.boxed();
        assert!(other.downcast_ref::<DebugCrash>().is_none());
        assert_eq!(other.to_string(), "broken");
    }
}
