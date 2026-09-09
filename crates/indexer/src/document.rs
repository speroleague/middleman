//! Pure extraction of a conservative Markdown subset; no link resolution or I/O.

use std::collections::{BTreeMap, BTreeSet};

use middleman_core::config::LimitsConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub label: String,
    pub target: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingRule {
    pub condition: String,
    pub references: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentIndex {
    pub title: Option<String>,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    pub frontmatter: BTreeMap<String, String>,
    pub identifiers: Vec<String>,
    pub routing: Vec<RoutingRule>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DocumentError {
    #[error("document exceeds byte or line limits")]
    LimitExceeded,
    #[error("document contains binary content")]
    Binary,
}

pub fn parse_document(text: &str, limits: &LimitsConfig) -> Result<DocumentIndex, DocumentError> {
    if text.len() as u64 > u64::from(limits.max_file_kb) * 1024
        || text.lines().count() as u64 > limits.max_lines
    {
        return Err(DocumentError::LimitExceeded);
    }
    if text.contains('\0') {
        return Err(DocumentError::Binary);
    }
    let mut result = DocumentIndex::default();
    let mut identifiers = BTreeSet::new();
    let lines: Vec<_> = text.lines().collect();
    let start = frontmatter(&lines, &mut result.frontmatter);
    let mut fence: Option<(char, usize)> = None;
    let mut routing_table = false;
    let mut pending_header = false;

    for (index, raw) in lines.iter().enumerate().skip(start) {
        let line_number = index + 1;
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();
        if let Some((marker, count)) = fence {
            let closing = trimmed.chars().take_while(|c| *c == marker).count();
            if indent <= 3 && closing >= count && trimmed[closing..].trim().is_empty() {
                fence = None;
            }
            continue;
        }
        if indent >= 4 || raw.starts_with('\t') {
            continue;
        }
        let line = trimmed.trim_end();
        if let Some(marker @ ('`' | '~')) = line.chars().next() {
            let count = line.chars().take_while(|c| *c == marker).count();
            if count >= 3 {
                fence = Some((marker, count));
                routing_table = false;
                pending_header = false;
                continue;
            }
        }
        if let Some(heading) = heading(line, line_number) {
            if result.title.is_none() && heading.level == 1 {
                result.title = Some(heading.text.clone());
            }
            result.headings.push(heading);
        }
        let references = inline_links(line, line_number);
        for code in code_spans(line) {
            if is_identifier(code) {
                identifiers.insert(code.to_owned());
            }
        }
        let cells = table_cells(line);
        if cells.len() >= 2 {
            if pending_header && cells.iter().all(|cell| table_separator(cell)) {
                routing_table = true;
                pending_header = false;
            } else if routing_table {
                let mut references = Vec::new();
                for cell in &cells[1..] {
                    references.extend(code_spans(cell).into_iter().map(str::to_owned));
                    references.extend(
                        inline_links(cell, line_number)
                            .into_iter()
                            .map(|link| link.target),
                    );
                }
                let mut seen = BTreeSet::new();
                references.retain(|value| seen.insert(value.clone()));
                if !references.is_empty() && !cells[0].is_empty() {
                    result.routing.push(RoutingRule {
                        condition: cells[0].to_owned(),
                        references,
                        line: line_number,
                    });
                }
            } else {
                let first = cells[0].to_ascii_lowercase();
                pending_header = (first.contains("task")
                    || first == "when"
                    || first == "condition")
                    && cells[1..].iter().any(|cell| {
                        let cell = cell.to_ascii_lowercase();
                        cell.contains("read") || cell.contains("inspect") || cell.contains("path")
                    });
            }
        } else {
            routing_table = false;
            pending_header = false;
        }
        result.links.extend(references);
    }
    if let Some(id) = result.frontmatter.get("id") {
        identifiers.insert(id.clone());
    }
    result.identifiers = identifiers.into_iter().collect();
    Ok(result)
}

fn frontmatter(lines: &[&str], fields: &mut BTreeMap<String, String>) -> usize {
    if lines.first().map(|line| line.trim()) != Some("---") {
        return 0;
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, line)| (line.trim() == "---").then_some(index))
    else {
        return 0;
    };
    for line in &lines[1..end] {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                fields.insert(key.to_owned(), value.trim().to_owned());
            }
        }
    }
    end + 1
}

fn heading(line: &str, number: usize) -> Option<Heading> {
    let level = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut title = rest.trim();
    let without_hashes = title.trim_end_matches('#');
    if without_hashes.ends_with(char::is_whitespace) {
        title = without_hashes.trim_end();
    }
    Some(Heading {
        level: u8::try_from(level).ok()?,
        text: title.to_owned(),
        line: number,
    })
}

fn inline_links(mut text: &str, line: usize) -> Vec<Link> {
    let mut links = Vec::new();
    while let Some(start) = text.find('[') {
        text = &text[start + 1..];
        let Some(middle) = text.find("](") else { break };
        let label = &text[..middle];
        let rest = &text[middle + 2..];
        let Some(end) = rest.find(')') else { break };
        let target = rest[..end].trim();
        if !target.is_empty() {
            links.push(Link {
                label: label.to_owned(),
                target: target.to_owned(),
                line,
            });
        }
        text = &rest[end + 1..];
    }
    links
}

fn code_spans(text: &str) -> Vec<&str> {
    let mut remaining = text;
    let mut spans = Vec::new();
    while let Some(start) = remaining.find('`') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('`') else {
            break;
        };
        if end > 0 {
            spans.push(&remaining[..end]);
        }
        remaining = &remaining[end + 1..];
    }
    spans
}

fn is_identifier(value: &str) -> bool {
    let Some((prefix, name)) = value.split_once(':') else {
        return false;
    };
    matches!(
        prefix,
        "mod" | "sym" | "test" | "doc" | "dec" | "inv" | "con" | "task" | "risk" | "q" | "op"
    ) && !name.is_empty()
        && !name.chars().any(char::is_whitespace)
}

fn table_cells(line: &str) -> Vec<&str> {
    if !line.contains('|') {
        return Vec::new();
    }
    line.trim_matches('|').split('|').map(str::trim).collect()
}

fn table_separator(cell: &str) -> bool {
    let value = cell.trim_matches(':');
    value.len() >= 3 && value.chars().all(|c| c == '-')
}
