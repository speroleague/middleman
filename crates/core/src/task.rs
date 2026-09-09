//! Pure task observations: endpoint differences are not proof of causality.

use crate::{EntityId, Hash};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub files: BTreeMap<PathBuf, Hash>,
    pub symbols: BTreeSet<EntityId>,
    pub git_commit: Option<String>,
    pub history_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Baseline,
    Finish,
}

pub fn valid(snapshot: &Snapshot) -> bool {
    snapshot.files.len() <= 100_000
        && snapshot.symbols.len() <= 100_000
        && snapshot.files.keys().all(|path| {
            !path.as_os_str().is_empty()
                && path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
        })
        && snapshot
            .files
            .keys()
            .map(|path| path.as_os_str().len())
            .sum::<usize>()
            <= 8 * 1024 * 1024
        && snapshot
            .symbols
            .iter()
            .map(|id| id.as_str().len())
            .sum::<usize>()
            <= 8 * 1024 * 1024
        && snapshot.git_commit.as_ref().is_none_or(|sha| {
            matches!(sha.len(), 40 | 64) && sha.bytes().all(|ch| ch.is_ascii_hexdigit())
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    pub baseline_available: bool,
    pub added: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
    pub unavailable: Vec<PathBuf>,
    pub symbols_added: Vec<EntityId>,
    pub symbols_removed: Vec<EntityId>,
    pub git_before: Option<String>,
    pub git_after: Option<String>,
    pub history_truncated: bool,
}

pub fn compare(before: Option<&Snapshot>, after: &Snapshot) -> Changes {
    let mut result = Changes {
        baseline_available: before.is_some(),
        added: vec![],
        changed: vec![],
        unavailable: vec![],
        symbols_added: vec![],
        symbols_removed: vec![],
        git_before: before.and_then(|snapshot| snapshot.git_commit.clone()),
        git_after: after.git_commit.clone(),
        history_truncated: after.history_truncated
            || before.is_some_and(|snapshot| snapshot.history_truncated),
    };
    if let Some(before) = before {
        for (path, hash) in &after.files {
            match before.files.get(path) {
                None => result.added.push(path.clone()),
                Some(previous) if previous != hash => result.changed.push(path.clone()),
                _ => {}
            }
        }
        result.unavailable = before
            .files
            .keys()
            .filter(|path| !after.files.contains_key(*path))
            .cloned()
            .collect();
        result.symbols_added = after.symbols.difference(&before.symbols).cloned().collect();
        result.symbols_removed = before.symbols.difference(&after.symbols).cloned().collect();
    }
    result
}

pub fn summary(changes: &Changes) -> String {
    if !changes.baseline_available {
        return "Task finished; no baseline available for comparison.".into();
    }
    format!(
        "Observed {} added, {} changed and {} unavailable paths; {} added and {} removed symbol IDs.",
        changes.added.len(),
        changes.changed.len(),
        changes.unavailable.len(),
        changes.symbols_added.len(),
        changes.symbols_removed.len()
    )
}
