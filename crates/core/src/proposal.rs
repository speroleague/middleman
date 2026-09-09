//! Pure validation of proposed durable memory; validation never applies a claim.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Entity, EntityId, EntityKind, EntityPayload, Evidence, State, Status, TaskId};

pub const MAX_CLAIMS: usize = 32;
const MAX_SCOPE: usize = 32;
const MAX_EVIDENCE: usize = 16;
const MAX_TEXT: usize = 4096;

/// Unapplied durable-memory edits submitted for review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub task_id: Option<TaskId>,
    pub claims: Vec<DraftClaim>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftClaim {
    pub kind: EntityKind,
    pub claim: crate::Claim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    EmptyDraft,
    UnsupportedKind,
    InvalidText,
    DuplicateDraft,
    MissingTask,
    MissingScope,
    InactiveScope,
    ScopeOutsideTask,
    MissingEvidence,
    InvalidEvidence,
    UnknownEvidencePath,
    DuplicateActiveFact,
    RelatedActiveFact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub code: Code,
    pub claim: Option<usize>,
    pub references: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub affected_owners: BTreeSet<String>,
}

impl Report {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Validates one draft against the current projected state.
///
/// The comparison is deliberately lexical: it rejects exact duplicate facts and
/// surfaces related active facts for review, but never claims to infer a natural-
/// language contradiction. Callers must require explicit review for warnings.
#[must_use]
pub fn validate(draft: &Draft, state: &State) -> Report {
    let mut report = Report {
        errors: vec![],
        warnings: vec![],
        affected_owners: BTreeSet::new(),
    };
    if draft.claims.is_empty() || draft.claims.len() > MAX_CLAIMS {
        report.errors.push(issue(Code::EmptyDraft, None, vec![]));
        return report;
    }
    let task_scope = draft.task_id.as_ref().and_then(|id| state.tasks.get(id));
    if draft.task_id.is_some() && task_scope.is_none() {
        report.errors.push(issue(Code::MissingTask, None, vec![]));
    }
    let known_paths = paths(&state.entities);
    let mut seen = BTreeSet::new();
    for (index, draft_claim) in draft.claims.iter().enumerate() {
        let claim = &draft_claim.claim;
        if !matches!(
            draft_claim.kind,
            EntityKind::Decision | EntityKind::Invariant | EntityKind::Contract
        ) {
            report
                .errors
                .push(issue(Code::UnsupportedKind, Some(index), vec![]));
        }
        if !text(&claim.label, 256)
            || !text(&claim.statement, MAX_TEXT)
            || claim
                .rationale
                .as_ref()
                .is_some_and(|value| !text(value, MAX_TEXT))
        {
            report
                .errors
                .push(issue(Code::InvalidText, Some(index), vec![]));
        }
        let key = (
            draft_claim.kind.as_str(),
            normalized(&claim.label),
            normalized(&claim.statement),
        );
        if !seen.insert(key) {
            report
                .errors
                .push(issue(Code::DuplicateDraft, Some(index), vec![]));
        }
        validate_scope(
            &mut report,
            index,
            claim.scope.as_slice(),
            state,
            task_scope.map(|task| &task.scope),
        );
        validate_evidence(&mut report, index, claim.evidence.as_slice(), &known_paths);
        validate_existing(&mut report, index, draft_claim, &state.entities);
        for id in &claim.scope {
            if let Some(entity) = state.entities.get(id) {
                if let Some(value) = owner(&entity.payload) {
                    report.affected_owners.insert(value);
                }
            }
        }
    }
    report
}

fn validate_scope(
    report: &mut Report,
    index: usize,
    scope: &[EntityId],
    state: &State,
    task_scope: Option<&Vec<EntityId>>,
) {
    if scope.len() > MAX_SCOPE || scope.iter().collect::<BTreeSet<_>>().len() != scope.len() {
        report
            .errors
            .push(issue(Code::MissingScope, Some(index), vec![]));
        return;
    }
    for id in scope {
        match state.entities.get(id) {
            None => report
                .errors
                .push(issue(Code::MissingScope, Some(index), vec![id.clone()])),
            Some(entity) if entity.status != Status::Active => {
                report
                    .errors
                    .push(issue(Code::InactiveScope, Some(index), vec![id.clone()]));
            }
            Some(_) if task_scope.is_some_and(|task_scope| !task_scope.contains(id)) => report
                .warnings
                .push(issue(Code::ScopeOutsideTask, Some(index), vec![id.clone()])),
            Some(_) => {}
        }
    }
}

fn validate_evidence(
    report: &mut Report,
    index: usize,
    evidence: &[Evidence],
    known_paths: &BTreeSet<String>,
) {
    if evidence.is_empty() || evidence.len() > MAX_EVIDENCE {
        report
            .errors
            .push(issue(Code::MissingEvidence, Some(index), vec![]));
        return;
    }
    for item in evidence {
        let valid = match item {
            Evidence::GitCommit { sha, paths } => {
                hash(sha)
                    && paths
                        .iter()
                        .all(|path| known_paths.contains(&path_key(path)))
            }
            Evidence::SourceSpan {
                path,
                start_line,
                end_line,
                content_hash,
            } => {
                *start_line > 0
                    && start_line <= end_line
                    && hash(content_hash)
                    && known_paths.contains(&path_key(path))
            }
            Evidence::TestRun {
                command,
                status,
                output_digest,
                ..
            } => {
                text(command, 2048)
                    && matches!(status.as_str(), "passed" | "failed" | "skipped")
                    && hash(output_digest)
            }
            Evidence::Document { path, content_hash } => {
                hash(content_hash) && known_paths.contains(&path_key(path))
            }
            Evidence::UserApproval { actor, .. } => text(actor, 256),
        };
        if !valid {
            let code = if missing_path(item, known_paths) {
                Code::UnknownEvidencePath
            } else {
                Code::InvalidEvidence
            };
            report.errors.push(issue(code, Some(index), vec![]));
        }
    }
}

fn missing_path(item: &Evidence, known_paths: &BTreeSet<String>) -> bool {
    match item {
        Evidence::GitCommit { paths, .. } => paths
            .iter()
            .any(|path| !known_paths.contains(&path_key(path))),
        Evidence::SourceSpan { path, .. } | Evidence::Document { path, .. } => {
            !known_paths.contains(&path_key(path))
        }
        Evidence::TestRun { .. } | Evidence::UserApproval { .. } => false,
    }
}

fn validate_existing(
    report: &mut Report,
    index: usize,
    draft: &DraftClaim,
    entities: &BTreeMap<EntityId, Entity>,
) {
    let statement = normalized(&draft.claim.statement);
    let related: Vec<_> = entities
        .values()
        .filter(|entity| entity.status == Status::Active && entity.kind == draft.kind)
        .filter(|entity| {
            active_text(&entity.payload).is_some_and(|text| normalized(text) == statement)
        })
        .map(|entity| entity.id.clone())
        .collect();
    if !related.is_empty() {
        report
            .errors
            .push(issue(Code::DuplicateActiveFact, Some(index), related));
    }
}

fn paths(entities: &BTreeMap<EntityId, Entity>) -> BTreeSet<String> {
    entities
        .values()
        .filter_map(|entity| match &entity.payload {
            EntityPayload::Module { path, .. }
            | EntityPayload::Symbol { path, .. }
            | EntityPayload::Test { path, .. }
            | EntityPayload::Document { path, .. } => Some(path),
            EntityPayload::Contract { path, .. } => path.as_ref(),
            _ => None,
        })
        .map(|path| path_key(path))
        .collect()
}

fn active_text(payload: &EntityPayload) -> Option<&str> {
    match payload {
        EntityPayload::Decision { statement, .. } | EntityPayload::Invariant { statement, .. } => {
            Some(statement)
        }
        EntityPayload::Contract { what, .. } => Some(what),
        _ => None,
    }
}

fn owner(payload: &EntityPayload) -> Option<String> {
    match payload {
        EntityPayload::Decision { owner, .. }
        | EntityPayload::Contract { owner, .. }
        | EntityPayload::Risk { owner, .. } => owner.clone(),
        _ => None,
    }
}

fn issue(code: Code, claim: Option<usize>, references: Vec<EntityId>) -> Issue {
    Issue {
        code,
        claim,
        references,
    }
}
fn path_key(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn hash(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
