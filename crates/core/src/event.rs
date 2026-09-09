//! The immutable event log model (spec section 6).
//!
//! Every durable mutation is an append-only [`Event`]. Events carry a
//! blake3 hash chaining each event to its predecessor, so any
//! corruption or tampering is detectable by replaying the log.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::Error;
use crate::ids::{EntityId, EventId, Hash, ProjectId, ProposalId, TaskId};

/// Who caused the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    Harness(String),
    System,
}

/// Outcome signals used for learning-based weight tuning (spec section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSignal {
    Retrieved,
    Expanded,
    Referenced,
    FileOverlap,
    TestOverlap,
    UserAccepted,
    UserRejected,
}

/// A durable-memory change as proposed, before review (spec section 10).
/// The accepted form is a fully typed `Entity` in the `*Accepted` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub label: String,
    pub statement: String,
    pub rationale: Option<String>,
    pub scope: Vec<EntityId>,
    pub evidence: Vec<crate::entity::Evidence>,
    #[serde(default)]
    pub details: Option<ClaimDetails>,
}

/// Kind-specific fields retained until a reviewed claim becomes an entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaimDetails {
    Decision {
        owner: Option<String>,
        supersedes: Option<EntityId>,
    },
    Invariant {
        consequence: String,
    },
    Contract {
        input: String,
        output: String,
        compatibility: Option<String>,
        owner: Option<String>,
        path: Option<std::path::PathBuf>,
    },
}

/// One append-only log entry. The hash covers every other field plus the
/// predecessor's hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub project_id: ProjectId,
    pub sequence: u64,
    pub occurred_at: OffsetDateTime,
    pub actor: Actor,
    pub kind: EventKind,
    pub evidence: Vec<crate::entity::Evidence>,
    pub proposal_id: Option<ProposalId>,
    pub previous_hash: Hash,
    pub hash: Hash,
}

/// All durable mutations (spec section 6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    PacketPrepared {
        candidates: Vec<crate::routing::Candidate>,
        selected: Vec<EntityId>,
        format: crate::config::OutputFormat,
        budget: usize,
        estimated_tokens: usize,
        ranking_truncated: bool,
        low_confidence: bool,
        expanded: bool,
    },
    ProjectInitialized {
        project_name: String,
    },
    SourceIndexed {
        git_commit: Option<String>,
        added: Vec<PathBuf>,
        changed: Vec<PathBuf>,
        removed: Vec<PathBuf>,
    },
    EntityDeclared {
        entity: crate::entity::Entity,
    },
    EntitySuperseded {
        entity_id: EntityId,
        replacement_id: EntityId,
        reason: String,
    },
    EdgeDeclared {
        edge: crate::entity::Edge,
    },
    TaskStarted {
        task: crate::entity::Task,
    },
    TaskObserved {
        task_id: TaskId,
        phase: crate::task::Phase,
        snapshot: crate::task::Snapshot,
    },
    TaskCompleted {
        task_id: TaskId,
        summary: String,
        #[serde(default)]
        validation: Vec<crate::entity::ValidationResult>,
    },
    TaskAbandoned {
        task_id: TaskId,
        reason: String,
    },
    DecisionProposed {
        task_id: Option<TaskId>,
        claim: Claim,
    },
    DecisionAccepted {
        entity: crate::entity::Entity,
    },
    InvariantProposed {
        task_id: Option<TaskId>,
        claim: Claim,
    },
    InvariantAccepted {
        entity: crate::entity::Entity,
    },
    ContractProposed {
        task_id: Option<TaskId>,
        claim: Claim,
    },
    ContractAccepted {
        entity: crate::entity::Entity,
    },
    EvidenceAttached {
        entity_id: EntityId,
        evidence: crate::entity::Evidence,
    },
    RetrievalObserved {
        task_id: Option<TaskId>,
        node_id: EntityId,
        signal: RetrievalSignal,
    },
    ProposalRejected {
        reason: String,
    },
}

