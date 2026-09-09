//! Materialized state: the pure fold over the event log (spec section 6).
//!
//! `project` replays a verified event log into [`State`]. It is a pure
//! function: same log, same state, on every run and platform. Every
//! command in the system reads durable state through this fold — no
//! other state-change path exists.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::entity::{Edge, Entity, EntityKind, Status, TaskStatus, ValidationResult};
use crate::error::Error;
use crate::event::{Claim, Event, EventKind, RetrievalSignal};
use crate::ids::{EntityId, Hash, ProjectId, ProposalId, TaskId};

/// Materialized state of one project.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct State {
    pub project_id: Option<ProjectId>,
    pub project_name: String,
    pub last_indexed_commit: Option<String>,
    pub entities: BTreeMap<EntityId, Entity>,
    pub edges: Vec<Edge>,
    pub tasks: BTreeMap<TaskId, TaskRecord>,
    pub proposals: BTreeMap<ProposalId, ProposalRecord>,
    pub retrieval: Vec<RetrievalRecord>,
    pub last_sequence: u64,
    pub last_hash: Option<Hash>,
}

/// A unit of work as projected from start/complete/abandon events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: TaskId,
    pub objective: String,
    pub status: TaskStatus,
    pub scope: Vec<EntityId>,
    pub validation: Vec<String>,
    pub validation_results: Vec<ValidationResult>,
    pub handoff: String,
    pub summary: String,
    pub started_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

/// A durable-memory proposal under review, assembled from all
/// `*Proposed` events that share one `proposal_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub id: ProposalId,
    pub task_id: Option<TaskId>,
    pub claims: Vec<ProposedClaim>,
    pub created_at: OffsetDateTime,
    pub rejected: bool,
    pub rejection_reason: String,
}

/// One proposed fact; its kind is fixed by the `*Proposed` event that
/// created it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedClaim {
    pub kind: EntityKind,
    pub claim: Claim,
}

/// One recorded retrieval outcome, used to tune weights (spec 9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalRecord {
    pub task_id: Option<TaskId>,
    pub node_id: EntityId,
    pub signal: RetrievalSignal,
    pub at: OffsetDateTime,
}

