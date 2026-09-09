//! Structured durable-memory proposal intake; review and application are separate.

use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::{Args, ValueEnum};
use middleman_core::{
    Actor, Claim, EntityKind, Event, EventId, EventKind, Evidence, Hash, ProposalId, State, TaskId,
    proposal::{self, Draft, DraftClaim, Report},
    task,
};
use middleman_store::Store;
use serde::Deserialize;

const MAX_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Args)]
pub struct Options {
    /// Existing task that supplies scope and optional Git baseline context.
    #[arg(long)]
    task_id: String,
    /// JSON file containing decisions, invariants, and contracts.
    #[arg(long)]
    input: PathBuf,
    /// Attach the current bounded Git head and changed task paths as evidence.
    #[arg(long)]
    from_git: bool,
    /// Render the reviewable proposal as JSON or Markdown.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Json,
    Markdown,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    #[serde(default)]
    decisions: Vec<Claim>,
    #[serde(default)]
    invariants: Vec<Claim>,
    #[serde(default)]
    contracts: Vec<Claim>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("input is invalid, unreadable, or exceeds the proposal size limit")]
    InvalidInput,
    #[error("run middleman init first")]
    NotInitialized,
    #[error("current Git evidence is unavailable")]
    GitUnavailable,
    #[error("proposal validation failed")]
    InvalidDraft { report: Report },
    #[error("storage: {0}")]
    Store(#[from] middleman_store::Error),
    #[error("domain: {0}")]
    Core(#[from] middleman_core::Error),
    #[error("input JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("index input: {0}")]
    Index(#[from] super::retrieval::Error),
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput | Self::Json(_) => "invalid_input",
            Self::NotInitialized => "not_initialized",
            Self::GitUnavailable => "git_unavailable",
            Self::InvalidDraft { .. } => "invalid_proposal",
            Self::Store(_) => "storage_error",
            Self::Core(_) => "domain_error",
            Self::Index(error) => error.code(),
        }
    }

    pub const fn report(&self) -> Option<&Report> {
        match self {
            Self::InvalidDraft { report } => Some(report),
            _ => None,
        }
    }
}

pub fn run(repo: &Path, options: &Options) -> Result<(), Error> {
    let task_id = TaskId::new(&options.task_id).map_err(|_| Error::InvalidInput)?;
    let input = read_input(&options.input)?;
    let state_dir = repo.join(super::STATE_DIR);
    if !state_dir.join(super::STATE_DB).is_file() {
        return Err(Error::NotInitialized);
    }
    let store = Store::open_read_only(&state_dir)?;
    let state = middleman_core::project(&store.events()?)?;
    drop(store);
    let loaded = super::retrieval::task_input(repo, state, options.from_git)?;
    let task = loaded
        .state
        .tasks
        .get(&task_id)
        .ok_or(Error::InvalidInput)?;
    let mut draft = Draft {
        task_id: Some(task_id),
        claims: claims(input),
    };
    if options.from_git {
        attach_git_evidence(&mut draft, task, &loaded.snapshot)?;
    }
    let validation_state = validation_state(&loaded.state, &loaded.graph.entities);
    let report = proposal::validate(&draft, &validation_state);
    if !report.is_valid() {
        return Err(Error::InvalidDraft { report });
    }
    let id = ProposalId::new(format!("prop_{}", ulid::Ulid::generate()))?;
    let events = events(&draft, &loaded.state, &id)?;
    Store::open(&state_dir)?.append_batch(&events)?;
    render(options.format, &id, &draft, &report, options.from_git)
}

