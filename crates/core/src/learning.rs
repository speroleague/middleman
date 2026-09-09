//! Deterministic learning over source-free retrieval observations.
//!
//! This module never reads a clock, filesystem, prompt, or model response. It
//! turns bounded event-log outcomes into explainable ranking adjustments and
//! maintenance suggestions. The CLI owns review and configuration writes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    Edge, EdgeKind, Entity, EntityId, EntityKind, Evidence, RetrievalRecord, RetrievalSignal,
    State, Status, WeightConfig,
};

/// Prevent a long-lived history from overpowering deterministic routing.
pub const MAX_LEARNED_ADJUSTMENT: i16 = 20;
const MIN_SUPPORT: usize = 2;

/// A transparent local count for one retrieved node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeCounts {
    pub retrieved: usize,
    pub expanded: usize,
    pub referenced: usize,
    pub file_overlap: usize,
    pub test_overlap: usize,
    pub user_accepted: usize,
    pub user_rejected: usize,
}

impl OutcomeCounts {
    fn record(&mut self, signal: RetrievalSignal) {
        match signal {
            RetrievalSignal::Retrieved => self.retrieved += 1,
            RetrievalSignal::Expanded => self.expanded += 1,
            RetrievalSignal::Referenced => self.referenced += 1,
            RetrievalSignal::FileOverlap => self.file_overlap += 1,
            RetrievalSignal::TestOverlap => self.test_overlap += 1,
            RetrievalSignal::UserAccepted => self.user_accepted += 1,
            RetrievalSignal::UserRejected => self.user_rejected += 1,
        }
    }

    #[must_use]
    pub const fn positive_outcomes(&self) -> usize {
        self.expanded + self.referenced + self.file_overlap + self.test_overlap + self.user_accepted
    }
}

/// One suggested configuration field and its local evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightSuggestion {
    pub field: String,
    pub current: f64,
    pub proposed: f64,
    pub observations: usize,
    pub retrieved: usize,
}

/// A reviewable maintenance item. None of these types authorize a write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaintenanceSuggestion {
    MissingDocumentation {
        node_id: EntityId,
        title: String,
        file_overlap: usize,
    },
    RoutingRule {
        node_id: EntityId,
        title: String,
        successful_outcomes: usize,
    },
    StaleDecision {
        node_id: EntityId,
        title: String,
        paths: Vec<String>,
    },
}

/// The complete, stable output of a learning review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub outcomes: BTreeMap<EntityId, OutcomeCounts>,
    pub learned_adjustments: BTreeMap<EntityId, i16>,
    pub weight_suggestions: Vec<WeightSuggestion>,
    pub maintenance_suggestions: Vec<MaintenanceSuggestion>,
}

/// Aggregates immutable observations without applying a policy decision.
#[must_use]
pub fn outcomes(records: &[RetrievalRecord]) -> BTreeMap<EntityId, OutcomeCounts> {
    let mut result = BTreeMap::new();
    for record in records {
        result
            .entry(record.node_id.clone())
            .or_insert_with(OutcomeCounts::default)
            .record(record.signal);
    }
    result
}

/// Calculates bounded adjustment points from the configured, explainable
/// outcome weights. A node must have been retrieved before it can earn one.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn adjustments(records: &[RetrievalRecord], weights: WeightConfig) -> BTreeMap<EntityId, i16> {
    outcomes(records)
        .into_iter()
        .filter_map(|(id, counts)| {
            (counts.retrieved > 0)
                .then(|| {
                    let positive = counts.expanded as f64 * weights.expanded
                        + counts.referenced as f64 * weights.referenced
                        + counts.file_overlap as f64 * weights.file_overlap
                        + counts.test_overlap as f64 * weights.test_overlap
                        + counts.user_accepted as f64 * weights.accepted;
                    let negative = counts.user_rejected as f64 * weights.rejected;
                    let bounded = (positive - negative).round().clamp(
                        -f64::from(MAX_LEARNED_ADJUSTMENT),
                        f64::from(MAX_LEARNED_ADJUSTMENT),
                    );
                    (bounded != 0.0).then_some((id, bounded as i16))
                })
                .flatten()
        })
        .collect()
}

