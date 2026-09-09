//! Deterministic request classification and explainable, one-hop retrieval.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Edge, EdgeKind, Entity, EntityId, EntityKind, EntityPayload, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Feature,
    Bug,
    Refactor,
    Test,
    Documentation,
    Operations,
    Security,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    pub modes: BTreeSet<Mode>,
    pub explicit_mode: bool,
}

#[must_use]
pub fn classify(request: &str, mode: Option<Mode>) -> Classification {
    let folded = request.to_lowercase();
    let modes = mode.map_or_else(
        || {
            [
                (Mode::Feature, &["add", "feature", "implement"][..]),
                (Mode::Bug, &["fix", "bug", "failure", "broken"][..]),
                (Mode::Refactor, &["refactor", "simplify", "rename"][..]),
                (Mode::Test, &["test", "tests", "coverage"][..]),
                (Mode::Documentation, &["docs", "document", "readme"][..]),
                (
                    Mode::Operations,
                    &["deploy", "deployment", "ci", "backup"][..],
                ),
                (
                    Mode::Security,
                    &["security", "authorization", "authentication"][..],
                ),
            ]
            .into_iter()
            .filter_map(|(mode, words)| {
                words
                    .iter()
                    .any(|word| contains(&folded, word))
                    .then_some(mode)
            })
            .collect()
        },
        |mode| BTreeSet::from([mode]),
    );
    Classification {
        modes,
        explicit_mode: mode.is_some(),
    }
}

/// Adapter-supplied metadata. Phrases may include headings, tags, issue IDs or URLs.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    pub routing_phrases: Vec<String>,
    pub modes: BTreeSet<Mode>,
    pub recent_task: bool,
    pub recent: bool,
    pub ignored: bool,
    pub stale_penalty: u16,
    /// Bounded, local outcome adjustment. It can only reorder candidates that
    /// already have deterministic routing evidence.
    pub learned_adjustment: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    ExactPath,
    ExactSymbol,
    ContractOrInvariant,
    ExplicitRouting,
    DirectDependency,
    DirectTest,
    RecentTask,
    CoChange,
    Recency,
}

