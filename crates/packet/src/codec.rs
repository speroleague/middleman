use std::collections::BTreeSet;
use std::fmt::Write;

use serde::Serialize;

use crate::{Error, Format, Header, Packet};

pub fn estimate(text: &str, format: Format) -> usize {
    match format {
        Format::Cir => text.split_whitespace().count(),
        Format::Markdown | Format::Json => text.chars().count().div_ceil(4),
    }
}

pub fn render(packet: &Packet, format: Format) -> Result<String, Error> {
    match format {
        Format::Json => Ok(serde_json::to_string_pretty(packet)?),
        Format::Cir => cir(packet),
        Format::Markdown => markdown(packet),
    }
}

fn statement<T: Serialize>(text: &mut String, keyword: &str, value: &T) -> Result<(), Error> {
    text.push_str(keyword);
    text.push(' ');
    text.push_str(&serde_json::to_string(value)?);
    text.push('\n');
    Ok(())
}

fn cir(packet: &Packet) -> Result<String, Error> {
    let mut text = String::from("CIR/1\n");
    statement(&mut text, "P", &packet.header)?;
    for item in &packet.items {
        statement(&mut text, "R", item)?;
    }
    for reference in &packet.expandable {
        statement(&mut text, "E", reference)?;
    }
    for constraint in &packet.constraints {
        statement(&mut text, "X", constraint)?;
    }
    Ok(text)
}

/// Parses the canonical JSON-valued CIR/1 dialect. Input is capped at 1 MiB.
pub fn parse_cir(text: &str) -> Result<Packet, Error> {
    if text.len() > 1_048_576 {
        return Err(Error::InvalidCir);
    }
    let mut lines = text.lines();
    if lines.next() != Some("CIR/1") {
        return Err(Error::InvalidCir);
    }
    let header: Header = serde_json::from_str(
        lines
            .next()
            .and_then(|line| line.strip_prefix("P "))
            .ok_or(Error::InvalidCir)?,
    )?;
    let mut packet = Packet {
        header,
        items: vec![],
        expandable: vec![],
        constraints: vec![],
    };
    for line in lines {
        let (key, value) = line.split_once(' ').ok_or(Error::InvalidCir)?;
        match key {
            "R" => packet.items.push(serde_json::from_str(value)?),
            "E" => packet.expandable.push(serde_json::from_str(value)?),
            "X" => packet.constraints.push(serde_json::from_str(value)?),
            _ => return Err(Error::InvalidCir),
        }
    }
    let mut ids = BTreeSet::new();
    if packet
        .items
        .iter()
        .map(|item| &item.id)
        .chain(packet.expandable.iter().map(|item| &item.id))
        .any(|id| !ids.insert(id))
    {
        return Err(Error::InvalidCir);
    }
    Ok(packet)
}

fn markdown(packet: &Packet) -> Result<String, Error> {
    let mut text = format!(
        "# Context: {}\n\nGoal: {}\n\nLow confidence: {}\n\nOmitted: {}; upstream truncated: {}\n",
        escape(&packet.header.project),
        escape(&packet.header.goal),
        packet.header.low_confidence,
        packet.header.omitted,
        packet.header.upstream_truncated
    );
    if let Some(task) = &packet.header.task {
        let _ = write!(text, "\nTask: {}\n", escape(task.as_str()));
    }
    text.push_str("\n## Constraints\n");
    for constraint in &packet.constraints {
        let _ = write!(text, "\n- {}\n", escape(constraint));
    }
    text.push_str("\n## Selected context\n");
    for item in &packet.items {
        let _ = write!(
            text,
            "\n### {}\n\nID: {} ({})\n\n{}\n\nScore: {}; stale penalty: {}; reasons: {}\n",
            escape(&item.title),
            escape(item.id.as_str()),
            item.kind.as_str(),
            escape(&item.summary),
            item.score,
            item.stale_penalty,
            escape(&serde_json::to_string(&item.reasons)?)
        );
        if let Some(path) = &item.path {
            let _ = write!(text, "\nSource: {}\n", escape(path));
        }
    }
    text.push_str("\n## Recommended validation\n");
    for command in packet.validation_commands() {
        let _ = write!(text, "\n- {}\n", escape(command));
    }
    text.push_str("\n## Expandable\n");
    for reference in &packet.expandable {
        let _ = write!(
            text,
            "\n- {}: {}\n",
            escape(reference.id.as_str()),
            escape(&reference.title)
        );
    }
    Ok(text)
}

fn escape(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_control() {
            result.extend(ch.escape_default());
        } else if matches!(
            ch,
            '\\' | '`'
                | '*'
                | '_'
                | '{'
                | '}'
                | '['
                | ']'
                | '('
                | ')'
                | '#'
                | '+'
                | '-'
                | '.'
                | '!'
                | '|'
                | '>'
                | '<'
                | '&'
        ) {
            result.push('\\');
            result.push(ch);
        } else {
            result.push(ch);
        }
    }
    result
}
