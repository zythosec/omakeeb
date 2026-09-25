//! Failures a caller can put on screen.

use std::fmt;

/// What went wrong talking to a keyboard, or reading a definition.
#[derive(Debug)]
pub enum Error {
    /// A command the firmware does not implement. Callers fall back or skip.
    Unhandled,
    /// The raw HID node exists, and this user cannot open it.
    Permission { path: String, detail: String },
    /// Anything else, already phrased for a person.
    Message(String),
}

impl Error {
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unhandled => write!(f, "the keyboard does not implement that command"),
            Self::Permission { path, detail } => {
                write!(f, "cannot open {path} ({detail})")
            }
            Self::Message(text) => write!(f, "{text}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
