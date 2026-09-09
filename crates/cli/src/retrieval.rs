use std::{collections::BTreeMap, path::Path};

use clap::{Args, Subcommand, ValueEnum};
use middleman_core::{
    Actor, Config, EntityId, EntityPayload, Event, EventId, EventKind, Hash, State, Status,
    entity::TaskStatus,
    routing::{self, Context, Hints, Ranking},
};
use middleman_indexer::{git, graph, scan};
use middleman_packet::{Format, Header, Limits};
use middleman_store::Store;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Cir,
    Markdown,
    Json,
}

impl From<OutputFormat> for Format {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Cir => Self::Cir,
            OutputFormat::Markdown => Self::Markdown,
            OutputFormat::Json => Self::Json,
        }
    }
}

#[derive(Debug, Args)]
pub struct OutputOptions {
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
    #[arg(long)]
    budget: Option<usize>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Prepare a budgeted context packet without retaining the task text.
    Prepare {
        #[arg(long)]
        task: String,
        #[command(flatten)]
        output: OutputOptions,
    },
    /// Expand a stable entity ID and its direct neighborhood.
    Expand {
        id: String,
        #[command(flatten)]
        output: OutputOptions,
    },
    /// Search current entities; returns JSON summaries and selection reasons.
    Search {
        text: String,
        #[arg(long = "type")]
        kind: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value = "json", value_parser = ["json"])]
        format: String,
    },
    /// Explain a historical packet event ID or current entity ID as JSON.
    Explain {
        id: String,
        #[arg(long, default_value = "json", value_parser = ["json"])]
        format: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("run middleman init first")]
    NotInitialized,
    #[error("request or option is invalid or exceeds its limit")]
    InvalidInput,
    #[error("the requested packet or entity was not found")]
    NotFound,
    #[error("broker configuration is invalid")]
    Config,
    #[error("storage: {0}")]
    Store(#[from] middleman_store::Error),
    #[error("domain: {0}")]
    Core(#[from] middleman_core::Error),
    #[error("scan: {0}")]
    Scan(#[from] scan::ScanError),
    #[error("graph: {0}")]
    Graph(#[from] graph::Error),
    #[error("git: {0}")]
    Git(#[from] git::Error),
    #[error("packet: {0}")]
    Packet(#[from] middleman_packet::Error),
    #[error("serialization failed")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::Config => "invalid_config",
            Self::Store(_) => "storage_error",
            Self::Core(_) => "domain_error",
            Self::Scan(_) | Self::Graph(_) | Self::Git(_) => "index_error",
            Self::Packet(_) => "packet_error",
            Self::Json(_) => "serialization_error",
        }
    }
}

struct Loaded {
    config: Config,
    state: State,
    graph: graph::Graph,
    hints: BTreeMap<EntityId, Hints>,
}

pub(super) fn guide_input(
    repo: &Path,
) -> Result<(String, BTreeMap<EntityId, middleman_core::Entity>), Error> {
    let state_dir = repo.join(super::STATE_DIR);
    if !state_dir.join(super::STATE_DB).is_file() {
        return Err(Error::NotInitialized);
    }
    let store = Store::open_read_only(&state_dir)?;
    let state = middleman_core::project(&store.events()?)?;
    if state.project_id.is_none() {
        return Err(Error::NotInitialized);
    }
    let loaded = load(repo, state)?;
    Ok((loaded.state.project_name, loaded.graph.entities))
}

pub fn run(repo: &Path, command: &Commands) -> Result<(), Error> {
    validate(command)?;
    let state_dir = repo.join(super::STATE_DIR);
    if !state_dir.join(super::STATE_DB).is_file() {
        return Err(Error::NotInitialized);
    }
    let store = Store::open_read_only(&state_dir)?;
    let events = store.events()?;
    if let Commands::Explain { id, .. } = command {
        if id.starts_with("evt_") {
            let id = EventId::new(id).map_err(|_| Error::InvalidInput)?;
            explain_packet(&events, &id)?;
            return Ok(());
        }
    }
    let state = middleman_core::project(&events)?;
    if state.project_id.is_none() {
        return Err(Error::NotInitialized);
    }
    drop(store);
    let loaded = load(repo, state)?;
    match command {
        Commands::Prepare { task, output } => {
            let ranking = rank(task, &loaded, 128);
            emit(repo, &loaded, task, &ranking, output, false)
        }
        Commands::Expand { id, output } => {
            let id = EntityId::new(id).map_err(|_| Error::InvalidInput)?;
            let entity = loaded
                .graph
                .entities
                .get(&id)
                .filter(|entity| entity.status == Status::Active)
                .ok_or(Error::NotFound)?;
            let ranking = routing::expand(
                &id,
                &Context {
                    entities: &loaded.graph.entities,
                    edges: &loaded.graph.edges,
                    hints: &loaded.hints,
                    cochanges: &[],
                },
            );
            emit(
                repo,
                &loaded,
                &format!("Expand {}", entity.id),
                &ranking,
                output,
                true,
            )
        }
        Commands::Search {
            text, kind, limit, ..
        } => search(&loaded, text, kind.as_deref(), *limit),
        Commands::Explain { id, .. } => {
            let id = EntityId::new(id).map_err(|_| Error::InvalidInput)?;
            let entity = loaded
                .graph
                .entities
                .get(&id)
                .filter(|entity| entity.status == Status::Active)
                .ok_or(Error::NotFound)?;
            let edges: Vec<_> = loaded
                .graph
                .edges
                .iter()
                .filter(|edge| edge.from == id || edge.to == id)
                .take(64)
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"entity": entity, "edges": edges,
                        "edges_truncated": loaded.graph.edges.iter().filter(|edge| edge.from == id || edge.to == id).count() > edges.len(),
                        "scope": "current evidence; use a packet ID for historical scores"})
                )?
            );
            Ok(())
        }
    }
}

