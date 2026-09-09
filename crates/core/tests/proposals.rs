#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use middleman_core::{
    Entity, EntityId, EntityKind, EntityPayload, Evidence, State, Status,
    entity::TaskStatus,
    proposal::{self, Code, Draft, DraftClaim},
};

fn entity(id: &str, kind: EntityKind, payload: EntityPayload) -> Entity {
    Entity::new(
        EntityId::derived(id, &["item"]).unwrap(),
        kind,
        Status::Active,
        "Lease record".into(),
        payload,
        vec![],
    )
    .unwrap()
}

fn state() -> (State, EntityId, EntityId) {
    let module = entity(
        "mod",
        EntityKind::Module,
        EntityPayload::Module {
            path: "src/lease.rs".into(),
            language: None,
            responsibility: String::new(),
            public_surface: vec![],
        },
    );
    let document = entity(
        "doc",
        EntityKind::Document,
        EntityPayload::Document {
            path: "docs/lease.md".into(),
            title: "Lease notes".into(),
            kind: "guide".into(),
            authority: "observed".into(),
        },
    );
    let state = State {
        entities: [
            (module.id.clone(), module.clone()),
            (document.id.clone(), document.clone()),
        ]
        .into_iter()
        .collect(),
        ..State::default()
    };
    (state, module.id, document.id)
}

fn draft(scope: Vec<EntityId>, evidence: Vec<Evidence>) -> Draft {
    Draft {
        task_id: None,
        claims: vec![DraftClaim {
            kind: EntityKind::Invariant,
            claim: middleman_core::Claim {
                label: "lease owner".into(),
                statement: "Only the active owner can renew".into(),
                rationale: Some("Protect exclusivity".into()),
                scope,
                evidence,
            },
        }],
    }
}

fn document() -> Evidence {
    Evidence::Document {
        path: "docs/lease.md".into(),
        content_hash: "a".repeat(64),
    }
}

#[test]
fn valid_draft_is_accepted_with_active_scope_and_evidence() {
    let (state, module, _) = state();
    let report = proposal::validate(&draft(vec![module], vec![document()]), &state);
    assert!(report.is_valid(), "{report:?}");
}

#[test]
fn validation_rejects_shape_scope_evidence_and_duplicate_drafts() {
    let (state, module, _) = state();
    let mut input = draft(vec![module.clone(), module], vec![]);
    input.claims.push(input.claims[0].clone());
    input.claims[0].claim.statement = "\n".into();
    input.claims[1].claim.statement = "\n".into();
    input.task_id =
        Some(middleman_core::TaskId::new(format!("task_{}", ulid::Ulid::from(1u128))).unwrap());
    let report = proposal::validate(&input, &state);
    let codes: Vec<_> = report.errors.iter().map(|issue| issue.code).collect();
    for code in [
        Code::MissingTask,
        Code::InvalidText,
        Code::MissingScope,
        Code::MissingEvidence,
        Code::DuplicateDraft,
    ] {
        assert!(codes.contains(&code), "missing {code:?}: {codes:?}");
    }
}

#[test]
fn inactive_and_unknown_references_are_never_accepted() {
    let (mut state, module, _) = state();
    state.entities.get_mut(&module).unwrap().status = Status::Superseded;
    let unknown = EntityId::derived("mod", &["unknown"]).unwrap();
    let input = draft(
        vec![module, unknown],
        vec![Evidence::SourceSpan {
            path: "../outside".into(),
            start_line: 1,
            end_line: 2,
            content_hash: "b".repeat(64),
        }],
    );
    let report = proposal::validate(&input, &state);
    let codes: Vec<_> = report.errors.iter().map(|issue| issue.code).collect();
    assert!(codes.contains(&Code::InactiveScope));
    assert!(codes.contains(&Code::MissingScope));
    assert!(codes.contains(&Code::UnknownEvidencePath));
}

#[test]
fn exact_active_fact_is_a_duplicate_and_task_scope_mismatch_is_a_warning() {
    let (mut state, module, document_id) = state();
    let existing = entity(
        "inv",
        EntityKind::Invariant,
        EntityPayload::Invariant {
            statement: "ONLY the active owner CAN renew".into(),
            consequence: String::new(),
        },
    );
    state.entities.insert(existing.id.clone(), existing);
    let task_id = middleman_core::TaskId::new(format!("task_{}", ulid::Ulid::from(2u128))).unwrap();
    state.tasks.insert(
        task_id.clone(),
        middleman_core::TaskRecord {
            id: task_id.clone(),
            objective: String::new(),
            status: TaskStatus::Completed,
            scope: vec![module],
            validation: vec![],
            validation_results: vec![],
            handoff: String::new(),
            summary: String::new(),
            started_at: time::OffsetDateTime::UNIX_EPOCH,
            finished_at: None,
            baseline: None,
            observations: None,
        },
    );
    let mut input = draft(vec![document_id], vec![document()]);
    input.task_id = Some(task_id);
    let report = proposal::validate(&input, &state);
    assert!(
        report
            .errors
            .iter()
            .any(|issue| issue.code == Code::DuplicateActiveFact)
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.code == Code::ScopeOutsideTask)
    );
}

#[test]
fn validator_is_pure_and_report_is_serde_stable() {
    let (state, module, _) = state();
    let input = draft(vec![module], vec![document()]);
    let first = proposal::validate(&input, &state);
    let second = proposal::validate(&input, &state);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::from_str::<proposal::Report>(&serde_json::to_string(&first).unwrap()).unwrap(),
        first
    );
    assert_eq!(
        state.entities.len(),
        BTreeMap::from_iter(state.entities.clone()).len()
    );
}