impl Event {
    /// Builds a sealed event: then computes the hash over the canonical
    /// form of every other field plus the predecessor's hash.
    pub fn new(
        id: EventId,
        project_id: ProjectId,
        sequence: u64,
        occurred_at: OffsetDateTime,
        actor: Actor,
        kind: EventKind,
        evidence: Vec<crate::entity::Evidence>,
        proposal_id: Option<ProposalId>,
        previous_hash: Hash,
    ) -> Result<Self, Error> {
        let payload = canonical_bytes(
            id.as_str(),
            project_id.as_str(),
            sequence,
            occurred_at,
            actor.clone(),
            kind.clone(),
            evidence.clone(),
            proposal_id.clone(),
            previous_hash,
        )
        .map_err(|e| Error::InvalidEvent {
            sequence,
            reason: e.to_string(),
        })?;

        let hash = hash_payload_and_predecessor(&payload, previous_hash);
        Ok(Self {
            id,
            project_id,
            sequence,
            occurred_at,
            actor,
            kind,
            evidence,
            proposal_id,
            previous_hash,
            hash,
        })
    }

    /// Recomputes this event's hash from its fields.
    pub fn verify_hash(&self) -> bool {
        let Ok(bytes) = canonical_bytes(
            self.id.as_str(),
            self.project_id.as_str(),
            self.sequence,
            self.occurred_at,
            self.actor.clone(),
            self.kind.clone(),
            self.evidence.clone(),
            self.proposal_id.clone(),
            self.previous_hash,
        ) else {
            return false;
        };
        hash_payload_and_predecessor(&bytes, self.previous_hash) == self.hash
    }

    /// Verifies the whole log: initialization first, contiguous
    /// sequences, an unbroken hash chain, and a valid per-event hash.
    pub fn verify_chain(events: &[Event]) -> Result<(), Error> {
        let Some(first) = events.first() else {
            return Ok(());
        };
        if !matches!(first.kind, EventKind::ProjectInitialized { .. }) {
            return Err(Error::FirstEventNotInitialization(kind_tag(&first.kind)));
        }
        if first.previous_hash != Hash::genesis() {
            return Err(Error::invalid_event(
                first.sequence,
                "genesis must chain from the genesis hash",
            ));
        }

        for (index, event) in events.iter().enumerate() {
            let expected = index as u64 + 1;
            if event.sequence != expected {
                return Err(Error::SequenceMismatch {
                    expected,
                    found: event.sequence,
                });
            }
            if index > 0 && event.previous_hash != events[index - 1].hash {
                return Err(Error::HashBroken(event.sequence));
            }
            if !event.verify_hash() {
                return Err(Error::HashBroken(event.sequence));
            }
        }
        Ok(())
    }
}