fn explain_packet(events: &[Event], id: &EventId) -> Result<(), Error> {
    let event = events
        .iter()
        .find(|event| event.id == *id && matches!(event.kind, EventKind::PacketPrepared { .. }))
        .ok_or(Error::NotFound)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"packet_id": event.id, "record": event.kind})
        )?
    );
    Ok(())
}

fn validate(command: &Commands) -> Result<(), Error> {
    let text = match command {
        Commands::Prepare { task, output } => {
            check_output(output)?;
            task
        }
        Commands::Expand { id, output } => {
            check_output(output)?;
            id
        }
        Commands::Search {
            text, kind, limit, ..
        } => {
            if *limit > 128
                || kind.as_ref().is_some_and(|kind| {
                    ![
                        "module",
                        "symbol",
                        "test",
                        "document",
                        "decision",
                        "invariant",
                        "contract",
                        "risk",
                        "open_question",
                        "operation",
                    ]
                    .contains(&kind.as_str())
                })
            {
                return Err(Error::InvalidInput);
            }
            text
        }
        Commands::Explain { id, .. } => id,
    };
    if text.trim().is_empty() || text.len() > 16_384 {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn check_output(output: &OutputOptions) -> Result<(), Error> {
    if output.budget.is_some_and(|budget| budget > 100_000) {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn load(repo: &Path, state: State) -> Result<Loaded, Error> {
    let config = super::read_toml_config(&repo.join(super::STATE_DIR).join(super::CONFIG_FILE))
        .map_err(|_| Error::Config)?;
    let report = scan::scan(repo, &config)?;
    let history = if repo.join(".git").exists() {
        let allowed = report.files.iter().map(|file| file.path.clone()).collect();
        Some(git::scan(repo, &allowed, git::Limits::default())?)
    } else {
        None
    };
    let refreshed = graph::refresh(
        None,
        &report.files,
        history.as_ref(),
        &config.limits,
        graph::Budget::default(),
    )?;
    let mut graph = refreshed.index.graph().clone();
    let mut hints =
        middleman_indexer::routing::hints(&report.files, &graph, history.as_ref(), &config.limits);
    for (id, entity) in &state.entities {
        // File observations always come from the current scan, not old durable snapshots.
        if matches!(
            entity.payload,
            EntityPayload::Module { .. }
                | EntityPayload::Symbol { .. }
                | EntityPayload::Test { .. }
                | EntityPayload::Document { .. }
        ) {
            continue;
        }
        hints
            .entry(id.clone())
            .or_default()
            .routing_phrases
            .push(entity.title.clone());
        graph.entities.insert(id.clone(), entity.clone());
    }
    graph.edges.extend(
        state
            .edges
            .iter()
            .filter(|edge| {
                graph.entities.contains_key(&edge.from) && graph.entities.contains_key(&edge.to)
            })
            .cloned(),
    );
    for task in state
        .tasks
        .values()
        .filter(|task| matches!(task.status, TaskStatus::Open | TaskStatus::InProgress))
    {
        for id in &task.scope {
            hints.entry(id.clone()).or_default().recent_task = true;
        }
    }
    Ok(Loaded {
        config,
        state,
        graph,
        hints,
    })
}

fn rank(task: &str, loaded: &Loaded, limit: usize) -> Ranking {
    let cochanges: Vec<_> = loaded
        .graph
        .cochanges
        .iter()
        .map(|pair| (pair.left.clone(), pair.right.clone()))
        .collect();
    routing::rank(
        task,
        None,
        &Context {
            entities: &loaded.graph.entities,
            edges: &loaded.graph.edges,
            hints: &loaded.hints,
            cochanges: &cochanges,
        },
        limit,
    )
}

fn emit(
    repo: &Path,
    loaded: &Loaded,
    goal: &str,
    ranking: &Ranking,
    options: &OutputOptions,
    expanded: bool,
) -> Result<(), Error> {
    let format = options.format.map_or_else(
        || match loaded.config.output.format {
            middleman_core::config::OutputFormat::Cir => Format::Cir,
            middleman_core::config::OutputFormat::Markdown => Format::Markdown,
            middleman_core::config::OutputFormat::Json => Format::Json,
        },
        Format::from,
    );
    let budget = options
        .budget
        .unwrap_or(loaded.config.output.token_budget as usize);
    if budget > 100_000 {
        return Err(Error::InvalidInput);
    }
    let header = Header {
        project: loaded.state.project_name.clone(),
        task: None,
        goal: goal.to_owned(),
        low_confidence: false,
        omitted: 0,
        upstream_truncated: false,
    };
    let constraints = loaded
        .graph
        .entities
        .values()
        .filter(|entity| entity.status == Status::Active)
        .filter_map(|entity| {
            if let EntityPayload::Invariant {
                statement,
                consequence,
            } = &entity.payload
            {
                Some(format!("{statement}: {consequence}"))
            } else {
                None
            }
        })
        .collect();
    let output = middleman_packet::prepare(
        header,
        constraints,
        ranking,
        &loaded.graph.entities,
        format,
        Limits {
            budget,
            ..Limits::for_format(format)
        },
    )?;
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
        EventKind::PacketPrepared {
            candidates: ranking.candidates.clone(),
            selected: output
                .packet
                .items
                .iter()
                .map(|item| item.id.clone())
                .collect(),
            format: match format {
                Format::Cir => middleman_core::config::OutputFormat::Cir,
                Format::Markdown => middleman_core::config::OutputFormat::Markdown,
                Format::Json => middleman_core::config::OutputFormat::Json,
            },
            budget,
            estimated_tokens: output.estimated_tokens,
            ranking_truncated: ranking.truncated,
            low_confidence: output.packet.header.low_confidence,
            expanded,
        },
        vec![],
        None,
        loaded.state.last_hash.unwrap_or(Hash::genesis()),
    )?;
    Store::open(&repo.join(super::STATE_DIR))?.append(&event)?;
    eprintln!(
        "{}",
        serde_json::json!({"packet_id": event.id, "estimated_tokens": output.estimated_tokens,
            "history_truncated": loaded.graph.history_truncated, "unresolved_references": loaded.graph.diagnostics.len()})
    );
    print!("{}", output.text);
    Ok(())
}

fn search(loaded: &Loaded, text: &str, kind: Option<&str>, limit: usize) -> Result<(), Error> {
    let mut ranking = rank(text, loaded, loaded.graph.entities.len());
    ranking.candidates.retain(|candidate| {
        kind.is_none_or(|kind| loaded.graph.entities[&candidate.id].kind.as_str() == kind)
    });
    ranking.truncated |= ranking.candidates.len() > limit;
    ranking.candidates.truncate(limit);
    ranking.low_confidence = !ranking.candidates.iter().any(|candidate| {
        candidate.signals.iter().any(|signal| {
            signal.weight() >= 70 && signal.weight() - i32::from(candidate.stale_penalty) >= 55
        })
    });
    let items: Vec<_> = ranking.candidates.iter().map(|candidate| {
                let entity = &loaded.graph.entities[&candidate.id];
                serde_json::json!({"id": entity.id, "kind": entity.kind, "title": entity.title,
                    "path": middleman_indexer::routing::entity_path(&entity.payload), "score": candidate.score, "reasons": candidate.signals})
            }).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"items": items, "truncated": ranking.truncated, "low_confidence": ranking.low_confidence})
        )?
    );
    Ok(())
}
