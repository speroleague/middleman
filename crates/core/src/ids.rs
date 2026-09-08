//! Identifier and hash types.
//!
//! Identifiers are prefixed ULIDs (`evt_`, `ent_`, `task_`, `prop_`,
//! `proj_`), matching the spec's `task_01H...` examples. Construction
//! validates the format, so an invalid identifier cannot exist.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::Error;

/// A blake3 digest, serialized as 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Hash(blake3::Hash);

impl Hash {
    pub fn of(input: &[u8]) -> Self {
        Self(blake3::hash(input))
    }

    /// Hash every genesis event chains back to.
    pub fn genesis() -> Self {
        Self::of(b"middleman/genesis")
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    pub fn to_hex(&self) -> String {
        self.0.to_hex().to_string()
    }

    /// Finalizes a blake3 hasher into a `Hash`; the only place the
    /// hasher type is accepted, keeping the blake3 surface at one seam.
    pub fn of_finalized(hasher: &blake3::Hasher) -> Self {
        Self(hasher.finalize())
    }
}

impl FromStr for Hash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<blake3::Hash>()
            .map(Self)
            .map_err(|_| Error::InvalidHash(s.to_owned()))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Hash").field(&self.to_hex()).finish()
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(D::Error::custom)
    }
}

/// Validates `prefix` + 26 Crockford base32 characters (a ULID).
fn validate_ulid_id(prefix: &str, raw: &str) -> Result<String, Error> {
    let body = raw
        .strip_prefix(prefix)
        .ok_or_else(|| Error::InvalidId(raw.to_owned()))?;
    ulid::Ulid::from_string(body)
        .map(|_| raw.to_owned())
        .map_err(|_| Error::InvalidId(raw.to_owned()))
}

/// Display/debug/serde protocol shared by every identifier type.
macro_rules! impl_id_protocol {
    ($name:ident, $prefix:literal) => {
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                let validated = validate_ulid_id($prefix, &raw).map_err(D::Error::custom)?;
                Ok(Self(validated))
            }
        }
    };
}

/// Identifier of one entry in the event log.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(String);

impl EventId {
    pub fn new(raw: impl AsRef<str>) -> Result<Self, Error> {
        validate_ulid_id("evt_", raw.as_ref()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_id_protocol!(EventId, "evt_");

/// Identifier of a projected entity (module, test, decision, ...).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(String);

impl EntityId {
    pub fn new(raw: impl AsRef<str>) -> Result<Self, Error> {
        validate_ulid_id("ent_", raw.as_ref()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_id_protocol!(EntityId, "ent_");

/// Identifier of one recorded task.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(raw: impl AsRef<str>) -> Result<Self, Error> {
        validate_ulid_id("task_", raw.as_ref()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_id_protocol!(TaskId, "task_");

/// Identifier of one durable-memory proposal.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProposalId(String);

impl ProposalId {
    pub fn new(raw: impl AsRef<str>) -> Result<Self, Error> {
        validate_ulid_id("prop_", raw.as_ref()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_id_protocol!(ProposalId, "prop_");

/// Identifier of one indexed repository.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(raw: impl AsRef<str>) -> Result<Self, Error> {
        validate_ulid_id("proj_", raw.as_ref()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_id_protocol!(ProjectId, "proj_");

#[cfg(test)]
pub(crate) mod test_support {
    /// Deterministic well-formed ULID for tests: timestamp + counter body.
    pub fn ulid_for(sequence: u64) -> ulid::Ulid {
        let mut bytes = [0u8; 16];
        let ts = 1_700_000_000_000u64 + sequence;
        bytes[..6].copy_from_slice(&ts.to_be_bytes()[2..]);
        bytes[6..14].copy_from_slice(&sequence.to_be_bytes());
        ulid::Ulid::from_bytes(bytes)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn id(prefix: &str, sequence: u64) -> String {
        format!("{prefix}{}", test_support::ulid_for(sequence))
    }

    #[test]
    fn accepts_prefixed_ulids_and_rejects_malformed_values() {
        assert!(TaskId::new(id("task_", 7)).is_ok());
        assert!(EventId::new(id("evt_", 3)).is_ok());
        assert!(EntityId::new(id("ent_", 1)).is_ok());
        assert!(ProposalId::new(id("prop_", 2)).is_ok());
        assert!(ProjectId::new(id("proj_", 9)).is_ok());

        let malformed = [
            "task_",                             // missing body
            "task_01HXYZ",                       // too short
            "evt_01HXYZ0000000000000000A",       // wrong prefix
            "task_0123456789abcdef456789abcdef", // not base32
            "task_01HXY000000000000000000",      // 25 chars, not 26
        ];
        for raw in malformed {
            assert!(TaskId::new(raw).is_err(), "should reject `{raw}`");
            assert!(EventId::new(raw).is_err(), "should reject `{raw}`");
        }
        assert!(serde_json::from_value::<TaskId>(serde_json::json!("task_nope")).is_err());
    }

    #[test]
    fn hash_round_trips_through_serde_and_hex() {
        let hash = Hash::of(b"payload one");
        let json = serde_json::to_value(hash).expect("hash serializes");
        assert!(json.is_string());
        let back: Hash = serde_json::from_value(json).expect("hash deserializes");
        assert_eq!(hash, back);
        assert_eq!(hash.to_hex(), back.to_hex());
        assert!(Hash::from_str("nope").is_err());
        assert_ne!(Hash::of(b"payload one"), Hash::of(b"payload two"));
    }
}