/// Folds a verified event log into materialized state.
///
/// The log is hash-verified before any projection happens; a broken or
/// tampered log is rejected whole. Unknown references (e.g. completing
/// a task that was never started) are rejected with a typed error.
///
/// The fold stays one function on purpose: its whole contract is the
/// ordered, single-pass application of every event kind, and splitting
/// it would hide the invariants the hash chain guarantees. The
/// `EntityDeclared` and `*Accepted` arms share a body because, to the
/// projection, an accepted claim is exactly a declaration.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn project(events: &[Event]) -> Result<State, Error> {
    Event::verify_chain(events)?;
    let mut state = State::default();

    for event in events {
        match &event.kind {
            EventKind::PacketPrepared {
                selected, expanded, ..
            } => {
                for node_id in selected {
                    state.retrieval.push(RetrievalRecord {
                        task_id: None,
                        node_id: node_id.clone(),
                        signal: if *expanded {
                            RetrievalSignal::Expanded
                        } else {
                            RetrievalSignal::Retrieved
                        },
                        at: event.occurred_at,
                    });
                }
            }
            EventKind::ProjectInitialized { project_name } => {
                state.project_id = Some(event.project_id.clone());
                state.project_name.clone_from(project_name);
            }
            EventKind::SourceIndexed { git_commit, .. } => {
                if let Some(commit) = git_commit {
                    state.last_indexed_commit = Some(commit.clone());
                }
            }
            EventKind::EntityDeclared { entity } => {
                state.entities.insert(entity.id.clone(), entity.clone());
            }
            EventKind::EntitySuperseded { entity_id, .. } => {
                if let Some(entity) = state.entities.get_mut(entity_id) {
                    entity.status = Status::Superseded;
                }
            }
            EventKind::EdgeDeclared { edge } => {
                state.edges.push(edge.clone());
            }
            EventKind::TaskStarted { task } => {
                let record = TaskRecord {
                    id: task.id.clone(),
                    objective: task.objective.clone(),
                    status: TaskStatus::InProgress,
                    scope: task.scope.clone(),
                    validation: task.validation.clone(),
                    validation_results: Vec::new(),
                    handoff: task.handoff.clone(),
                    summary: String::new(),
                    started_at: event.occurred_at,
                    finished_at: None,
                };
                state.tasks.insert(task.id.clone(), record);
            }
            EventKind::TaskCompleted {
                task_id,
                summary,
                validation,
            } => {
                if let Some(task) = state.tasks.get_mut(task_id) {
                    task.status = TaskStatus::Completed;
                    task.summary.clone_from(summary);
                    task.validation_results.clone_from(validation);
                    task.finished_at = Some(event.occurred_at);
                } else {
                    return Err(Error::invalid_event(
                        event.sequence,
                        "TaskCompleted for a task that was never started",
                    ));
                }
            }
            EventKind::TaskAbandoned { task_id, reason } => {
                if let Some(task) = state.tasks.get_mut(task_id) {
                    task.status = TaskStatus::Abandoned;
                    task.summary.clone_from(reason);
                    task.finished_at = Some(event.occurred_at);
                } else {
                    return Err(Error::invalid_event(
                        event.sequence,
                        "TaskAbandoned for a task that was never started",
                    ));
                }
            }
            EventKind::DecisionProposed { task_id, claim } => {
                upsert_claim(
                    &mut state,
                    event,
                    EntityKind::Decision,
                    task_id.as_ref(),
                    claim,
                );
            }
            EventKind::InvariantProposed { task_id, claim } => {
                upsert_claim(
                    &mut state,
                    event,
                    EntityKind::Invariant,
                    task_id.as_ref(),
                    claim,
                );
            }
            EventKind::ContractProposed { task_id, claim } => {
                upsert_claim(
                    &mut state,
                    event,
                    EntityKind::Contract,
                    task_id.as_ref(),
                    claim,
                );
            }
            EventKind::DecisionAccepted { entity }
            | EventKind::InvariantAccepted { entity }
            | EventKind::ContractAccepted { entity } => {
                state.entities.insert(entity.id.clone(), entity.clone());
            }
            EventKind::EvidenceAttached {
                entity_id,
                evidence,
            } => {
                if let Some(entity) = state.entities.get_mut(entity_id) {
                    entity.evidence.push(evidence.clone());
                }
            }
            EventKind::RetrievalObserved {
                task_id,
                node_id,
                signal,
            } => {
                state.retrieval.push(RetrievalRecord {
                    task_id: task_id.clone(),
                    node_id: node_id.clone(),
                    signal: *signal,
                    at: event.occurred_at,
                });
            }
            EventKind::ProposalRejected { reason } => {
                if let Some(id) = &event.proposal_id {
                    if let Some(proposal) = state.proposals.get_mut(id) {
                        proposal.rejected = true;
                        proposal.rejection_reason.clone_from(reason);
                    }
                }
            }
        }
        state.last_sequence = event.sequence;
        state.last_hash = Some(event.hash);
    }

    Ok(state)
}

