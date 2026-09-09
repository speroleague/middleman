//! Pure graph refresh. Opaque cached facts/fragments never contain source text.

mod facts;
mod resolve;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use middleman_core::config::LimitsConfig;
use middleman_core::{Edge, EdgeKind, Entity, EntityId, Evidence, Hash};

use crate::{
    document, git, language,
    scan::{self, SourceFile},
};
use facts::{Facts, Fragment};
use resolve::{Catalog, Resolution};

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_files: usize,
    pub max_input_bytes: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    /// Maximum pair observations across all supplied commits, including repeats.
    pub max_cochanges: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_files: 100_000,
            max_input_bytes: 32 * 1024 * 1024,
            max_nodes: 100_000,
            max_edges: 200_000,
            max_cochanges: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Missing,
    Ambiguous,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    pub source: PathBuf,
    pub line: u32,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoChange {
    pub left: EntityId,
    pub right: EntityId,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Graph {
    pub entities: BTreeMap<EntityId, Entity>,
    pub edges: Vec<Edge>,
    pub cochanges: Vec<CoChange>,
    pub diagnostics: Vec<Diagnostic>,
    pub history_truncated: bool,
}

#[derive(Debug, Clone)]
struct Cached {
    facts: Arc<Facts>,
    resolution: Resolution,
    fragment: Arc<Fragment>,
}

#[derive(Debug, Clone)]
pub struct Index {
    files: BTreeMap<PathBuf, Cached>,
    graph: Graph,
    limits: LimitsConfig,
}

impl Index {
    pub fn graph(&self) -> &Graph {
        &self.graph
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub reparsed: Vec<PathBuf>,
    pub rederived: Vec<PathBuf>,
    pub reused: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Refresh {
    pub index: Index,
    pub changes: Changes,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("graph input contains an invalid path, kind, duplicate, or content hash")]
    InvalidInput,
    #[error("graph exceeds its {0} budget")]
    Budget(&'static str),
    #[error("Cargo manifest metadata is invalid")]
    Manifest,
    #[error("Git evidence contains an invalid commit identifier")]
    GitEvidence,
    #[error("derived graph contains duplicate identities or dangling relationships")]
    InvalidGraph,
    #[error(transparent)]
    Core(#[from] middleman_core::Error),
    #[error(transparent)]
    Document(#[from] document::DocumentError),
    #[error(transparent)]
    Language(#[from] language::Error),
}

pub fn refresh(
    previous: Option<&Index>,
    source: &[SourceFile],
    history: Option<&git::Snapshot>,
    limits: &LimitsConfig,
    budget: Budget,
) -> Result<Refresh, Error> {
    validate(source, limits, budget)?;
    let empty = BTreeMap::new();
    let old = previous.map_or(&empty, |index| &index.files);
    let reset = previous.is_none_or(|index| index.limits != *limits);
    let mut changes = Changes::default();
    let mut parsed = BTreeMap::new();
    for file in source {
        let path = PathBuf::from(path_key(&file.path)?);
        let cached = old.get(&file.path);
        let modified = cached
            .is_some_and(|entry| entry.facts.hash != file.hash || entry.facts.kind != file.kind);
        if cached.is_none() {
            changes.added.push(path.clone());
        }
        if modified {
            changes.changed.push(path.clone());
        }
        let facts = if !reset && !modified {
            cached.map(|entry| Arc::clone(&entry.facts))
        } else {
            None
        };
        let facts = if let Some(facts) = facts {
            facts
        } else {
            changes.reparsed.push(path.clone());
            Arc::new(Facts::parse(file, limits)?)
        };
        parsed.insert(path, facts);
    }
    changes.removed = old
        .keys()
        .filter(|path| !parsed.contains_key(*path))
        .cloned()
        .collect();
    let catalog = Catalog::new(&parsed);
    let resolutions: BTreeMap<_, _> = parsed
        .iter()
        .map(|(path, facts)| Ok((path.clone(), catalog.resolve(path, facts)?)))
        .collect::<Result<_, Error>>()?;
    let seeds: BTreeSet<_> = changes
        .reparsed
        .iter()
        .chain(&changes.removed)
        .cloned()
        .collect();
    let affected = direct_neighborhood(&seeds, old, &resolutions);
    let mut files = BTreeMap::new();
    for (path, facts) in parsed {
        let resolution = resolutions.get(&path).ok_or(Error::InvalidGraph)?.clone();
        let fragment = if affected.contains(&path) {
            None
        } else {
            old.get(&path).map(|cached| Arc::clone(&cached.fragment))
        };
        let fragment = if let Some(fragment) = fragment {
            changes.reused.push(path.clone());
            fragment
        } else {
            changes.rederived.push(path.clone());
            Arc::new(facts::derive(&path, &facts, &resolution, budget)?)
        };
        files.insert(
            path,
            Cached {
                facts,
                resolution,
                fragment,
            },
        );
    }
    changes.added.sort();
    changes.changed.sort();
    changes.reparsed.sort();
    let graph = assemble(&files, history, budget)?;
    Ok(Refresh {
        index: Index {
            files,
            graph,
            limits: limits.clone(),
        },
        changes,
    })
}

fn direct_neighborhood(
    seeds: &BTreeSet<PathBuf>,
    old: &BTreeMap<PathBuf, Cached>,
    resolutions: &BTreeMap<PathBuf, Resolution>,
) -> BTreeSet<PathBuf> {
    let mut affected = seeds.clone();
    for (path, resolution) in resolutions {
        if old
            .get(path)
            .is_none_or(|cached| cached.resolution != *resolution)
        {
            affected.insert(path.clone());
        }
    }
    for (path, resolution) in old
        .iter()
        .map(|(p, c)| (p, &c.resolution))
        .chain(resolutions.iter())
    {
        for link in &resolution.links {
            if seeds.contains(path) {
                affected.insert(link.path.clone());
            }
            if seeds.contains(&link.path) {
                affected.insert(path.clone());
            }
        }
    }
    affected
}

fn validate(source: &[SourceFile], limits: &LimitsConfig, budget: Budget) -> Result<(), Error> {
    if source.len() > budget.max_files {
        return Err(Error::Budget("files"));
    }
    let mut paths = BTreeSet::new();
    let mut bytes = 0usize;
    for file in source {
        path_key(&file.path)?;
        if !paths.insert(&file.path) || scan::classify(&file.path) != Some(file.kind) {
            return Err(Error::InvalidInput);
        }
        bytes = bytes
            .checked_add(file.content.len())
            .ok_or(Error::Budget("input bytes"))?;
        if bytes > budget.max_input_bytes {
            return Err(Error::Budget("input bytes"));
        }
        if file.content.len() as u64 > u64::from(limits.max_file_kb) * 1024 {
            return Err(Error::Budget("file bytes"));
        }
        if Hash::of(file.content.as_bytes()) != file.hash || file.content.contains('\0') {
            return Err(Error::InvalidInput);
        }
        if file.content.lines().count() as u64 > limits.max_lines {
            return Err(Error::Budget("lines"));
        }
    }
    Ok(())
}

fn assemble(
    files: &BTreeMap<PathBuf, Cached>,
    history: Option<&git::Snapshot>,
    budget: Budget,
) -> Result<Graph, Error> {
    let mut graph = Graph::default();
    let mut edges: BTreeMap<(EntityId, EntityId, EdgeKind), Edge> = BTreeMap::new();
    let mut edge_observations = 0usize;
    for cached in files.values() {
        for entity in &cached.fragment.entities {
            if graph.entities.len() >= budget.max_nodes {
                return Err(Error::Budget("nodes"));
            }
            if graph
                .entities
                .insert(entity.id.clone(), entity.clone())
                .is_some()
            {
                return Err(Error::InvalidGraph);
            }
        }
        for edge in &cached.fragment.edges {
            edge_observations += 1;
            if edge_observations > budget.max_edges {
                return Err(Error::Budget("edges"));
            }
            let entry = edges
                .entry((edge.from.clone(), edge.to.clone(), edge.kind))
                .or_insert_with(|| Edge {
                    evidence: Vec::new(),
                    ..edge.clone()
                });
            for evidence in &edge.evidence {
                if !entry.evidence.contains(evidence) {
                    entry.evidence.push(evidence.clone());
                }
            }
        }
        graph
            .diagnostics
            .extend(cached.resolution.diagnostics.clone());
    }
    graph.edges = edges.into_values().collect();
    if graph.edges.iter().any(|edge| {
        !graph.entities.contains_key(&edge.from) || !graph.entities.contains_key(&edge.to)
    }) {
        return Err(Error::InvalidGraph);
    }
    graph.diagnostics.sort();
    graph.diagnostics.dedup();
    if let Some(history) = history {
        graph.history_truncated = history.history_truncated;
        graph.cochanges = cochanges(files, history, budget)?;
    }
    Ok(graph)
}

fn cochanges(
    files: &BTreeMap<PathBuf, Cached>,
    history: &git::Snapshot,
    budget: Budget,
) -> Result<Vec<CoChange>, Error> {
    if history.commits.len() > 1024 {
        return Err(Error::Budget("history"));
    }
    let mut pairs: BTreeMap<(EntityId, EntityId), CoChange> = BTreeMap::new();
    let mut observations = 0usize;
    for commit in &history.commits {
        if !matches!(commit.sha.len(), 40 | 64)
            || !commit.sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::GitEvidence);
        }
        if commit.paths.len() > budget.max_files {
            return Err(Error::Budget("history paths"));
        }
        let paths: Vec<_> = commit
            .paths
            .iter()
            .filter_map(|path| files.get_key_value(path).map(|(path, _)| path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let count = paths.len().saturating_mul(paths.len().saturating_sub(1)) / 2;
        observations = observations
            .checked_add(count)
            .ok_or(Error::Budget("cochanges"))?;
        if observations > budget.max_cochanges {
            return Err(Error::Budget("cochanges"));
        }
        for (position, left) in paths.iter().enumerate() {
            for right in &paths[position + 1..] {
                let mut ids = [
                    facts::file_id(left, files[*left].facts.kind)?,
                    facts::file_id(right, files[*right].facts.kind)?,
                ];
                ids.sort();
                let key = (ids[0].clone(), ids[1].clone());
                let pair = pairs.entry(key.clone()).or_insert_with(|| CoChange {
                    left: key.0,
                    right: key.1,
                    evidence: Vec::new(),
                });
                let evidence = Evidence::GitCommit {
                    sha: commit.sha.clone(),
                    paths: vec![(*left).clone(), (*right).clone()],
                };
                if !pair.evidence.contains(&evidence) {
                    pair.evidence.push(evidence);
                }
            }
        }
    }
    Ok(pairs.into_values().collect())
}

fn path_key(path: &Path) -> Result<String, Error> {
    if path.as_os_str().len() > 4096 {
        return Err(Error::InvalidInput);
    }
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(Error::InvalidInput);
        };
        let name = name.to_str().ok_or(Error::InvalidInput)?;
        if name.contains(['\\', '\0']) {
            return Err(Error::InvalidInput);
        }
        parts.push(name);
    }
    if parts.is_empty() {
        return Err(Error::InvalidInput);
    }
    Ok(parts.join("/"))
}
