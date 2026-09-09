//! Explicit, bounded index refresh and persistence orchestration.

use std::path::Path;

use clap::Args;
use middleman_core::{Actor, Event, EventId, EventKind, Hash};
use middleman_store::Store;

use super::retrieval;

#[derive(Debug, Args)]
pub struct Options {
    /// Discard the reusable source-free cache before rebuilding.
    #[arg(long)]
    full: bool,
    /// Prefer the reusable cache; a missing or invalid cache falls back to a full refresh.
    #[arg(long)]
    changed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("run middleman init first")]
    NotInitialized,
    #[error("--full and --changed cannot be used together")]
    InvalidInput,
    #[error("indexing: {0}")]
    Index(#[from] retrieval::Error),
    #[error("storage: {0}")]
    Store(#[from] middleman_store::Error),
    #[error("domain: {0}")]
    Core(#[from] middleman_core::Error),
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::InvalidInput => "invalid_input",
            Self::Index(_) => "index_error",
            Self::Store(_) => "storage_error",
            Self::Core(_) => "domain_error",
        }
    }
}

pub fn run(repo: &Path, options: &Options) -> Result<(), Error> {
    if options.full && options.changed {
        return Err(Error::InvalidInput);
    }
    let state_dir = repo.join(super::STATE_DIR);
    if !state_dir.join(super::STATE_DB).is_file() {
        return Err(Error::NotInitialized);
    }
    let store = Store::open(&state_dir)?;
    let state = store.state()?;
    let previous = (!options.full)
        .then(|| store.index_snapshot())
        .transpose()?
        .flatten()
        .and_then(|bytes| middleman_indexer::graph::Index::decode(&bytes).ok());
    let refreshed = retrieval::refresh_input(repo, state.clone(), true, previous.as_ref())?;
    let cache = refreshed
        .index
        .encode()
        .map_err(|_| Error::Index(retrieval::Error::InvalidInput))?;
    let event = Event::new(
        EventId::new(format!("evt_{}", ulid::Ulid::generate()))?,
        state.project_id.ok_or(Error::NotInitialized)?,
        state.last_sequence + 1,
        time::OffsetDateTime::now_utc(),
        Actor::System,
        EventKind::SourceIndexed {
            git_commit: refreshed.loaded.snapshot.git_commit,
            added: refreshed.changes.added,
            changed: refreshed.changes.changed,
            removed: refreshed.changes.removed,
        },
        vec![],
        None,
        state.last_hash.unwrap_or(Hash::genesis()),
    )?;
    store.append_batch_with_snapshot(&[event], Some(&cache))?;
    println!(
        "{}",
        serde_json::json!({
            "reparsed": refreshed.changes.reparsed.len(),
            "rederived": refreshed.changes.rederived.len(),
            "reused": refreshed.changes.reused.len(),
            "cache": if previous.is_some() { "reused" } else { "rebuilt" },
            "history_truncated": refreshed.loaded.graph.history_truncated,
            "diagnostics": refreshed.loaded.graph.diagnostics.len(),
        })
    );
    Ok(())
}
