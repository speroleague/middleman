//! Pure adapter from bounded document and graph observations to routing hints.

use crate::{
    document::parse_document,
    git::Snapshot,
    graph::Graph,
    scan::{FileKind, SourceFile},
};
use middleman_core::{EntityId, EntityPayload, config::LimitsConfig, routing::Hints};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

pub fn hints(
    files: &[SourceFile],
    graph: &Graph,
    history: Option<&Snapshot>,
    limits: &LimitsConfig,
) -> BTreeMap<EntityId, Hints> {
    let recent: BTreeSet<_> = history
        .into_iter()
        .flat_map(|history| history.changes.iter().map(|change| change.path.as_path()))
        .collect();
    let mut result = BTreeMap::new();
    let mut by_path: BTreeMap<String, Vec<EntityId>> = BTreeMap::new();
    for (id, entity) in &graph.entities {
        let entry = result.entry(id.clone()).or_insert_with(Hints::default);
        entry.routing_phrases = words(&entity.title);
        if let Some(path) = entity_path(&entity.payload) {
            let key = path.to_string_lossy().replace('\\', "/");
            by_path.entry(key).or_default().push(id.clone());
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                entry.routing_phrases.extend(words(stem));
            }
            entry.recent = recent.contains(path);
        }
    }
    for file in files.iter().filter(|file| file.kind == FileKind::Document) {
        let Ok(doc) = parse_document(&file.content, limits) else {
            continue;
        };
        if let Some(ids) = by_path.get(&file.path.to_string_lossy().replace('\\', "/")) {
            for id in ids {
                if let Some(entry) = result.get_mut(id) {
                    for heading in &doc.headings {
                        entry.routing_phrases.extend(words(&heading.text));
                    }
                    for key in ["tags", "id"] {
                        if let Some(value) = doc.frontmatter.get(key) {
                            entry.routing_phrases.extend(words(value));
                        }
                    }
                    entry.routing_phrases.extend(doc.identifiers.clone());
                }
            }
        }
        for rule in &doc.routing {
            for reference in &rule.references {
                let relative = file.path.parent().unwrap_or(Path::new("")).join(reference);
                let keys = [normalize(Path::new(reference)), normalize(&relative)];
                for key in keys.into_iter().flatten() {
                    if let Some(ids) = by_path.get(&key) {
                        for id in ids {
                            if let Some(entry) = result.get_mut(id) {
                                entry.routing_phrases.push(rule.condition.clone());
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    for entry in result.values_mut() {
        entry.routing_phrases.sort();
        entry.routing_phrases.dedup();
    }
    result
}

pub fn entity_path(payload: &EntityPayload) -> Option<&Path> {
    match payload {
        EntityPayload::Module { path, .. }
        | EntityPayload::Document { path, .. }
        | EntityPayload::Symbol { path, .. }
        | EntityPayload::Test { path, .. } => Some(path),
        EntityPayload::Contract { path, .. } => path.as_deref(),
        _ => None,
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .filter(|word| {
            word.len() >= 3
                && ![
                    "the",
                    "and",
                    "for",
                    "with",
                    "src",
                    "test",
                    "tests",
                    "routing",
                    "validation",
                ]
                .contains(word)
        })
        .map(str::to_owned)
        .collect()
}

fn normalize(path: &Path) -> Option<String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            _ => return None,
        }
    }
    Some(normalized.to_string_lossy().replace('\\', "/"))
}