impl Signal {
    #[must_use]
    pub const fn weight(self) -> i32 {
        match self {
            Self::ExactPath => 100,
            Self::ExactSymbol => 90,
            Self::ContractOrInvariant => 80,
            Self::ExplicitRouting => 70,
            Self::DirectDependency => 50,
            Self::DirectTest => 45,
            Self::RecentTask => 25,
            Self::CoChange => 15,
            Self::Recency => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: EntityId,
    pub score: i32,
    pub signals: BTreeSet<Signal>,
    pub stale_penalty: u16,
    #[serde(default)]
    pub learned_adjustment: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclusion {
    Ignored,
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ranking {
    pub classification: Classification,
    pub candidates: Vec<Candidate>,
    pub excluded: BTreeMap<EntityId, Exclusion>,
    /// Heuristic strength in 0..=100, not a calibrated probability.
    pub confidence: i32,
    pub low_confidence: bool,
    pub truncated: bool,
}

/// Already bounded observations; neither history nor metadata are fetched here.
pub struct Context<'a> {
    pub entities: &'a BTreeMap<EntityId, Entity>,
    pub edges: &'a [Edge],
    pub hints: &'a BTreeMap<EntityId, Hints>,
    pub cochanges: &'a [(EntityId, EntityId)],
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn rank(request: &str, mode: Option<Mode>, context: &Context<'_>, limit: usize) -> Ranking {
    let classification = classify(request, mode);
    let normalized = request.replace('\\', "/");
    let folded = normalized.to_lowercase();
    let mut excluded = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    let mut seeds = BTreeSet::new();

    for (id, entity) in context.entities {
        let empty = Hints::default();
        let hints = context.hints.get(id).unwrap_or(&empty);
        if hints.ignored || entity.status != Status::Active {
            excluded.insert(
                id.clone(),
                if hints.ignored {
                    Exclusion::Ignored
                } else {
                    Exclusion::Inactive
                },
            );
            continue;
        }
        let signals = direct_signals(&normalized, &folded, entity, hints, &classification);
        if signals.iter().any(|signal| signal.weight() >= 70) && hints.stale_penalty == 0 {
            seeds.insert(id.clone());
        }
        candidates.insert(
            id.clone(),
            Candidate {
                id: id.clone(),
                score: 0,
                signals,
                stale_penalty: hints.stale_penalty,
                learned_adjustment: hints.learned_adjustment,
            },
        );
    }

    for edge in context.edges {
        if edge.from == edge.to {
            continue;
        }
        let signal = match edge.kind {
            EdgeKind::Imports | EdgeKind::DependsOn | EdgeKind::Calls => Signal::DirectDependency,
            EdgeKind::Tests | EdgeKind::Validates => Signal::DirectTest,
            _ => continue,
        };
        if seeds.contains(&edge.from) {
            add_signal(&mut candidates, &edge.to, signal);
        }
        if seeds.contains(&edge.to) {
            add_signal(&mut candidates, &edge.from, signal);
        }
    }
    for (left, right) in context.cochanges {
        if left == right {
            continue;
        }
        if seeds.contains(left) {
            add_signal(&mut candidates, right, Signal::CoChange);
        }
        if seeds.contains(right) {
            add_signal(&mut candidates, left, Signal::CoChange);
        }
    }

    let mut candidates: Vec<_> = candidates
        .into_values()
        .map(|mut candidate| {
            candidate.score = candidate
                .signals
                .iter()
                .map(|signal| signal.weight())
                .sum::<i32>()
                - i32::from(candidate.stale_penalty)
                + i32::from(candidate.learned_adjustment);
            candidate
        })
        .filter(|candidate| !candidate.signals.is_empty() && candidate.score > 0)
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    let truncated = candidates.len() > limit;
    candidates.truncate(limit);
    let confidence = candidates
        .iter()
        .map(|candidate| {
            let direct = candidate
                .signals
                .iter()
                .filter(|signal| signal.weight() >= 70)
                .map(|signal| signal.weight())
                .max()
                .unwrap_or(0);
            (direct - i32::from(candidate.stale_penalty)).clamp(0, 100)
        })
        .max()
        .unwrap_or(0);
    Ranking {
        classification,
        candidates,
        excluded,
        confidence,
        low_confidence: confidence < 55,
        truncated,
    }
}

fn add_signal(candidates: &mut BTreeMap<EntityId, Candidate>, id: &EntityId, signal: Signal) {
    if let Some(candidate) = candidates.get_mut(id) {
        candidate.signals.insert(signal);
    }
}

fn direct_signals(
    request: &str,
    folded: &str,
    entity: &Entity,
    hints: &Hints,
    classification: &Classification,
) -> BTreeSet<Signal> {
    let mut signals = BTreeSet::new();
    let path = match &entity.payload {
        EntityPayload::Module { path, .. }
        | EntityPayload::Symbol { path, .. }
        | EntityPayload::Test { path, .. }
        | EntityPayload::Document { path, .. } => Some(path),
        EntityPayload::Contract { path, .. } => path.as_ref(),
        _ => None,
    };
    if path
        .and_then(|path| path.to_str())
        .is_some_and(|path| contains(request, &path.replace('\\', "/")))
    {
        signals.insert(Signal::ExactPath);
    }
    if entity.kind == EntityKind::Symbol && contains(request, &entity.title) {
        signals.insert(Signal::ExactSymbol);
    }
    if matches!(entity.kind, EntityKind::Contract | EntityKind::Invariant)
        && contains(folded, &entity.title.to_lowercase())
    {
        signals.insert(Signal::ContractOrInvariant);
    }
    if hints
        .routing_phrases
        .iter()
        .any(|phrase| contains(folded, &phrase.replace('\\', "/").to_lowercase()))
        || (classification.explicit_mode && !classification.modes.is_disjoint(&hints.modes))
    {
        signals.insert(Signal::ExplicitRouting);
    }
    if hints.recent_task {
        signals.insert(Signal::RecentTask);
    }
    if hints.recent {
        signals.insert(Signal::Recency);
    }
    signals
}

fn contains(text: &str, phrase: &str) -> bool {
    if phrase.trim().is_empty() {
        return false;
    }
    text.match_indices(phrase).any(|(start, _)| {
        let end = start + phrase.len();
        !text[..start]
            .chars()
            .next_back()
            .is_some_and(identifier_char)
            && !text[end..].chars().next().is_some_and(identifier_char)
    })
}

fn identifier_char(value: char) -> bool {
    value.is_alphanumeric() || matches!(value, '_' | '/' | '\\' | '-' | '.')
}

/// Rank an explicit ID and its direct dependency/test neighbors (at most 128).
pub fn expand(id: &EntityId, context: &Context<'_>) -> Ranking {
    let eligible = |id: &EntityId| {
        context
            .entities
            .get(id)
            .is_some_and(|entity| entity.status == Status::Active)
            && context
                .hints
                .get(id)
                .is_none_or(|hints| !hints.ignored && hints.stale_penalty == 0)
    };
    if !eligible(id) {
        return Ranking {
            candidates: vec![],
            classification: classify("", None),
            excluded: BTreeMap::new(),
            confidence: 0,
            low_confidence: true,
            truncated: false,
        };
    }
    let mut signals = BTreeMap::from([(id.clone(), Signal::ExplicitRouting)]);
    for edge in context.edges {
        let target = if edge.from == *id {
            &edge.to
        } else if edge.to == *id {
            &edge.from
        } else {
            continue;
        };
        if target == id {
            continue;
        }
        let signal = match edge.kind {
            EdgeKind::Imports | EdgeKind::DependsOn | EdgeKind::Calls => Signal::DirectDependency,
            EdgeKind::Tests | EdgeKind::Validates => Signal::DirectTest,
            _ => continue,
        };
        signals
            .entry(target.clone())
            .and_modify(|current| {
                if signal.weight() > current.weight() {
                    *current = signal;
                }
            })
            .or_insert(signal);
    }
    let mut candidates: Vec<_> = signals
        .into_iter()
        .filter(|(id, _)| eligible(id))
        .map(|(id, signal)| Candidate {
            id,
            score: signal.weight(),
            signals: BTreeSet::from([signal]),
            stale_penalty: 0,
            learned_adjustment: 0,
        })
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    let truncated = candidates.len() > 128;
    candidates.truncate(128);
    Ranking {
        candidates,
        classification: classify("", None),
        excluded: BTreeMap::new(),
        confidence: 70,
        low_confidence: false,
        truncated,
    }
}