/// Produces a report from local state and the current source-free graph.
#[must_use]
pub fn analyze(
    state: &State,
    entities: &BTreeMap<EntityId, Entity>,
    edges: &[Edge],
    weights: WeightConfig,
) -> Report {
    let outcomes = outcomes(&state.retrieval);
    let learned_adjustments = adjustments(&state.retrieval, weights);
    let weight_suggestions = suggest_weights(&outcomes, weights);
    let maintenance_suggestions = maintenance(entities, edges, &outcomes);
    Report {
        outcomes,
        learned_adjustments,
        weight_suggestions,
        maintenance_suggestions,
    }
}

fn suggest_weights(
    counts: &BTreeMap<EntityId, OutcomeCounts>,
    weights: WeightConfig,
) -> Vec<WeightSuggestion> {
    let retrieved: usize = counts.values().map(|outcome| outcome.retrieved).sum();
    if retrieved == 0 {
        return Vec::new();
    }
    [
        (
            "test_overlap",
            weights.test_overlap,
            counts.values().map(|v| v.test_overlap).sum(),
        ),
        (
            "file_overlap",
            weights.file_overlap,
            counts.values().map(|v| v.file_overlap).sum(),
        ),
        (
            "expanded",
            weights.expanded,
            counts.values().map(|v| v.expanded).sum(),
        ),
        (
            "referenced",
            weights.referenced,
            counts.values().map(|v| v.referenced).sum(),
        ),
        (
            "accepted",
            weights.accepted,
            counts.values().map(|v| v.user_accepted).sum(),
        ),
        (
            "rejected",
            weights.rejected,
            counts.values().map(|v| v.user_rejected).sum(),
        ),
    ]
    .into_iter()
    .filter(|(_, _, observations)| *observations >= MIN_SUPPORT)
    .map(|(field, current, observations)| WeightSuggestion {
        field: field.into(),
        current,
        proposed: round_weight(current * (1.0 + outcome_ratio(observations, retrieved))),
        observations,
        retrieved,
    })
    .collect()
}

fn round_weight(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[allow(clippy::cast_precision_loss)]
fn outcome_ratio(observations: usize, retrieved: usize) -> f64 {
    observations as f64 / retrieved as f64
}

fn maintenance(
    entities: &BTreeMap<EntityId, Entity>,
    edges: &[Edge],
    counts: &BTreeMap<EntityId, OutcomeCounts>,
) -> Vec<MaintenanceSuggestion> {
    let documented: BTreeSet<_> = edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Documents)
        .flat_map(|edge| [edge.from.clone(), edge.to.clone()])
        .collect();
    let hashes = current_hashes(entities);
    let mut suggestions = Vec::new();
    for (id, entity) in entities {
        if entity.status != Status::Active {
            continue;
        }
        let outcome = counts.get(id).cloned().unwrap_or_default();
        if entity.kind == EntityKind::Module
            && outcome.file_overlap >= MIN_SUPPORT
            && !documented.contains(id)
        {
            suggestions.push(MaintenanceSuggestion::MissingDocumentation {
                node_id: id.clone(),
                title: entity.title.clone(),
                file_overlap: outcome.file_overlap,
            });
        }
        if outcome.positive_outcomes() >= 3 {
            suggestions.push(MaintenanceSuggestion::RoutingRule {
                node_id: id.clone(),
                title: entity.title.clone(),
                successful_outcomes: outcome.positive_outcomes(),
            });
        }
        if entity.kind == EntityKind::Decision {
            let paths = stale_paths(entity, &hashes);
            if !paths.is_empty() {
                suggestions.push(MaintenanceSuggestion::StaleDecision {
                    node_id: id.clone(),
                    title: entity.title.clone(),
                    paths,
                });
            }
        }
    }
    suggestions.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    suggestions
}

