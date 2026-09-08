//! Typed errors for the pure core.
//!
//! Every variant is deterministic: the same input always yields the
//! same error, which keeps errors testable and diffable.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid identifier `{0}`")]
    InvalidId(String),

    #[error("invalid hash value `{0}`")]
    InvalidHash(String),

    #[error("event hash verification failed at sequence {0}")]
    HashBroken(u64),

    #[error("event sequence out of order: expected {expected}, found {found}")]
    SequenceMismatch { expected: u64, found: u64 },

    #[error("event {sequence}: {reason}")]
    InvalidEvent { sequence: u64, reason: String },

    #[error("entity {id}: {reason}")]
    InvalidEntity { id: String, reason: String },

    #[error("project log starts with {0}, expected ProjectInitialized")]
    FirstEventNotInitialization(String),
}

impl Error {
    pub(crate) fn invalid_event(sequence: u64, reason: impl Into<String>) -> Self {
        Self::InvalidEvent {
            sequence,
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_entity(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidEntity {
            id: id.into(),
            reason: reason.into(),
        }
    }
}
