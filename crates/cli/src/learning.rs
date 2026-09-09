//! CLI boundary for reviewable local learning and optional-AI proposal intake.

use std::{fs, path::Path};

use clap::{Args, Subcommand, ValueEnum};
use middleman_core::{
    Actor, EntityId, Event, EventId, EventKind, Hash, OptionalAiMode, RetrievalSignal,
};
use middleman_store::Store;
use serde::{Deserialize, Serialize};

#[derive(Debug, Args)]
pub struct Options {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render deterministic, locally evidenced tuning and maintenance suggestions.
    Suggest,
    /// Apply the currently rendered weight suggestions to local configuration.
    Apply,
    /// Record explicit harness or user feedback for one previously retrieved node.
    Observe {
        #[arg(long)]
        node: String,
        #[arg(long, value_enum)]
        signal: Signal,
        #[arg(long)]
        task_id: Option<String>,
    },
    /// Validate an opt-in AI maintenance proposal without persisting it.
    Summarize {
        #[arg(long, value_enum)]
        source: AiSource,
        #[arg(long)]
        input: std::path::PathBuf,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Signal {
    Expanded,
    Referenced,
    FileOverlap,
    TestOverlap,
    UserAccepted,
    UserRejected,
}

impl From<Signal> for RetrievalSignal {
    fn from(value: Signal) -> Self {
        match value {
            Signal::Expanded => Self::Expanded,
            Signal::Referenced => Self::Referenced,
            Signal::FileOverlap => Self::FileOverlap,
            Signal::TestOverlap => Self::TestOverlap,
            Signal::UserAccepted => Self::UserAccepted,
            Signal::UserRejected => Self::UserRejected,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AiSource {
    Harness,
    Local,
    Remote,
}

impl AiSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

impl From<AiSource> for OptionalAiMode {
    fn from(value: AiSource) -> Self {
        match value {
            AiSource::Harness => Self::Harness,
            AiSource::Local => Self::Local,
            AiSource::Remote => Self::Remote,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiProposal {
    suggestions: Vec<AiSuggestion>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiSuggestion {
    title: String,
    rationale: String,
    #[serde(default)]
    node_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("run middleman init first")]
    NotInitialized,
    #[error("the learning input is invalid, missing, or exceeds its limit")]
    InvalidInput,
    #[error("optional AI is disabled or this source is not enabled in configuration")]
    OptionalAiDisabled,
    #[error("retrieval: {0}")]
    Retrieval(#[from] super::retrieval::Error),
    #[error("storage: {0}")]
    Store(#[from] middleman_store::Error),
    #[error("domain: {0}")]
    Core(#[from] middleman_core::Error),
    #[error("configuration could not be written")]
    Config,
    #[error("serialization failed")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::InvalidInput => "invalid_input",
            Self::OptionalAiDisabled => "optional_ai_disabled",
            Self::Retrieval(error) => error.code(),
            Self::Store(_) => "storage_error",
            Self::Core(_) => "domain_error",
            Self::Config => "config_error",
            Self::Json(_) => "serialization_error",
        }
    }
}

pub fn run(repo: &Path, options: &Options) -> Result<(), Error> {
    let directory = repo.join(super::STATE_DIR);
    if !directory.join(super::STATE_DB).is_file() {
        return Err(Error::NotInitialized);
    }
    let store = Store::open_read_only(&directory)?;
    let state = middleman_core::project(&store.events()?)?;
    drop(store);
    let loaded = super::retrieval::load(repo, state)?;
    match &options.command {
        Command::Suggest => print_report(&loaded),
        Command::Apply => apply(repo, &loaded),
        Command::Observe {
            node,
            signal,
            task_id,
        } => observe(repo, &loaded, node, (*signal).into(), task_id.as_deref()),
        Command::Summarize { source, input } => summarize(&loaded, *source, input),
    }
}

fn report(loaded: &super::retrieval::Loaded) -> middleman_core::learning::Report {
    middleman_core::learning::analyze(
        &loaded.state,
        &loaded.graph.entities,
        &loaded.graph.edges,
        loaded.config.weight,
    )
}

fn print_report(loaded: &super::retrieval::Loaded) -> Result<(), Error> {
    println!("{}", serde_json::to_string_pretty(&report(loaded))?);
    Ok(())
}

fn apply(repo: &Path, loaded: &super::retrieval::Loaded) -> Result<(), Error> {
    let report = report(loaded);
    let mut config = loaded.config.clone();
    for suggestion in &report.weight_suggestions {
        match suggestion.field.as_str() {
            "test_overlap" => config.weight.test_overlap = suggestion.proposed,
            "file_overlap" => config.weight.file_overlap = suggestion.proposed,
            "expanded" => config.weight.expanded = suggestion.proposed,
            "referenced" => config.weight.referenced = suggestion.proposed,
            "accepted" => config.weight.accepted = suggestion.proposed,
            "rejected" => config.weight.rejected = suggestion.proposed,
            _ => return Err(Error::Config),
        }
    }
    fs::write(
        repo.join(super::STATE_DIR).join(super::CONFIG_FILE),
        config.to_toml().map_err(|_| Error::Config)?,
    )
    .map_err(|_| Error::Config)?;
    println!(
        "{}",
        serde_json::json!({"applied": report.weight_suggestions, "review_required": false,
            "maintenance_suggestions": report.maintenance_suggestions.len()})
    );
    Ok(())
}

fn observe(
    repo: &Path,
    loaded: &super::retrieval::Loaded,
    node: &str,
    signal: RetrievalSignal,
    task_id: Option<&str>,
) -> Result<(), Error> {
    let node_id = EntityId::new(node).map_err(|_| Error::InvalidInput)?;
    if !loaded.graph.entities.contains_key(&node_id) {
        return Err(Error::InvalidInput);
    }
    let task_id = task_id
        .map(middleman_core::TaskId::new)
        .transpose()
        .map_err(|_| Error::InvalidInput)?;
    if task_id
        .as_ref()
        .is_some_and(|id| !loaded.state.tasks.contains_key(id))
    {
        return Err(Error::InvalidInput);
    }
    let event = Event::new(
        EventId::new(format!("evt_{}", ulid::Ulid::generate()))?,
        loaded
            .state
            .project_id
            .clone()
            .ok_or(Error::NotInitialized)?,
        loaded.state.last_sequence + 1,
        time::OffsetDateTime::now_utc(),
        Actor::User,
        EventKind::RetrievalObserved {
            task_id,
            node_id,
            signal,
        },
        vec![],
        None,
        loaded.state.last_hash.unwrap_or(Hash::genesis()),
    )?;
    Store::open(&repo.join(super::STATE_DIR))?.append(&event)?;
    println!("{}", serde_json::json!({"recorded": event.id}));
    Ok(())
}

fn summarize(
    loaded: &super::retrieval::Loaded,
    source: AiSource,
    path: &Path,
) -> Result<(), Error> {
    if !loaded.config.optional_ai.enabled || loaded.config.optional_ai.mode != source.into() {
        return Err(Error::OptionalAiDisabled);
    }
    let metadata = fs::metadata(path).map_err(|_| Error::InvalidInput)?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return Err(Error::InvalidInput);
    }
    let proposal: AiProposal =
        serde_json::from_slice(&fs::read(path).map_err(|_| Error::InvalidInput)?)
            .map_err(|_| Error::InvalidInput)?;
    if proposal.suggestions.len() > 32 || !valid_ai_suggestions(&proposal.suggestions, loaded) {
        return Err(Error::InvalidInput);
    }
    println!(
        "{}",
        serde_json::json!({"source": source.as_str(), "review_required": true,
            "suggestions": proposal.suggestions,
            "persistence": "none; submit reviewed durable claims through middleman propose"})
    );
    Ok(())
}

fn valid_ai_suggestions(suggestions: &[AiSuggestion], loaded: &super::retrieval::Loaded) -> bool {
    suggestions.iter().all(|suggestion| {
        !suggestion.title.trim().is_empty()
            && suggestion.title.len() <= 240
            && !suggestion.rationale.trim().is_empty()
            && suggestion.rationale.len() <= 4096
            && suggestion.node_ids.len() <= 16
            && suggestion.node_ids.iter().all(|raw| {
                EntityId::new(raw)
                    .ok()
                    .is_some_and(|id| loaded.graph.entities.contains_key(&id))
            })
    })
}
