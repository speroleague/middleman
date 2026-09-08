//! Entities, edges, and evidence — the durable facts the event log projects.
//!
//! `Entity` pairs a stable id with a kind-specific payload; `Entity::new`
//! refuses kind/payload mismatches so invalid states cannot be constructed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::Error;
use crate::ids::EntityId;

/// Kinds of durable facts (spec section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Module,
    Symbol,
    Test,
    Document,
    Decision,
    Invariant,
    Contract,
    Task,
    Risk,
    OpenQuestion,
    Operation,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Symbol => "symbol",
            Self::Test => "test",
            Self::Document => "document",
            Self::Decision => "decision",
            Self::Invariant => "invariant",
            Self::Contract => "contract",
            Self::Task => "task",
            Self::Risk => "risk",
            Self::OpenQuestion => "open_question",
            Self::Operation => "operation",
        }
    }
}

/// Lifecycle of an entity. Only `Active` entries are authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Superseded,
    Proposed,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Kind-specific data for an entity. The `kind` field on `Entity` must
/// agree with the payload variant; `Entity::new` enforces the match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntityPayload {
    Module {
        path: PathBuf,
        language: Option<String>,
        responsibility: String,
        public_surface: Vec<String>,
    },
    Symbol {
        path: PathBuf,
        span: Option<String>,
        symbol_kind: String,
        signature: Option<String>,
        visibility: String,
    },
    Test {
        path: PathBuf,
        scope: String,
        command: String,
        covered: Vec<EntityId>,
    },
    Document {
        path: PathBuf,
        title: String,
        kind: String,
        authority: String,
    },
    Decision {
        statement: String,
        rationale: Option<String>,
        owner: Option<String>,
        supersedes: Option<EntityId>,
    },
    Invariant {
        statement: String,
        consequence: String,
    },
    Contract {
        what: String,
        input: String,
        output: String,
        compatibility: Option<String>,
        owner: Option<String>,
        path: Option<PathBuf>,
    },
    Risk {
        condition: String,
        severity: Severity,
        mitigation: Option<String>,
        owner: Option<String>,
    },
    OpenQuestion {
        question: String,
        blocking: bool,
    },
    Operation {
        runbook: String,
        environment: Option<String>,
        recovery: Option<String>,
    },
}

impl EntityPayload {
    fn expected_kind(&self) -> EntityKind {
        match self {
            Self::Module { .. } => EntityKind::Module,
            Self::Symbol { .. } => EntityKind::Symbol,
            Self::Test { .. } => EntityKind::Test,
            Self::Document { .. } => EntityKind::Document,
            Self::Decision { .. } => EntityKind::Decision,
            Self::Invariant { .. } => EntityKind::Invariant,
            Self::Contract { .. } => EntityKind::Contract,
            Self::Risk { .. } => EntityKind::Risk,
            Self::OpenQuestion { .. } => EntityKind::OpenQuestion,
            Self::Operation { .. } => EntityKind::Operation,
        }
    }
}

/// One durable fact with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub status: Status,
    pub title: String,
    pub payload: EntityPayload,
    pub evidence: Vec<Evidence>,
}

impl Entity {
    /// Refuses kind/payload mismatches and empty titles at construction.
    pub fn new(
        id: EntityId,
        kind: EntityKind,
        status: Status,
        title: String,
        payload: EntityPayload,
        evidence: Vec<Evidence>,
    ) -> Result<Self, Error> {
        if payload.expected_kind() != kind {
            return Err(Error::invalid_entity(
                id.as_str(),
                format!(
                    "kind `{}` does not match payload kind `{}`",
                    kind.as_str(),
                    payload.expected_kind().as_str()
                ),
            ));
        }
        if title.trim().is_empty() {
            return Err(Error::invalid_entity(
                id.as_str(),
                "title must not be empty",
            ));
        }
        Ok(Self {
            id,
            kind,
            status,
            title,
            payload,
            evidence,
        })
    }
}

/// Directed, typed relationship between two entities (spec section 6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: EntityId,
    pub to: EntityId,
    pub kind: EdgeKind,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Owns,
    Implements,
    Calls,
    Imports,
    Tests,
    Documents,
    Constrains,
    Supersedes,
    DependsOn,
    Affects,
    Evidences,
    Validates,
    RiskOf,
}

/// Evidence a durable claim rests on (spec section 6). Spans are
/// references plus a content hash — never copied source or raw output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Evidence {
    GitCommit {
        sha: String,
        paths: Vec<PathBuf>,
    },
    SourceSpan {
        path: PathBuf,
        start_line: u32,
        end_line: u32,
        content_hash: String,
    },
    TestRun {
        command: String,
        status: String,
        at: OffsetDateTime,
        output_digest: String,
    },
    Document {
        path: PathBuf,
        content_hash: String,
    },
    UserApproval {
        actor: String,
        at: OffsetDateTime,
    },
}

/// A tracked unit of work (spec section 6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: crate::ids::TaskId,
    pub objective: String,
    pub status: TaskStatus,
    pub scope: Vec<EntityId>,
    pub validation: Vec<String>,
    pub handoff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    InProgress,
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub command: String,
    pub status: ValidationStatus,
}

impl ValidationResult {
    pub fn new(command: impl Into<String>, status: ValidationStatus) -> Self {
        Self {
            command: command.into(),
            status,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn entity_id(sequence: u64) -> EntityId {
        EntityId::new(format!(
            "ent_{}",
            super::super::ids::test_support::ulid_for(sequence)
        ))
        .expect("test id is well formed")
    }

    #[test]
    fn rejects_kind_payload_mismatch() {
        let result = Entity::new(
            entity_id(1),
            EntityKind::Decision,
            Status::Active,
            "a decision".into(),
            EntityPayload::Invariant {
                statement: "s".into(),
                consequence: "c".into(),
            },
            Vec::new(),
        );
        assert!(matches!(result, Err(Error::InvalidEntity { .. })));
    }

    #[test]
    fn accepts_matching_pairs_and_empty_evidence() {
        let entity = Entity::new(
            entity_id(2),
            EntityKind::Invariant,
            Status::Active,
            "leases stay unique".into(),
            EntityPayload::Invariant {
                statement: "one active owner per lease".into(),
                consequence: "duplicate processing".into(),
            },
            Vec::new(),
        );
        assert!(entity.is_ok());
    }

    #[test]
    fn task_round_trips_through_serde() {
        let task = Task {
            id: crate::ids::TaskId::new(format!(
                "task_{}",
                super::super::ids::test_support::ulid_for(5)
            ))
            .expect("test id"),
            objective: "implement lease renewal".into(),
            status: TaskStatus::InProgress,
            scope: vec![entity_id(3)],
            validation: vec!["cargo test -p frontier".into()],
            handoff: "none".into(),
        };
        let json = serde_json::to_string(&task).expect("task serializes");
        assert_eq!(
            task,
            serde_json::from_str(&json).expect("task deserializes")
        );
    }
}
