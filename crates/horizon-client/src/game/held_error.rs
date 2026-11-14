//! The error that ends a game, kept until the loop can report it.
//!
//! `Game` moves to other threads (a region's tick), so the error it holds has to be `Send`: it keeps the
//! message of the error it was made from, which is all anything reads of it.
use std::fmt;

#[derive(Debug)]
pub(super) struct HeldError(String);

impl fmt::Display for HeldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HeldError {}

impl From<Box<dyn std::error::Error>> for HeldError {
    fn from(e: Box<dyn std::error::Error>) -> Self {
        Self(e.to_string())
    }
}

impl From<&str> for HeldError {
    fn from(e: &str) -> Self {
        Self(e.to_owned())
    }
}

impl From<String> for HeldError {
    fn from(e: String) -> Self {
        Self(e)
    }
}