/// Stable tag for a kind, used in error messages.
fn kind_tag(kind: &EventKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| {
            v.get("kind")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Canonical serialization form; struct field order is the hash's field order.
#[derive(Serialize)]
struct CanonicalEvent {
    id: String,
    project_id: String,
    sequence: u64,
    occurred_at: OffsetDateTime,
    actor: Actor,
    kind: EventKind,
    evidence: Vec<crate::entity::Evidence>,
    proposal_id: Option<ProposalId>,
    previous_hash: Hash,
}

fn canonical_bytes(
    id: &str,
    project_id: &str,
    sequence: u64,
    occurred_at: OffsetDateTime,
    actor: Actor,
    kind: EventKind,
    evidence: Vec<crate::entity::Evidence>,
    proposal_id: Option<ProposalId>,
    previous_hash: Hash,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&CanonicalEvent {
        id: id.to_owned(),
        project_id: project_id.to_owned(),
        sequence,
        occurred_at,
        actor,
        kind,
        evidence,
        proposal_id,
        previous_hash,
    })
}

fn hash_payload_and_predecessor(payload: &[u8], previous: Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(payload);
    hasher.update(previous.as_bytes());
    Hash::of_finalized(&hasher)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::entity::{Task, TaskStatus};
    use crate::ids::test_support;

    fn project_id() -> ProjectId {
        ProjectId::new(format!("proj_{}", test_support::ulid_for(1))).expect("project id")
    }

    fn event_id(sequence: u64) -> EventId {
        EventId::new(format!("evt_{}", test_support::ulid_for(sequence))).expect("event id")
    }

    fn task_id() -> TaskId {
        TaskId::new(format!("task_{}", test_support::ulid_for(11))).expect("task id")
    }

    fn started_task(task_id: TaskId, objective: &str) -> Task {
        Task {
            id: task_id,
            objective: objective.into(),
            status: TaskStatus::InProgress,
            scope: Vec::new(),
            validation: Vec::new(),
            handoff: String::new(),
        }
    }

    fn init_event(sequence: u64) -> Event {
        Event::new(
            event_id(sequence),
            project_id(),
            sequence,
            OffsetDateTime::UNIX_EPOCH,
            Actor::User,
            EventKind::ProjectInitialized {
                project_name: "fixture".into(),
            },
            Vec::new(),
            None,
            Hash::genesis(),
        )
        .expect("event seals")
    }

    fn next_event(sequence: u64, previous: Hash, kind: EventKind) -> Event {
        Event::new(
            event_id(sequence),
            project_id(),
            sequence,
            OffsetDateTime::UNIX_EPOCH,
            Actor::System,
            kind,
            Vec::new(),
            None,
            previous,
        )
        .expect("event seals")
    }

    #[test]
    fn a_chain_of_tasks_verifies() {
        let init = init_event(1);
        let task = started_task(task_id(), "renew leases");
        let started = next_event(2, init.hash, EventKind::TaskStarted { task: task.clone() });
        let finished = next_event(
            3,
            started.hash,
            EventKind::TaskCompleted {
                task_id: task.id,
                summary: "done".into(),
                validation: Vec::new(),
            },
        );

        let log = vec![init, started, finished];
        assert!(Event::verify_chain(&log).is_ok());
        assert_eq!(log[1].previous_hash, log[0].hash);
        assert_eq!(log[2].previous_hash, log[1].hash);
    }

    #[test]
    fn tampered_payload_breaks_the_chain() {
        let init = init_event(1);
        let mut started = next_event(
            2,
            init.hash,
            EventKind::TaskStarted {
                task: started_task(task_id(), "renew leases"),
            },
        );
        started.occurred_at += time::Duration::minutes(1);

        assert!(!started.verify_hash());
        let error = Event::verify_chain(&[init, started]).expect_err("tamper must be detected");
        assert!(matches!(error, Error::HashBroken(2)));
    }

    #[test]
    fn sequence_gaps_and_bad_first_events_are_rejected() {
        let init = init_event(1);
        let skipped = next_event(
            3,
            init.hash,
            EventKind::TaskCompleted {
                task_id: task_id(),
                summary: "x".into(),
                validation: Vec::new(),
            },
        );
        assert!(matches!(
            Event::verify_chain(&[init.clone(), skipped]),
            Err(Error::SequenceMismatch {
                expected: 2,
                found: 3
            })
        ));

        let mut not_init = init.clone();
        not_init.kind = EventKind::TaskCompleted {
            task_id: task_id(),
            summary: "x".into(),
            validation: Vec::new(),
        };
        assert!(matches!(
            Event::verify_chain(&[not_init]),
            Err(Error::FirstEventNotInitialization(tag)) if tag == "task_completed"
        ));
    }

    #[test]
    fn proposed_event_carries_its_proposal_id() {
        let init = init_event(1);
        let proposal =
            ProposalId::new(format!("prop_{}", test_support::ulid_for(21))).expect("proposal id");
        let event = Event::new(
            event_id(2),
            project_id(),
            2,
            OffsetDateTime::UNIX_EPOCH,
            Actor::User,
            EventKind::DecisionProposed {
                task_id: None,
                claim: Claim {
                    label: "pg clock authoritative".into(),
                    statement: "PostgreSQL time is authoritative for leases".into(),
                    rationale: None,
                    scope: Vec::new(),
                    evidence: Vec::new(),
                    details: None,
                },
            },
            Vec::new(),
            Some(proposal),
            init.hash,
        )
        .expect("event seals");

        assert!(event.proposal_id.is_some());
        assert!(Event::verify_chain(&[init, event]).is_ok());
    }
}
