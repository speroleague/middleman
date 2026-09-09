use std::collections::BTreeSet;

use middleman_core::{EntityId, EntityKind, TaskId, routing::Signal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Cir,
    Markdown,
    Json,
}

impl Format {
    pub const fn default_budget(self) -> usize {
        match self {
            Self::Cir => 1200,
            Self::Markdown | Self::Json => 2000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub project: String,
    pub task: Option<TaskId>,
    pub goal: String,
    pub low_confidence: bool,
    pub omitted: usize,
    pub upstream_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub id: EntityId,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: EntityId,
    pub kind: EntityKind,
    pub title: String,
    pub summary: String,
    pub path: Option<String>,
    pub score: i32,
    pub reasons: BTreeSet<Signal>,
    pub stale_penalty: u16,
    pub validation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    pub header: Header,
    pub items: Vec<Item>,
    pub expandable: Vec<Reference>,
    pub constraints: Vec<String>,
}

impl Packet {
    pub fn validation_commands(&self) -> BTreeSet<&str> {
        self.items
            .iter()
            .filter_map(|item| item.validation.as_deref())
            .filter(|command| !command.trim().is_empty())
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("packet serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid or unsupported CIR packet")]
    InvalidCir,
    #[error("required packet metadata needs {required} estimated tokens; budget is {budget}")]
    BudgetTooSmall { required: usize, budget: usize },
}
