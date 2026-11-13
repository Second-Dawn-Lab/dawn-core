//! Errors of Quality mode. Every one has a stable `code()` the UI maps to a message; none of them is fatal for the
//! app: the caller keeps the fast translation.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    /// Quality mode is off, or no model / runtime is installed.
    Unavailable(String),
    /// Not enough free memory to load the model (bytes).
    LowMemory { need: u64, free: u64 },
    Cancelled,
    /// The local server could not be reached or answered badly.
    Server(String),
    /// The model's answer was rejected by the output checks; the fast translation stays.
    Rejected(crate::validate::Reject),
    /// An address that is not on this computer was refused.
    NotLocal(String),
}

impl LlmError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "unavailable",
            Self::LowMemory { .. } => "low-memory",
            Self::Cancelled => "cancelled",
            Self::Server(_) => "server",
            Self::Rejected(reason) => reason.code(),
            Self::NotLocal(_) => "not-local",
        }
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "Quality mode is not available: {why}"),
            Self::LowMemory { need, free } => {
                write!(f, "Not enough free memory for the model: it needs about {:.1} GB, {:.1} GB are free", *need as f64 / 1e9, *free as f64 / 1e9)
            }
            Self::Cancelled => write!(f, "cancelled"),
            Self::Server(why) => write!(f, "The local model did not answer: {why}"),
            Self::Rejected(r) => write!(f, "The model's answer was not used ({r:?})"),
            Self::NotLocal(addr) => write!(f, "{addr} is not a local address; Quality mode only talks to this computer"),
        }
    }
}

impl std::error::Error for LlmError {}

pub type Result<T> = std::result::Result<T, LlmError>;