fn current_hashes(entities: &BTreeMap<EntityId, Entity>) -> BTreeMap<String, String> {
    entities
        .values()
        .filter(|entity| {
            matches!(
                entity.kind,
                EntityKind::Module | EntityKind::Symbol | EntityKind::Test | EntityKind::Document
            )
        })
        .flat_map(|entity| entity.evidence.iter())
        .filter_map(|evidence| match evidence {
            Evidence::Document { path, content_hash }
            | Evidence::SourceSpan {
                path, content_hash, ..
            } => Some((
                path.to_string_lossy().replace('\\', "/"),
                content_hash.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn stale_paths(entity: &Entity, current: &BTreeMap<String, String>) -> Vec<String> {
    entity
        .evidence
        .iter()
        .filter_map(|evidence| match evidence {
            Evidence::Document { path, content_hash }
            | Evidence::SourceSpan {
                path, content_hash, ..
            } => {
                let path = path.to_string_lossy().replace('\\', "/");
                current
                    .get(&path)
                    .filter(|hash| *hash != content_hash)
                    .map(|_| path)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{EntityPayload, Hash};

    fn id(value: u64) -> EntityId {
        EntityId::derived("mod", &[&format!("fixture-{value}")]).expect("id")
    }

    #[test]
    fn outcomes_are_bounded_and_only_adjust_retrieved_nodes() {
        let node = id(1);
        let records = [
            RetrievalRecord {
                task_id: None,
                node_id: node.clone(),
                signal: RetrievalSignal::Retrieved,
                at: time::OffsetDateTime::UNIX_EPOCH,
            },
            RetrievalRecord {
                task_id: None,
                node_id: node.clone(),
                signal: RetrievalSignal::FileOverlap,
                at: time::OffsetDateTime::UNIX_EPOCH,
            },
            RetrievalRecord {
                task_id: None,
                node_id: node.clone(),
                signal: RetrievalSignal::TestOverlap,
                at: time::OffsetDateTime::UNIX_EPOCH,
            },
            RetrievalRecord {
                task_id: None,
                node_id: id(2),
                signal: RetrievalSignal::UserAccepted,
                at: time::OffsetDateTime::UNIX_EPOCH,
            },
        ];
        assert_eq!(adjustments(&records, WeightConfig::default())[&node], 5);
        assert!(!adjustments(&records, WeightConfig::default()).contains_key(&id(2)));
    }

    #[test]
    fn report_explains_missing_docs_routing_and_stale_decisions() {
        let module = Entity::new(
            id(3),
            EntityKind::Module,
            Status::Active,
            "src/lib.rs".into(),
            EntityPayload::Module {
                path: "src/lib.rs".into(),
                language: None,
                responsibility: String::new(),
                public_surface: vec![],
            },
            vec![Evidence::Document {
                path: "src/lib.rs".into(),
                content_hash: "current".into(),
            }],
        )
        .expect("module");
        let decision = Entity::new(
            id(4),
            EntityKind::Decision,
            Status::Active,
            "Use local state".into(),
            EntityPayload::Decision {
                statement: "Use local state".into(),
                rationale: None,
                owner: None,
                supersedes: None,
            },
            vec![Evidence::SourceSpan {
                path: "src/lib.rs".into(),
                start_line: 1,
                end_line: 1,
                content_hash: Hash::genesis().to_hex(),
            }],
        )
        .expect("decision");
        let state = State {
            retrieval: vec![
                RetrievalRecord {
                    task_id: None,
                    node_id: module.id.clone(),
                    signal: RetrievalSignal::Retrieved,
                    at: time::OffsetDateTime::UNIX_EPOCH,
                },
                RetrievalRecord {
                    task_id: None,
                    node_id: module.id.clone(),
                    signal: RetrievalSignal::FileOverlap,
                    at: time::OffsetDateTime::UNIX_EPOCH,
                },
                RetrievalRecord {
                    task_id: None,
                    node_id: module.id.clone(),
                    signal: RetrievalSignal::FileOverlap,
                    at: time::OffsetDateTime::UNIX_EPOCH,
                },
                RetrievalRecord {
                    task_id: None,
                    node_id: module.id.clone(),
                    signal: RetrievalSignal::UserAccepted,
                    at: time::OffsetDateTime::UNIX_EPOCH,
                },
            ],
            ..State::default()
        };
        let entities =
            BTreeMap::from([(module.id.clone(), module), (decision.id.clone(), decision)]);
        let report = analyze(&state, &entities, &[], WeightConfig::default());
        assert!(
            report
                .maintenance_suggestions
                .iter()
                .any(|suggestion| matches!(
                    suggestion,
                    MaintenanceSuggestion::MissingDocumentation { .. }
                ))
        );
        assert!(
            report
                .maintenance_suggestions
                .iter()
                .any(|suggestion| matches!(suggestion, MaintenanceSuggestion::RoutingRule { .. }))
        );
        assert!(
            report
                .maintenance_suggestions
                .iter()
                .any(|suggestion| matches!(
                    suggestion,
                    MaintenanceSuggestion::StaleDecision { .. }
                ))
        );
        assert!(
            report
                .weight_suggestions
                .iter()
                .any(|suggestion| suggestion.field == "file_overlap")
        );
    }
}
