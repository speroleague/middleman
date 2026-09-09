//! The `middleman` command-line surface.
//!
//! Thin over the core/packet/store/indexer libraries: parse arguments,
//! load or write files, call operations, print or render results.
//! Business rules and state transitions live in `middleman-core`, not
//! here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use thiserror::Error;

use middleman_core::entity::TaskStatus;
use middleman_core::{Config, Event, EventId, Hash, ProjectId, ProposalId, TaskId, TaskRecord};

mod retrieval;

const STATE_DIR: &str = ".middleman";
const CONFIG_FILE: &str = "middleman.toml";
const STATE_DB: &str = "context.sqlite3";
const STATE_GITIGNORE: &str = "# Ignore generated broker state; keep the config and this file\n*\n!middleman.toml\n!.gitignore\n";

#[derive(Debug, Parser)]
#[command(
    name = "middleman",
    version,
    about = "Deterministic context broker for coding agents"
)]
struct Cli {
    /// Repository root (defaults to the working directory).
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(flatten)]
    Retrieval(retrieval::Commands),
    /// Initialize `.middleman/` in a repository.
    Init {
        /// Project name (defaults to the repository directory's name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Summarize the broker state for a repository.
    Status,
    /// Diagnose the environment and broker state.
    Doctor,
}

#[derive(Debug, Error)]
enum Failure {
    #[error("retrieval: {0}")]
    Retrieval(#[from] retrieval::Error),
    #[error("`{0}` is not a directory")]
    NotADirectory(PathBuf),
    #[error("`.middleman` already exists in `{0}`: run `middleman doctor` to inspect it")]
    AlreadyInitialized(PathBuf),
    #[error("`.middleman` not found in `{0}`: run `middleman init` first")]
    NotInitialized(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("store: {0}")]
    Store(#[from] middleman_store::Error),
    #[error("config: {0}")]
    Config(String),
    #[error("core: {0}")]
    Core(#[from] middleman_core::Error),
    #[error("git: {0}")]
    Git(#[from] middleman_indexer::git::Error),
    #[error("doctor: {0} of {1} checks failed")]
    DoctorChecks(usize, usize),
}

fn run(cli: &Cli) -> Result<(), Failure> {
    let repo = cli.repo.as_path();
    if !repo.is_dir() {
        return Err(Failure::NotADirectory(cli.repo.clone()));
    }
    match &cli.command {
        Commands::Retrieval(command) => retrieval::run(repo, command).map_err(Failure::from),
        Commands::Init { name } => init(repo, name.as_deref()),
        Commands::Status => status(repo),
        Commands::Doctor => doctor(repo),
    }
}

fn init(repo: &Path, name: Option<&str>) -> Result<(), Failure> {
    let state_dir = repo.join(STATE_DIR);
    let name = name
        .filter(|n| !n.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| repo.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .unwrap_or_else(|| "project".to_owned());

    let config = Config {
        project_name: name,
        ..Config::default()
    };
    let config_toml = config
        .to_toml()
        .map_err(|e| Failure::Config(e.to_string()))?;

    std::fs::create_dir_all(&state_dir)?;
    let config_path = state_dir.join(CONFIG_FILE);
    if config_path.exists() || state_dir.join("context.sqlite3").exists() {
        return Err(Failure::AlreadyInitialized(state_dir.clone()));
    }
    std::fs::write(&config_path, config_toml)?;
    std::fs::write(state_dir.join(".gitignore"), STATE_GITIGNORE)?;

    let store = middleman_store::Store::open(&state_dir)?;
    let event = genesis_event(&store, &config.project_name)?;
    store.append(&event)?;

    println!("initialized middleman in `{}`", state_dir.display());
    println!("  project:  {}", config.project_name);
    println!("  config:   {}", config_path.display());
    println!(
        "  state:    {}",
        state_dir.join("context.sqlite3").display()
    );
    println!("  events:   1 (ProjectInitialized)");
    Ok(())
}

/// Builds the `ProjectInitialized` event using fresh (time-ordered) ULID ids.
fn genesis_event(store: &middleman_store::Store, project_name: &str) -> Result<Event, Failure> {
    let (sequence, previous) = store.tail()?;
    let previous = previous.unwrap_or(Hash::genesis());

    let project_id = match store.state()?.project_id {
        Some(project_id) => project_id,
        None => ProjectId::new(format!("proj_{}", ulid::Ulid::generate()))?,
    };

    let event_id = EventId::new(format!("evt_{}", ulid::Ulid::generate()))?;
    let kind = middleman_core::EventKind::ProjectInitialized {
        project_name: project_name.to_owned(),
    };

    Ok(Event::new(
        event_id,
        project_id,
        sequence + 1,
        time::OffsetDateTime::now_utc(),
        middleman_core::Actor::User,
        kind,
        Vec::new(),
        None,
        previous,
    )?)
}

fn status(repo: &Path) -> Result<(), Failure> {
    let state_dir = require_state_dir(repo)?;
    let store = middleman_store::Store::open(&state_dir)?;
    let state = store.state()?;
    let (open, completed, abandoned) = task_tally(&state.tasks);
    let (pending, rejected) = proposal_tally(&state.proposals);

    println!("project:   {}", state.project_name);
    println!("state:     {}", state_dir.join(STATE_DB).display());
    println!("events:    {}", state.last_sequence);
    println!(
        "tail:      {}",
        state
            .last_hash
            .map_or_else(|| "-".to_string(), |h| h.to_string())
    );
    println!(
        "index:     {}",
        state
            .last_indexed_commit
            .as_deref()
            .unwrap_or("no commit indexed")
    );
    println!("entities:  {}", state.entities.len());
    println!("edges:     {}", state.edges.len());
    println!("tasks:     {open} open, {completed} completed, {abandoned} abandoned");
    println!("proposals: {pending} pending, {rejected} rejected");
    Ok(())
}

/// Splits the task records into open, completed, and abandoned counts.
fn task_tally(tasks: &BTreeMap<TaskId, TaskRecord>) -> (usize, usize, usize) {
    let open = tasks
        .values()
        .filter(|t| matches!(t.status, TaskStatus::Open | TaskStatus::InProgress))
        .count();
    let completed = tasks
        .values()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let abandoned = tasks
        .values()
        .filter(|t| t.status == TaskStatus::Abandoned)
        .count();
    (open, completed, abandoned)
}

/// Splits the proposal records into pending and rejected counts.
fn proposal_tally(
    proposals: &BTreeMap<ProposalId, middleman_core::ProposalRecord>,
) -> (usize, usize) {
    let pending = proposals.values().filter(|p| !p.rejected).count();
    let rejected = proposals.values().filter(|p| p.rejected).count();
    (pending, rejected)
}

fn doctor(repo: &Path) -> Result<(), Failure> {
    let checks = doctor_checks(repo);
    for (name, passed, detail) in &checks {
        let mark = if *passed { "ok  " } else { "fail" };
        println!("{mark}  {name:<10} {detail}");
    }
    let failed = checks.iter().filter(|c| !c.1).count();
    if failed == 0 {
        println!("doctor: all {} checks passed", checks.len());
        Ok(())
    } else {
        Err(Failure::DoctorChecks(failed, checks.len()))
    }
}

/// (name, passed, detail) for every environment check.
fn doctor_checks(repo: &Path) -> Vec<(&'static str, bool, String)> {
    let state_dir = repo.join(STATE_DIR);
    if !state_dir.is_dir() {
        return vec![(
            "state",
            false,
            ".middleman missing; run `middleman init`".to_string(),
        )];
    }

    let mut checks = vec![("state", true, format!("`{}`", state_dir.display()))];

    let config_path = state_dir.join(CONFIG_FILE);
    match read_toml_config(&config_path) {
        Ok(config) => checks.push((
            "config",
            true,
            format!("parsed (project: {})", config.project_name),
        )),
        Err(detail) => checks.push(("config", false, detail)),
    }

    match middleman_store::Store::open(&state_dir).and_then(|store| store.state()) {
        Ok(state) => checks.push((
            "event log",
            true,
            format!("{} events, hash chain verified", state.last_sequence),
        )),
        Err(e) => checks.push(("event log", false, e.to_string())),
    }

    match middleman_indexer::git::probe() {
        Ok(version) => checks.push(("git", true, version)),
        Err(e) => checks.push(("git", false, e.to_string())),
    }

    checks
}

fn read_toml_config(path: &Path) -> Result<Config, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 65_536 {
        return Err("configuration exceeds 64 KiB".into());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Config::parse(&raw).map_err(|e| e.to_string())
}

fn require_state_dir(repo: &Path) -> Result<PathBuf, Failure> {
    let state_dir = repo.join(STATE_DIR);
    if !state_dir.is_dir() {
        return Err(Failure::NotInitialized(repo.to_path_buf()));
    }
    Ok(state_dir)
}

pub fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            eprintln!(
                "{}",
                serde_json::json!({"error": {"code": "invalid_arguments", "message": "invalid command arguments; use --help"}})
            );
            return ExitCode::from(2);
        }
    };
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            if let Commands::Retrieval(_) = cli.command {
                let code = match &failure {
                    Failure::Retrieval(error) => error.code(),
                    _ => "invalid_repository",
                };
                eprintln!(
                    "{}",
                    serde_json::json!({"error": {"code": code, "message": failure.to_string()}})
                );
                return ExitCode::FAILURE;
            }
            eprintln!("{failure}");
            ExitCode::FAILURE
        }
    }
}
