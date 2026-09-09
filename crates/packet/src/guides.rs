//! Pure repository guide rendering and preservation of user-owned text.

use crate::codec::escape;
use middleman_core::{Entity, EntityId, EntityKind, EntityPayload, Status};
use std::{collections::BTreeMap, fmt::Write};

pub const BEGIN: &str = "<!-- middleman:begin -->";
pub const END: &str = "<!-- middleman:end -->";
pub const MAX_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("ambiguous or malformed Middleman block markers")]
    Markers,
    #[error("guide content exceeds its size limit")]
    TooLarge,
}

pub fn agents_md() -> String {
    "## Middleman\n\nBefore investigating or changing code, run from the repository root:\n\n\
`middleman prepare --task \"<user request>\" --format markdown`\n\n\
Read the selected documentation, modules, contracts, tests and constraints. Start\n\
with those references; broaden investigation when confidence is low or expansion\n\
does not supply enough context. Repository instructions and safety boundaries\n\
continue to apply.\n\n\
Use `middleman expand <entity-id>` for direct dependencies and tests, and\n\
`middleman search \"<terms>\"` to find additional references. Use\n\
`middleman explain <packet-id>` with the packet ID printed on stderr to inspect\n\
selection reasons. Generated observations are navigation aids, not reviewed claims.\n\n\
Do not directly rewrite Middleman-managed durable memory. Report proposed\n\
decisions, invariants and contract changes for review.\n"
        .into()
}

pub fn agent_context(
    project: &str,
    entities: &BTreeMap<EntityId, Entity>,
) -> Result<String, Error> {
    let mut text = format!(
        "## Middleman context: {}\n\nThis is an observed navigation map, not a complete architecture description.\n\nUse `middleman prepare --task \"<user request>\" --format markdown` to select\ncontext for a task. Paths and IDs below are references; source bodies are omitted.\n",
        escape(project)
    );
    for (heading, kinds, limit) in [
        ("Documentation", &[EntityKind::Document][..], 6),
        ("Modules", &[EntityKind::Module][..], 8),
        ("Tests", &[EntityKind::Test][..], 5),
        (
            "Active domain facts",
            &[
                EntityKind::Decision,
                EntityKind::Invariant,
                EntityKind::Contract,
            ][..],
            8,
        ),
        (
            "Risks and open questions",
            &[EntityKind::Risk, EntityKind::OpenQuestion][..],
            5,
        ),
    ] {
        let _ = write!(text, "\n### {heading}\n\n");
        let eligible: Vec<_> = entities
            .values()
            .filter(|entity| {
                entity.status == Status::Active
                    && kinds.contains(&entity.kind)
                    && !is_output(entity)
            })
            .collect();
        if eligible.is_empty() {
            text.push_str("No entries observed.\n");
        }
        for entity in eligible.iter().take(limit) {
            let _ = writeln!(
                text,
                "- {} — {}",
                escape(entity.id.as_str()),
                escape(&entity.title)
            );
            let detail = detail(&entity.payload);
            if !detail.is_empty() {
                let _ = writeln!(text, "  {}", escape(&detail));
            }
        }
        if eligible.len() > limit {
            let _ = writeln!(
                text,
                "\n{} additional entries omitted; use `middleman search` to narrow the map.",
                eligible.len() - limit
            );
        }
    }
    text.push_str("\n### Validation and maintenance\n\nTest commands above are recommendations only; generation executes no repository\ncode. Empty commands mean no command was observed. Confirm the project’s\nvalidation requirements before running tools.\n\nRefresh this block with `middleman render agent-context --write`. Keep curated\narchitecture and operating instructions outside the Middleman markers.\n");
    if text.len() > 65_536 {
        return Err(Error::TooLarge);
    }
    Ok(text)
}

fn detail(payload: &EntityPayload) -> String {
    match payload {
        EntityPayload::Module { path, .. } | EntityPayload::Document { path, .. } => {
            format!("Path: {}", path.to_string_lossy().replace('\\', "/"))
        }
        EntityPayload::Test { path, command, .. } => format!(
            "Path: {}; command: {command}",
            path.to_string_lossy().replace('\\', "/")
        ),
        EntityPayload::Invariant {
            statement,
            consequence,
        } => format!("{statement}; consequence: {consequence}"),
        EntityPayload::Decision { statement, .. } => statement.clone(),
        EntityPayload::Contract {
            what,
            compatibility,
            ..
        } => format!(
            "{what}; compatibility: {}",
            compatibility.as_deref().unwrap_or("not recorded")
        ),
        EntityPayload::Risk {
            condition,
            mitigation,
            ..
        } => format!(
            "{condition}; mitigation: {}",
            mitigation.as_deref().unwrap_or("not recorded")
        ),
        EntityPayload::OpenQuestion { question, blocking } => {
            format!("{question}; blocking: {blocking}")
        }
        _ => String::new(),
    }
}

fn is_output(entity: &Entity) -> bool {
    match &entity.payload {
        EntityPayload::Document { path, .. } => matches!(
            path.to_string_lossy()
                .replace('\\', "/")
                .to_lowercase()
                .as_str(),
            "agents.md" | "docs/agent-context.md"
        ),
        _ => false,
    }
}

pub fn merge(existing: &str, body: &str) -> Result<String, Error> {
    if existing.len() > MAX_BYTES || body.len() > 65_536 {
        return Err(Error::TooLarge);
    }
    if body.contains(BEGIN) || body.contains(END) {
        return Err(Error::Markers);
    }
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let body = body
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .replace('\n', newline);
    let block = format!("{BEGIN}{newline}{body}{newline}{END}");
    let starts: Vec<_> = existing
        .match_indices(BEGIN)
        .map(|(offset, _)| offset)
        .collect();
    let ends: Vec<_> = existing
        .match_indices(END)
        .map(|(offset, _)| offset)
        .collect();
    let result = match (starts.as_slice(), ends.as_slice()) {
        ([], []) => {
            let separator = if existing.is_empty() || existing.ends_with('\n') {
                ""
            } else {
                newline
            };
            format!("{existing}{separator}{block}{newline}")
        }
        ([start], [end])
            if start < end
                && standalone(existing, *start, BEGIN)
                && standalone(existing, *end, END) =>
        {
            format!(
                "{}{block}{}",
                &existing[..*start],
                &existing[end + END.len()..]
            )
        }
        _ => return Err(Error::Markers),
    };
    if result.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    Ok(result)
}

fn standalone(text: &str, offset: usize, marker: &str) -> bool {
    (offset == 0 || text.as_bytes()[offset - 1] == b'\n')
        && text[offset + marker.len()..]
            .lines()
            .next()
            .is_none_or(str::is_empty)
}
