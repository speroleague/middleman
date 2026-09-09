//! Portable event-log transfer; presentation formats never mutate state.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::Path,
};

use middleman_core::Event;
use middleman_store::Store;

const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Format {
    Jsonl,
    Cir,
    Markdown,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("run middleman init first")]
    NotInitialized,
    #[error("transfer input is invalid, empty, or exceeds its limit")]
    InvalidInput,
    #[error("input JSONL is invalid")]
    Json(#[from] serde_json::Error),
    #[error("storage: {0}")]
    Store(#[from] middleman_store::Error),
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::InvalidInput | Self::Json(_) => "invalid_input",
            Self::Store(_) => "storage_error",
        }
    }
}

pub fn export(repo: &Path, format: Format) -> Result<(), Error> {
    let events = events(repo)?;
    if !matches!(format, Format::Jsonl) {
        let state =
            middleman_core::projection::project(&events).map_err(|_| Error::InvalidInput)?;
        let ranking = middleman_core::routing::Ranking {
            classification: middleman_core::routing::classify("", None),
            candidates: state
                .entities
                .keys()
                .map(|id| middleman_core::routing::Candidate {
                    id: id.clone(),
                    score: 1,
                    signals: BTreeSet::new(),
                    stale_penalty: 0,
                    learned_adjustment: 0,
                })
                .collect(),
            excluded: BTreeMap::new(),
            confidence: 0,
            low_confidence: true,
            truncated: false,
        };
        let output = middleman_packet::prepare(
            middleman_packet::Header {
                project: state.project_name,
                task: None,
                goal: "Active entity export; JSONL is required for restoration".into(),
                low_confidence: true,
                omitted: 0,
                upstream_truncated: false,
            },
            vec![],
            &ranking,
            &state.entities,
            if matches!(format, Format::Cir) {
                middleman_packet::Format::Cir
            } else {
                middleman_packet::Format::Markdown
            },
            middleman_packet::Limits {
                items: state.entities.len(),
                expandable: 0,
                budget: usize::MAX,
            },
        )
        .map_err(|_| Error::InvalidInput)?;
        print!("{}", output.text);
        return Ok(());
    }
    for event in events {
        println!("{}", serde_json::to_string(&event)?);
    }
    Ok(())
}

pub fn backup(repo: &Path, path: &Path) -> Result<(), Error> {
    let output = jsonl(&events(repo)?)?;
    if output.len() as u64 > MAX_BYTES {
        return Err(Error::InvalidInput);
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| Error::InvalidInput)?;
    file.write_all(output.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| Error::InvalidInput)
}

pub fn import(repo: &Path, path: &Path) -> Result<(), Error> {
    let events = read(path)?;
    middleman_core::projection::project(&events).map_err(|_| Error::InvalidInput)?;
    Store::open(&repo.join(super::STATE_DIR))?.replace_events(&events)?;
    println!("{}", serde_json::json!({"imported_events": events.len()}));
    Ok(())
}

pub fn restore(repo: &Path, path: &Path) -> Result<(), Error> {
    let events = read(path)?;
    Store::restore_events(&repo.join(super::STATE_DIR), &events)?;
    println!("{}", serde_json::json!({"restored_events": events.len()}));
    Ok(())
}

fn events(repo: &Path) -> Result<Vec<Event>, Error> {
    let directory = repo.join(super::STATE_DIR);
    if !directory.join(super::STATE_DB).is_file() {
        return Err(Error::NotInitialized);
    }
    Ok(Store::open_read_only(&directory)?.events()?)
}

fn read(path: &Path) -> Result<Vec<Event>, Error> {
    let metadata = fs::metadata(path).map_err(|_| Error::InvalidInput)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(Error::InvalidInput);
    }
    let mut text = String::new();
    fs::File::open(path)
        .map_err(|_| Error::InvalidInput)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| Error::InvalidInput)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(Error::InvalidInput);
    }
    let events: Result<Vec<_>, _> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect();
    let events = events?;
    if events.is_empty() {
        return Err(Error::InvalidInput);
    }
    Ok(events)
}

fn jsonl(events: &[Event]) -> Result<String, Error> {
    events
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| format!("{}\n", lines.join("\n")))
        .map_err(Error::from)
}
