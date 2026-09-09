use clap::{Args, Subcommand};
use middleman_core::{
    Actor, EntityPayload, Event, EventId, EventKind, Hash, State, TaskId,
    entity::{Task, TaskStatus, ValidationResult, ValidationStatus},
    task::{self, Phase},
};
use middleman_store::Store;
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Args)]
pub struct Options {
    #[command(subcommand)]
    command: Command,
    #[arg(long, global = true, default_value = "json", value_parser = ["json"])]
    format: String,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Route transient task text and save a content-hash baseline.
    Start {
        #[arg(long)]
        task: String,
        /// Curated objective to store instead of the generated label.
        #[arg(long)]
        objective: Option<String>,
        /// Recommended command; never executed by Middleman.
        #[arg(long)]
        validation: Vec<String>,
    },
    /// Record endpoint observations and finish; completion does not imply tests passed.
    Finish {
        id: String,
        #[arg(long)]
        from_git: bool,
        /// Curated summary to store instead of the automatic observation summary.
        #[arg(long)]
        summary: Option<String>,
        /// Explicitly report a command as passed; Middleman does not execute it.
        #[arg(long)]
        passed: Vec<String>,
        #[arg(long)]
        failed: Vec<String>,
        #[arg(long)]
        skipped: Vec<String>,
    },
    /// Show the persisted task and its observations without scanning source.
    Show { id: String },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("input: {0}")]
    Input(#[from] super::retrieval::Error),
    #[error("storage: {0}")]
    Store(#[from] middleman_store::Error),
    #[error("domain: {0}")]
    Core(#[from] middleman_core::Error),
    #[error("invalid or oversized task input")]
    InvalidInput,
    #[error("task not found")]
    NotFound,
    #[error("task is already completed or abandoned")]
    Terminal,
    #[error("serialization failed")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Input(error) => error.code(),
            Self::Store(_) => "storage_error",
            Self::Core(_) => "domain_error",
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "task_not_found",
            Self::Terminal => "task_terminal",
            Self::Json(_) => "serialization_error",
        }
    }
}

pub fn run(repo: &Path, options: &Options) -> Result<(), Error> {
    let directory = repo.join(super::STATE_DIR);
    if !directory.join(super::STATE_DB).is_file() {
        return Err(super::retrieval::Error::NotInitialized.into());
    }
    let store = Store::open_read_only(&directory)?;
    let state = middleman_core::project(&store.events()?)?;
    drop(store);
    match &options.command {
        Command::Start {
            task,
            objective,
            validation,
        } => start(repo, &state, task, objective.as_deref(), validation),
        Command::Finish {
            id,
            from_git,
            summary,
            passed,
            failed,
            skipped,
        } => {
            let id = TaskId::new(id).map_err(|_| Error::InvalidInput)?;
            let record = state.tasks.get(&id).ok_or(Error::NotFound)?;
            if !matches!(record.status, TaskStatus::Open | TaskStatus::InProgress) {
                return Err(Error::Terminal);
            }
            if let Some(summary) = summary {
                text(summary, 4096)?;
            }
            let validation = results(passed, failed, skipped)?;
            if *from_git && !repo.join(".git").exists() {
                return Err(Error::InvalidInput);
            }
            let loaded = super::retrieval::task_input(repo, state.clone(), *from_git)?;
            let changes = task::compare(record.baseline.as_ref(), &loaded.snapshot);
            let summary = summary.clone().unwrap_or_else(|| task::summary(&changes));
            let mut kinds = vec![
                EventKind::TaskObserved {
                    task_id: id.clone(),
                    phase: Phase::Finish,
                    snapshot: loaded.snapshot.clone(),
                },
                EventKind::TaskCompleted {
                    task_id: id.clone(),
                    summary,
                    validation: validation.clone(),
                },
            ];
            kinds.extend(outcome_events(&loaded, record, &changes, &validation, &id));
            append(repo, &state, kinds, &id)
        }
        Command::Show { id } => {
            let id = TaskId::new(id).map_err(|_| Error::InvalidInput)?;
            let record = state.tasks.get(&id).ok_or(Error::NotFound)?;
            println!("{}", serde_json::to_string_pretty(record)?);
            Ok(())
        }
    }
}

fn outcome_events(
    loaded: &super::retrieval::Loaded,
    task: &middleman_core::TaskRecord,
    changes: &task::Changes,
    validation: &[ValidationResult],
    task_id: &TaskId,
) -> Vec<EventKind> {
    let changed_paths: BTreeSet<_> = changes
        .added
        .iter()
        .chain(&changes.changed)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();
    let passed = validation
        .iter()
        .any(|result| result.status == ValidationStatus::Passed);
    task.scope
        .iter()
        .filter_map(|id| {
            let entity = loaded.graph.entities.get(id)?;
            let path = middleman_indexer::routing::entity_path(&entity.payload)?;
            let overlaps = changed_paths.contains(&path.to_string_lossy().replace('\\', "/"));
            let test_overlap =
                entity.kind == middleman_core::EntityKind::Test && overlaps && passed;
            let mut events = Vec::new();
            if overlaps {
                events.push(EventKind::RetrievalObserved {
                    task_id: Some(task_id.clone()),
                    node_id: id.clone(),
                    signal: middleman_core::RetrievalSignal::FileOverlap,
                });
            }
            if test_overlap {
                events.push(EventKind::RetrievalObserved {
                    task_id: Some(task_id.clone()),
                    node_id: id.clone(),
                    signal: middleman_core::RetrievalSignal::TestOverlap,
                });
            }
            Some(events)
        })
        .flatten()
        .collect()
}

