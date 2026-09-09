//! Lightweight declaration hints, not semantic resolution or executable parsing.

use std::collections::BTreeSet;
use std::path::Path;

use middleman_core::config::LimitsConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Php,
    Elm,
    TypeScript,
    JavaScript,
    Python,
    Go,
}

impl Language {
    pub fn for_path(path: &Path) -> Option<Self> {
        Some(match path.extension()?.to_str()? {
            "rs" => Self::Rust,
            "php" => Self::Php,
            "elm" => Self::Elm,
            "ts" | "tsx" => Self::TypeScript,
            "js" | "jsx" => Self::JavaScript,
            "py" => Self::Python,
            "go" => Self::Go,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub line: usize,
    pub visibility: Visibility,
    pub is_test: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub target: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageIndex {
    pub language: Language,
    pub module_name: Option<String>,
    pub declarations: Vec<Declaration>,
    pub imports: Vec<Import>,
    pub exports: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("source exceeds byte or line limits")]
    LimitExceeded,
    #[error("source contains binary content")]
    Binary,
}

pub fn parse(
    language: Language,
    text: &str,
    limits: &LimitsConfig,
) -> Result<LanguageIndex, Error> {
    if text.len() as u64 > u64::from(limits.max_file_kb) * 1024
        || text.lines().count() as u64 > limits.max_lines
    {
        return Err(Error::LimitExceeded);
    }
    if text.contains('\0') {
        return Err(Error::Binary);
    }
    let masked = mask(text, language);
    let mut result = LanguageIndex {
        language,
        module_name: None,
        declarations: Vec::new(),
        imports: Vec::new(),
        exports: Vec::new(),
    };
    let mut elm_names = BTreeSet::new();
    let mut exports = BTreeSet::new();
    let mut test_attribute = false;
    let mut go_imports = false;
    for (index, (raw, masked_line)) in text.lines().zip(masked.lines()).enumerate() {
        let line = masked_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = module_name(language, line) {
            result.module_name = Some(name);
        }
        if language == Language::Rust && (line == "#[test]" || line.ends_with("::test]")) {
            test_attribute = true;
            continue;
        }
        let line_number = index + 1;
        let tokens: Vec<_> = line
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|s| !s.is_empty())
            .collect();
        if language == Language::Elm && line.starts_with("module ") {
            if let Some((_, exposed)) = line.split_once("exposing") {
                exports.extend(
                    exposed
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                );
                if exposed.trim() == "(..)" {
                    exports.insert("*".into());
                }
            }
        }
        if language == Language::Go && line.starts_with("import (") {
            go_imports = true;
        }
        let import = import_target(language, line, raw, go_imports);
        if let Some(target) = import {
            result.imports.push(Import {
                target,
                line: line_number,
            });
        }
        if go_imports && line == ")" {
            go_imports = false;
        }
        if let Some((kind, name)) = declaration(language, line, &tokens) {
            if language == Language::Elm && !elm_names.insert(name.to_owned()) {
                continue;
            }
            let visibility = visibility(language, name, &tokens, &exports);
            if visibility == Visibility::Public {
                let export = if matches!(language, Language::TypeScript | Language::JavaScript)
                    && tokens.contains(&"default")
                {
                    "default"
                } else {
                    name
                };
                exports.insert(export.to_owned());
            }
            let is_test = test_attribute
                || (matches!(language, Language::Python | Language::Php)
                    && name.starts_with("test"))
                || (language == Language::Go && name.starts_with("Test"));
            let signature = line.split('{').next().unwrap_or(line).trim_end().to_owned();
            result.declarations.push(Declaration {
                name: name.to_owned(),
                kind: kind.into(),
                signature,
                line: line_number,
                visibility,
                is_test,
            });
        }
        if !line.starts_with("#[") {
            test_attribute = false;
        }
    }
    result.exports = exports.into_iter().collect();
    Ok(result)
}

fn module_name(language: Language, line: &str) -> Option<String> {
    match language {
        Language::Php => line
            .strip_prefix("namespace ")
            .and_then(|s| s.strip_suffix(';'))
            .map(|s| s.trim().to_owned()),
        Language::Elm => line
            .strip_prefix("module ")
            .and_then(|s| s.split_whitespace().next())
            .map(str::to_owned),
        _ => None,
    }
}

fn declaration<'a>(
    language: Language,
    line: &str,
    words: &[&'a str],
) -> Option<(&'static str, &'a str)> {
    let mut position = 0;
    while language != Language::Elm
        && words.get(position).is_some_and(|word| {
            matches!(
                *word,
                "pub"
                    | "public"
                    | "private"
                    | "protected"
                    | "async"
                    | "unsafe"
                    | "export"
                    | "default"
                    | "abstract"
                    | "final"
                    | "static"
                    | "declare"
            )
        })
    {
        position += 1;
    }
    if language == Language::Rust && line.starts_with("pub(") {
        let end = line.find(')')?;
        position = line[..end]
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|s| !s.is_empty())
            .count();
    }
    let keyword = *words.get(position)?;
    let next = *words.get(position + 1).unwrap_or(&"");
    let kind = match (language, keyword) {
        (Language::Rust, "fn")
        | (Language::Php | Language::TypeScript | Language::JavaScript, "function")
        | (Language::Python, "def") => "function",
        (Language::Go, "func") => {
            if let Some(rest) = line.strip_prefix("func (") {
                let after = rest.split_once(')')?.1.trim_start();
                let name = after.split('(').next()?.trim();
                let found = words.iter().find(|word| **word == name)?;
                return Some(("function", found));
            }
            "function"
        }
        (Language::Rust, "struct") => "struct",
        (Language::Rust | Language::Php | Language::TypeScript, "enum") => "enum",
        (Language::Rust | Language::Php, "trait") => "trait",
        (Language::Rust, "mod") | (Language::Elm, "module") => "module",
        (
            Language::Python | Language::Php | Language::TypeScript | Language::JavaScript,
            "class",
        ) => "class",
        (Language::Php | Language::TypeScript, "interface") => "interface",
        (Language::Rust | Language::Go | Language::TypeScript | Language::Elm, "type") => {
            if language == Language::Elm && next == "alias" {
                return Some(("type", *words.get(position + 2)?));
            }
            "type"
        }
        (Language::Rust | Language::Go | Language::TypeScript | Language::JavaScript, "const") => {
            "constant"
        }
        (Language::Go | Language::TypeScript | Language::JavaScript, "var" | "let") => "variable",
        (Language::Elm, _) if keyword.chars().next()?.is_lowercase() && line.contains(':') => {
            return Some(("function", keyword));
        }
        _ => return None,
    };
    (!next.is_empty()
        && next
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_'))
    .then_some((kind, next))
}

fn visibility(
    language: Language,
    name: &str,
    words: &[&str],
    exports: &BTreeSet<String>,
) -> Visibility {
    if matches!(
        language,
        Language::Php | Language::TypeScript | Language::JavaScript
    ) && words.first() == Some(&"protected")
    {
        return Visibility::Protected;
    }
    if matches!(
        language,
        Language::Php | Language::TypeScript | Language::JavaScript
    ) && words.first() == Some(&"private")
    {
        return Visibility::Private;
    }
    match language {
        Language::Rust
            if words.first() == Some(&"pub")
                && matches!(words.get(1), Some(&"crate" | &"super" | &"in" | &"self")) =>
        {
            Visibility::Internal
        }
        Language::Rust if words.contains(&"pub") => Visibility::Public,
        Language::Php if words.contains(&"public") || words.first() == Some(&"class") => {
            Visibility::Public
        }
        Language::Go if name.starts_with(char::is_uppercase) => Visibility::Public,
        Language::Elm if exports.contains(name) || exports.contains("*") => Visibility::Public,
        Language::Python if !name.starts_with('_') => Visibility::Public,
        Language::TypeScript | Language::JavaScript if words.contains(&"export") => {
            Visibility::Public
        }
        _ => Visibility::Internal,
    }
}

fn import_target(language: Language, line: &str, raw: &str, go_group: bool) -> Option<String> {
    let rest = match language {
        Language::Rust => line
            .strip_prefix("use ")
            .or_else(|| line.strip_prefix("pub use "))?,
        Language::Php => line.strip_prefix("use ")?,
        Language::Elm => {
            return Some(
                line.strip_prefix("import ")?
                    .split_whitespace()
                    .next()?
                    .to_owned(),
            );
        }
        Language::Python => {
            return Some(
                line.strip_prefix("from ")
                    .or_else(|| line.strip_prefix("import "))?
                    .split_whitespace()
                    .next()?
                    .trim_end_matches(',')
                    .to_owned(),
            );
        }
        Language::Go if line.starts_with("import ") || go_group => return quoted(raw),
        Language::TypeScript | Language::JavaScript
            if line.starts_with("import ")
                || line.starts_with("export ") && line.contains(" from ") =>
        {
            return quoted(raw);
        }
        _ => return None,
    };
    Some(rest.trim_end_matches(';').trim().to_owned())
}

fn quoted(raw: &str) -> Option<String> {
    let start = raw.find(['\'', '"'])?;
    let marker = raw.as_bytes()[start] as char;
    let rest = &raw[start + 1..];
    let end = rest.find(marker)?;
    Some(rest[..end].to_owned())
}

fn mask(text: &str, language: Language) -> String {
    let input = text.as_bytes();
    let mut output = input.to_vec();
    let mut index = 0;
    let mut block_depth = 0;
    let mut closing: Vec<u8> = Vec::new();
    let mut escaped = false;
    let (block_start, block_end): (&[u8], &[u8]) = if language == Language::Elm {
        (b"{-", b"-}")
    } else {
        (b"/*", b"*/")
    };
    while index < input.len() {
        let tail = &input[index..];
        let mut count = 1;
        let mut hide = false;
        let mut opening_quote = false;
        if !closing.is_empty() {
            hide = true;
            if escaped {
                escaped = false;
            } else if tail.starts_with(&closing) {
                count = closing.len();
                closing.clear();
            } else if input[index] == b'\\' && closing.len() == 1 {
                escaped = true;
            }
        } else if block_depth > 0 {
            hide = true;
            if tail.starts_with(block_start) {
                block_depth += 1;
                count = 2;
            } else if tail.starts_with(block_end) {
                block_depth -= 1;
                count = 2;
            }
        } else if tail.starts_with(block_start) && language != Language::Python {
            block_depth = 1;
            count = 2;
            hide = true;
        } else if (language == Language::Python && tail.starts_with(b"#"))
            || (language == Language::Elm && tail.starts_with(b"--"))
            || (!matches!(language, Language::Python | Language::Elm) && tail.starts_with(b"//"))
        {
            count = tail.iter().position(|b| *b == b'\n').unwrap_or(tail.len());
            hide = true;
        } else if language == Language::Python
            && (tail.starts_with(b"\"\"\"") || tail.starts_with(b"'''"))
        {
            closing = tail[..3].to_vec();
            count = 3;
            hide = true;
            opening_quote = true;
        } else if language == Language::Rust && tail.starts_with(b"r") {
            let hashes = tail[1..].iter().take_while(|b| **b == b'#').count();
            if tail.get(hashes + 1) == Some(&b'"') {
                closing = vec![b'"'];
                closing.extend(std::iter::repeat_n(b'#', hashes));
                count = hashes + 2;
                hide = true;
            }
        } else if matches!(input[index], b'"' | b'`')
            || (input[index] == b'\'' && language != Language::Rust)
        {
            closing = vec![input[index]];
            hide = true;
            opening_quote = true;
        }
        if hide {
            for byte in &mut output[index..index + count] {
                if !matches!(*byte, b'\n' | b'\r') {
                    *byte = b' ';
                }
            }
        }
        if opening_quote {
            output[index] = input[index];
        }
        index += count;
    }
    String::from_utf8(output).unwrap_or_default()
}