fn read_input(path: &Path) -> Result<Input, Error> {
    let metadata = fs::metadata(path).map_err(|_| Error::InvalidInput)?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES {
        return Err(Error::InvalidInput);
    }
    let bytes = fs::read(path).map_err(|_| Error::InvalidInput)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn claims(input: Input) -> Vec<DraftClaim> {
    input
        .decisions
        .into_iter()
        .map(|claim| DraftClaim {
            kind: EntityKind::Decision,
            claim,
        })
        .chain(input.invariants.into_iter().map(|claim| DraftClaim {
            kind: EntityKind::Invariant,
            claim,
        }))
        .chain(input.contracts.into_iter().map(|claim| DraftClaim {
            kind: EntityKind::Contract,
            claim,
        }))
        .collect()
}

fn attach_git_evidence(
    draft: &mut Draft,
    record: &middleman_core::TaskRecord,
    snapshot: &task::Snapshot,
) -> Result<(), Error> {
    let sha = snapshot.git_commit.clone().ok_or(Error::GitUnavailable)?;
    let changes = task::compare(record.baseline.as_ref(), snapshot);
    let paths = changes.added.into_iter().chain(changes.changed).collect();
    let evidence = Evidence::GitCommit { sha, paths };
    for draft_claim in &mut draft.claims {
        draft_claim.claim.evidence.push(evidence.clone());
    }
    Ok(())
}

fn validation_state(
    state: &State,
    entities: &std::collections::BTreeMap<middleman_core::EntityId, middleman_core::Entity>,
) -> State {
    let mut result = state.clone();
    result.entities.clone_from(entities);
    result
}

fn events(draft: &Draft, state: &State, proposal_id: &ProposalId) -> Result<Vec<Event>, Error> {
    let project_id = state.project_id.clone().ok_or(Error::NotInitialized)?;
    let mut sequence = state.last_sequence;
    let mut previous = state.last_hash.unwrap_or(Hash::genesis());
    let at = time::OffsetDateTime::now_utc();
    let task_id = draft.task_id.clone();
    draft
        .claims
        .iter()
        .map(|draft_claim| {
            sequence += 1;
            let kind = match draft_claim.kind {
                EntityKind::Decision => EventKind::DecisionProposed {
                    task_id: task_id.clone(),
                    claim: draft_claim.claim.clone(),
                },
                EntityKind::Invariant => EventKind::InvariantProposed {
                    task_id: task_id.clone(),
                    claim: draft_claim.claim.clone(),
                },
                EntityKind::Contract => EventKind::ContractProposed {
                    task_id: task_id.clone(),
                    claim: draft_claim.claim.clone(),
                },
                _ => unreachable!("core validation rejects unsupported proposal kinds"),
            };
            let event = Event::new(
                EventId::new(format!("evt_{}", ulid::Ulid::generate()))?,
                project_id.clone(),
                sequence,
                at,
                Actor::User,
                kind,
                vec![],
                Some(proposal_id.clone()),
                previous,
            )?;
            previous = event.hash;
            Ok(event)
        })
        .collect()
}

fn render(
    format: OutputFormat,
    id: &ProposalId,
    draft: &Draft,
    report: &Report,
    from_git: bool,
) -> Result<(), Error> {
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "proposal_id": id,
                "task_id": draft.task_id,
                "claims": draft.claims,
                "warnings": report.warnings,
                "affected_owners": report.affected_owners,
                "git_evidence_attached": from_git,
                "status": "pending_review"
            }))?
        ),
        OutputFormat::Markdown => print_markdown(id, draft, report, from_git),
    }
    Ok(())
}

fn print_markdown(id: &ProposalId, draft: &Draft, report: &Report, from_git: bool) {
    println!("# Proposal {id}");
    println!();
    println!(
        "Task: {}",
        draft.task_id.as_ref().map_or("-", TaskId::as_str)
    );
    println!("Status: pending review");
    println!("Git evidence attached: {from_git}");
    println!();
    println!("## Claims");
    for claim in &draft.claims {
        println!("- **{}**: {}", claim.kind.as_str(), claim.claim.statement);
    }
    if !report.warnings.is_empty() {
        println!();
        println!("## Review warnings");
        for warning in &report.warnings {
            println!("- {:?}", warning.code);
        }
    }
    if !report.affected_owners.is_empty() {
        println!();
        println!("## Affected owners");
        for owner in &report.affected_owners {
            println!("- {owner}");
        }
    }
}