fn start(
    repo: &Path,
    state: &State,
    request: &str,
    objective: Option<&str>,
    validation: &[String],
) -> Result<(), Error> {
    text(request, 16_384)?;
    if let Some(objective) = objective {
        text(objective, 4096)?;
    }
    check_commands(validation)?;
    let loaded = super::retrieval::task_input(repo, state.clone(), true)?;
    let ranked = super::retrieval::rank(request, &loaded, 24);
    let scope: Vec<_> = ranked
        .candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect();
    let objective = objective.map_or_else(
        || {
            let ids = scope
                .iter()
                .take(3)
                .map(middleman_core::EntityId::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            if ids.is_empty() {
                "Repository task; context needs clarification".into()
            } else {
                format!("Work on {ids}")
            }
        },
        str::to_owned,
    );
    let mut commands: BTreeSet<_> = validation.iter().cloned().collect();
    for id in &scope {
        if let EntityPayload::Test { command, .. } = &loaded.graph.entities[id].payload {
            if !command.trim().is_empty() {
                commands.insert(command.clone());
            }
        }
    }
    let id = TaskId::new(format!("task_{}", ulid::Ulid::generate()))?;
    let task = Task {
        id: id.clone(),
        objective,
        status: TaskStatus::InProgress,
        scope: scope.clone(),
        validation: commands.into_iter().collect(),
        handoff: if ranked.low_confidence {
            "Routing confidence is low; confirm task scope before implementation.".into()
        } else {
            String::new()
        },
    };
    let mut kinds = vec![
        EventKind::TaskStarted { task },
        EventKind::TaskObserved {
            task_id: id.clone(),
            phase: Phase::Baseline,
            snapshot: loaded.snapshot,
        },
    ];
    kinds.extend(
        scope
            .into_iter()
            .map(|node_id| EventKind::RetrievalObserved {
                task_id: Some(id.clone()),
                node_id,
                signal: middleman_core::RetrievalSignal::Retrieved,
            }),
    );
    append(repo, state, kinds, &id)
}

fn append(repo: &Path, state: &State, kinds: Vec<EventKind>, id: &TaskId) -> Result<(), Error> {
    let project = state
        .project_id
        .clone()
        .ok_or(super::retrieval::Error::NotInitialized)?;
    let mut previous = state.last_hash.unwrap_or(Hash::genesis());
    let mut sequence = state.last_sequence;
    let at = time::OffsetDateTime::now_utc();
    let mut batch = Vec::new();
    for kind in kinds {
        sequence += 1;
        let event = Event::new(
            EventId::new(format!("evt_{}", ulid::Ulid::generate()))?,
            project.clone(),
            sequence,
            at,
            Actor::User,
            kind,
            vec![],
            None,
            previous,
        )?;
        previous = event.hash;
        batch.push(event);
    }
    let store = Store::open(&repo.join(super::STATE_DIR))?;
    store.append_batch(&batch)?;
    let state = store.state()?;
    let record = state.tasks.get(id).ok_or(Error::NotFound)?;
    // Baseline hashes remain available through task show, not in the compact command response.
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"id": record.id, "objective": record.objective,
        "status": record.status, "scope": record.scope, "summary": record.summary, "handoff": record.handoff, "observations": record.observations,
        "validation": record.validation, "validation_results": record.validation_results, "validation_source": "user_reported"})
        )?
    );
    Ok(())
}

fn text(value: &str, max: usize) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > max {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn check_commands(commands: &[String]) -> Result<(), Error> {
    if commands.len() > 64 {
        return Err(Error::InvalidInput);
    }
    for command in commands {
        text(command, 2048)?;
    }
    Ok(())
}

fn results(
    passed: &[String],
    failed: &[String],
    skipped: &[String],
) -> Result<Vec<ValidationResult>, Error> {
    let mut seen = BTreeSet::new();
    let mut results = Vec::new();
    for (commands, status) in [
        (passed, ValidationStatus::Passed),
        (failed, ValidationStatus::Failed),
        (skipped, ValidationStatus::Skipped),
    ] {
        check_commands(commands)?;
        for command in commands {
            if !seen.insert(command) {
                return Err(Error::InvalidInput);
            }
            results.push(ValidationResult::new(command, status));
        }
    }
    if results.len() > 64 {
        return Err(Error::InvalidInput);
    }
    results.sort_by(|a, b| a.command.cmp(&b.command));
    Ok(results)
}
