use std::collections::{BTreeMap, BTreeSet};

use middleman_core::{
    Entity, EntityId, EntityPayload, Status,
    routing::{Candidate, Ranking},
};

use crate::{Error, Format, Header, Item, Packet, Reference, estimate, render};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub items: usize,
    pub expandable: usize,
    pub budget: usize,
}

impl Limits {
    pub const fn for_format(format: Format) -> Self {
        Self {
            items: 24,
            expandable: 8,
            budget: format.default_budget(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub packet: Packet,
    pub text: String,
    pub estimated_tokens: usize,
}

pub fn prepare(
    header: Header,
    constraints: Vec<String>,
    ranking: &Ranking,
    entities: &BTreeMap<EntityId, Entity>,
    format: Format,
    limits: Limits,
) -> Result<Output, Error> {
    let mut packet = Packet {
        header,
        items: vec![],
        expandable: vec![],
        constraints,
    };
    packet.header.omitted = 0;
    packet.header.upstream_truncated = ranking.truncated;
    packet.header.low_confidence |= ranking.low_confidence;
    let mut candidates: Vec<_> = ranking.candidates.iter().collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    let mut seen = BTreeSet::new();

    for candidate in candidates {
        if !seen.insert(&candidate.id) {
            continue;
        }
        let Some(entity) = entities.get(&candidate.id).filter(|entity| {
            entity.status == Status::Active
                && entity.id == candidate.id
                && candidate.score > 0
                && !ranking.excluded.contains_key(&candidate.id)
        }) else {
            packet.header.omitted += 1;
            continue;
        };
        if packet.items.len() < limits.items {
            packet.items.push(item(entity, candidate));
        } else {
            packet.header.omitted += 1;
            if packet.expandable.len() < limits.expandable {
                packet.expandable.push(Reference {
                    id: entity.id.clone(),
                    title: entity.title.clone(),
                });
            }
        }
    }

    loop {
        packet.header.low_confidence |= !packet.items.iter().any(|item| {
            item.reasons.iter().any(|signal| {
                signal.weight() >= 70 && signal.weight() - i32::from(item.stale_penalty) >= 55
            })
        });
        let text = render(&packet, format)?;
        let estimated_tokens = estimate(&text, format);
        if estimated_tokens <= limits.budget {
            return Ok(Output {
                packet,
                text,
                estimated_tokens,
            });
        }
        if packet.expandable.pop().is_some() {
            continue;
        }
        if packet.items.pop().is_some() {
            packet.header.omitted += 1;
            continue;
        }
        return Err(Error::BudgetTooSmall {
            required: estimated_tokens,
            budget: limits.budget,
        });
    }
}

fn item(entity: &Entity, candidate: &Candidate) -> Item {
    let (summary, path, validation) = match &entity.payload {
        EntityPayload::Module {
            responsibility,
            path,
            ..
        } => (responsibility.clone(), Some(path), None),
        EntityPayload::Symbol { path, .. } | EntityPayload::Document { path, .. } => {
            (String::new(), Some(path), None)
        }
        EntityPayload::Test {
            path,
            scope,
            command,
            ..
        } => (scope.clone(), Some(path), Some(command.clone())),
        EntityPayload::Decision {
            statement,
            rationale,
            ..
        } => (
            format!("{statement}\n{}", rationale.as_deref().unwrap_or_default()),
            None,
            None,
        ),
        EntityPayload::Invariant {
            statement,
            consequence,
        } => (format!("{statement}\n{consequence}"), None, None),
        EntityPayload::Contract {
            what,
            input,
            output,
            compatibility,
            path,
            ..
        } => (
            format!(
                "{what}\nInput: {input}\nOutput: {output}\nCompatibility: {}",
                compatibility.as_deref().unwrap_or_default()
            ),
            path.as_ref(),
            None,
        ),
        EntityPayload::Risk {
            condition,
            severity,
            mitigation,
            ..
        } => (
            format!(
                "{severity:?}: {condition}\n{}",
                mitigation.as_deref().unwrap_or_default()
            ),
            None,
            None,
        ),
        EntityPayload::OpenQuestion { question, blocking } => {
            (format!("{question}\nBlocking: {blocking}"), None, None)
        }
        EntityPayload::Operation {
            runbook, recovery, ..
        } => (
            format!("{runbook}\n{}", recovery.as_deref().unwrap_or_default()),
            None,
            None,
        ),
    };
    Item {
        id: entity.id.clone(),
        kind: entity.kind,
        title: entity.title.clone(),
        summary,
        path: path
            .and_then(|path| path.to_str())
            .map(|path| path.replace('\\', "/")),
        score: candidate.score,
        reasons: candidate.signals.clone(),
        stale_penalty: candidate.stale_penalty,
        validation: validation.filter(|command| !command.trim().is_empty()),
    }
}