fn upsert_claim(
    state: &mut State,
    event: &Event,
    kind: EntityKind,
    task_id: Option<&TaskId>,
    claim: &Claim,
) {
    let Some(id) = &event.proposal_id else {
        return;
    };
    let exists = state.proposals.contains_key(id);
    let record = state
        .proposals
        .entry(id.clone())
        .or_insert_with(|| ProposalRecord {
            id: id.clone(),
            task_id: task_id.cloned(),
            claims: Vec::new(),
            created_at: event.occurred_at,
            rejected: false,
            rejection_reason: String::new(),
        });
    debug_assert!(!exists || record.created_at <= event.occurred_at);
    record.claims.push(ProposedClaim {
        kind,
        claim: claim.clone(),
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::entity::{
        Entity, EntityPayload, Evidence, Status, Task, TaskStatus, ValidationResult,
        ValidationStatus,
    };
    use crate::event::{Actor, Event};
    use crate::ids::{EventId, test_support};

    fn project_id() -> ProjectId {
        ProjectId::new(format!("proj_{}", test_support::ulid_for(1))).expect("id")
    }

    fn event_id(sequence: u64) -> EventId {
        EventId::new(format!("evt_{}", test_support::ulid_for(sequence))).expect("id")
    }

    fn entity_id(sequence: u64) -> EntityId {
        EntityId::new(format!("ent_{}", test_support::ulid_for(sequence))).expect("id")
    }

    fn task_id(sequence: u64) -> TaskId {
        TaskId::new(format!("task_{}", test_support::ulid_for(sequence))).expect("id")
    }

    fn proposal_id(sequence: u64) -> ProposalId {
        ProposalId::new(format!("prop_{}", test_support::ulid_for(sequence))).expect("id")
    }

    fn module_entity(id: EntityId, title: &str) -> Entity {
        Entity::new(
            id,
            EntityKind::Module,
            Status::Active,
            title.into(),
            EntityPayload::Module {
                path: "crates/demo".into(),
                language: Some("rust".into()),
                responsibility: "demos".into(),
                public_surface: Vec::new(),
            },
            Vec::new(),
        )
        .expect("entity")
    }

    fn claim(kind_label: &str) -> Claim {
        Claim {
            label: kind_label.into(),
            statement: format!("{kind_label} statement"),
            rationale: None,
            scope: Vec::new(),
            evidence: Vec::new(),
        }
    }

    /// Builds a chained log that exercises every event kind in order.
    fn full_log() -> Vec<Event> {
        let pid = project_id();
        let mut events: Vec<Event> = Vec::new();
        let push = |events: &mut Vec<Event>,
                    seq: u64,
                    actor: Actor,
                    kind: EventKind,
                    proposal: Option<ProposalId>| {
            let prev = events.last().map_or(Hash::genesis(), |e| e.hash);
            let event = Event::new(
                event_id(seq),
                pid.clone(),
                seq,
                OffsetDateTime::UNIX_EPOCH,
                actor,
                kind,
                Vec::new(),
                proposal,
                prev,
            )
            .expect("event");
            events.push(event);
        };

        let (m1, m2) = (entity_id(201), entity_id(202));
        let (d1, t1) = (entity_id(204), task_id(301));
        let (t2, (p1, p2)) = (task_id(302), (proposal_id(401), proposal_id(402)));

        push(
            &mut events,
            1,
            Actor::User,
            EventKind::ProjectInitialized {
                project_name: "demo".into(),
            },
            None,
        );
        push(
            &mut events,
            2,
            Actor::System,
            EventKind::SourceIndexed {
                git_commit: Some("abc123".into()),
                added: vec!["crates/demo/src/lib.rs".into()],
                changed: Vec::new(),
                removed: Vec::new(),
            },
            None,
        );
        push(
            &mut events,
            3,
            Actor::System,
            EventKind::EntityDeclared {
                entity: module_entity(m1.clone(), "demo module"),
            },
            None,
        );
        push(
            &mut events,
            4,
            Actor::System,
            EventKind::EntitySuperseded {
                entity_id: m1,
                replacement_id: m2.clone(),
                reason: "refactor".into(),
            },
            None,
        );
        push(
            &mut events,
            5,
            Actor::System,
            EventKind::EntityDeclared {
                entity: module_entity(m2.clone(), "demo module v2"),
            },
            None,
        );
        push(
            &mut events,
            6,
            Actor::System,
            EventKind::EdgeDeclared {
                edge: Edge {
                    from: m2.clone(),
                    to: d1.clone(),
                    kind: crate::entity::EdgeKind::Constrains,
                    evidence: Vec::new(),
                },
            },
            None,
        );
        push(
            &mut events,
            7,
            Actor::Harness("cline".into()),
            EventKind::TaskStarted {
                task: started_task(t1.clone(), "fix demo", vec![m2.clone()]),
            },
            None,
        );
        push(
            &mut events,
            8,
            Actor::System,
            EventKind::TaskCompleted {
                task_id: t1.clone(),
                summary: "fixed".into(),
                validation: vec![ValidationResult::new(
                    "cargo test",
                    ValidationStatus::Passed,
                )],
            },
            None,
        );
        push(
            &mut events,
            9,
            Actor::User,
            EventKind::DecisionProposed {
                task_id: Some(t1.clone()),
                claim: claim("decision"),
            },
            Some(p1.clone()),
        );
        push(
            &mut events,
            10,
            Actor::User,
            EventKind::DecisionAccepted {
                entity: decision_entity(d1.clone()),
            },
            Some(p1.clone()),
        );
        push(
            &mut events,
            11,
            Actor::User,
            EventKind::InvariantProposed {
                task_id: None,
                claim: claim("invariant"),
            },
            Some(p1),
        );
        push(
            &mut events,
            12,
            Actor::User,
            EventKind::ContractProposed {
                task_id: None,
                claim: claim("contract"),
            },
            Some(p2.clone()),
        );
        push(
            &mut events,
            13,
            Actor::User,
            EventKind::EvidenceAttached {
                entity_id: d1.clone(),
                evidence: Evidence::Document {
                    path: "docs/decisions.md".into(),
                    content_hash: "hash".into(),
                },
            },
            None,
        );
        push(
            &mut events,
            14,
            Actor::System,
            EventKind::RetrievalObserved {
                task_id: Some(t1),
                node_id: m2,
                signal: RetrievalSignal::Expanded,
            },
            None,
        );
        push(
            &mut events,
            15,
            Actor::Harness("cline".into()),
            EventKind::TaskStarted {
                task: started_task(t2.clone(), "other", Vec::new()),
            },
            None,
        );
        push(
            &mut events,
            16,
            Actor::User,
            EventKind::TaskAbandoned {
                task_id: t2,
                reason: "no longer needed".into(),
            },
            None,
        );
        push(
            &mut events,
            17,
            Actor::User,
            EventKind::ProposalRejected {
                reason: "already covered".into(),
            },
            Some(p2),
        );
        events
    }

    fn started_task(id: TaskId, objective: &str, scope: Vec<EntityId>) -> Task {
        Task {
            id,
            objective: objective.into(),
            status: TaskStatus::InProgress,
            scope,
            validation: vec!["cargo test".into()],
            handoff: String::new(),
        }
    }

    fn decision_entity(id: EntityId) -> Entity {
        Entity::new(
            id,
            EntityKind::Decision,
            Status::Active,
            "use blake3".into(),
            EntityPayload::Decision {
                statement: "hash with blake3".into(),
                rationale: Some("fast".into()),
                owner: None,
                supersedes: None,
            },
            Vec::new(),
        )
        .expect("entity")
    }

    #[test]
    fn empty_log_projects_to_default_state() {
        let state = project(&[]).expect("empty log projects");
        assert_eq!(state, State::default());
    }

    #[test]
    fn full_log_projects_every_event_kind() {
        let events = full_log();
        let state = project(&events).expect("log projects");

        assert_eq!(state.project_name, "demo");
        assert_eq!(state.last_indexed_commit.as_deref(), Some("abc123"));
        assert_eq!(state.last_sequence, 17);
        assert_eq!(state.last_hash, Some(events[16].hash));

        assert_eq!(
            state.entities.get(&entity_id(201)).expect("m1").status,
            Status::Superseded
        );
        assert_eq!(
            state.entities.get(&entity_id(202)).expect("m2").status,
            Status::Active
        );
        let decision = state.entities.get(&entity_id(204)).expect("decision");
        assert_eq!(decision.status, Status::Active);
        assert_eq!(decision.evidence.len(), 1);
        assert_eq!(state.edges.len(), 1);

        let first = state.tasks.get(&task_id(301)).expect("task one");
        assert_eq!(first.status, TaskStatus::Completed);
        assert_eq!(first.validation_results.len(), 1);
        let second = state.tasks.get(&task_id(302)).expect("task two");
        assert_eq!(second.status, TaskStatus::Abandoned);

        assert_eq!(state.proposals.len(), 2);
        let p1 = state
            .proposals
            .get(&proposal_id(401))
            .expect("proposal one");
        assert!(!p1.rejected);
        assert_eq!(p1.claims.len(), 2);
        assert_eq!(p1.claims[0].kind, EntityKind::Decision);
        assert_eq!(p1.claims[1].kind, EntityKind::Invariant);
        let p2 = state
            .proposals
            .get(&proposal_id(402))
            .expect("proposal two");
        assert!(p2.rejected);
        assert_eq!(p2.rejection_reason, "already covered");

        assert_eq!(state.retrieval.len(), 1);
        assert_eq!(state.retrieval[0].signal, RetrievalSignal::Expanded);
    }

    #[test]
    fn tampered_log_is_rejected_before_projection() {
        let mut events = full_log();
        events[1].occurred_at += time::Duration::minutes(1);
        assert!(project(&events).is_err());
    }

    #[test]
    fn completing_unknown_task_is_rejected() {
        let pid = project_id();
        let init = Event::new(
            event_id(1),
            pid.clone(),
            1,
            OffsetDateTime::UNIX_EPOCH,
            Actor::User,
            EventKind::ProjectInitialized {
                project_name: "demo".into(),
            },
            Vec::new(),
            None,
            Hash::genesis(),
        )
        .expect("init");
        let done = Event::new(
            event_id(2),
            pid,
            2,
            OffsetDateTime::UNIX_EPOCH,
            Actor::System,
            EventKind::TaskCompleted {
                task_id: task_id(301),
                summary: "x".into(),
                validation: Vec::new(),
            },
            Vec::new(),
            None,
            init.hash,
        )
        .expect("done");
        assert!(matches!(
            project(&[init, done]),
            Err(Error::InvalidEvent { sequence: 2, .. })
        ));
    }
}
