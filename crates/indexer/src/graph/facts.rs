use std::path::{Path, PathBuf};

use middleman_core::config::LimitsConfig;
use middleman_core::{
    Edge, EdgeKind, Entity, EntityId, EntityKind, EntityPayload, Evidence, Hash, Status,
};
use serde::{Deserialize, Serialize};

use super::{Budget, Error, path_key, resolve::Resolution};
use crate::{
    document,
    language::{self, Language, Visibility},
    scan::{FileKind, SourceFile},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Package {
    pub name: String,
    pub library: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Facts {
    pub hash: Hash,
    pub kind: FileKind,
    pub lines: u32,
    pub language: Option<language::LanguageIndex>,
    pub document: Option<document::DocumentIndex>,
    pub package: Option<Package>,
}

impl Facts {
    pub fn parse(file: &SourceFile, limits: &LimitsConfig) -> Result<Self, Error> {
        let language = Language::for_path(&file.path)
            .map(|lang| language::parse(lang, &file.content, limits))
            .transpose()?;
        let document = (file.kind == FileKind::Document)
            .then(|| document::parse_document(&file.content, limits))
            .transpose()?;
        let package = if file
            .path
            .file_name()
            .is_some_and(|name| name == "Cargo.toml")
        {
            let table = file
                .content
                .parse::<toml::Table>()
                .map_err(|_| Error::Manifest)?;
            let package_name = table
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str);
            package_name.map(|name| Package {
                name: table
                    .get("lib")
                    .and_then(|lib| lib.get("name"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or(name)
                    .replace('-', "_"),
                library: table
                    .get("lib")
                    .and_then(|lib| lib.get("path"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or("src/lib.rs")
                    .into(),
            })
        } else {
            None
        };
        Ok(Self {
            hash: file.hash,
            kind: file.kind,
            lines: u32::try_from(file.content.lines().count())
                .map_err(|_| Error::Budget("lines"))?,
            language,
            document,
            package,
        })
    }

    pub fn evidence(&self, path: &Path, line: u32) -> Evidence {
        if self.lines == 0 {
            return Evidence::Document {
                path: path.into(),
                content_hash: self.hash.to_hex(),
            };
        }
        Evidence::SourceSpan {
            path: path.into(),
            start_line: line,
            end_line: line.max(1),
            content_hash: self.hash.to_hex(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Fragment {
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
}

pub(super) fn file_id(path: &Path, kind: FileKind) -> Result<EntityId, Error> {
    Ok(EntityId::derived(
        if kind == FileKind::Document {
            "doc"
        } else {
            "mod"
        },
        &[&path_key(path)?],
    )?)
}

pub(super) fn derive(
    path: &Path,
    facts: &Facts,
    resolution: &Resolution,
    budget: Budget,
) -> Result<Fragment, Error> {
    let parent = file_id(path, facts.kind)?;
    let mut fragment = Fragment {
        entities: vec![primary_entity(path, facts)?],
        edges: Vec::new(),
    };
    let tests = declarations(&mut fragment, &parent, path, facts, budget)?;
    for link in &resolution.links {
        if fragment
            .edges
            .len()
            .saturating_add(tests.len())
            .saturating_add(1)
            > budget.max_edges
        {
            return Err(Error::Budget("edges"));
        }
        let evidence = facts.evidence(path, link.line);
        fragment.edges.push(Edge {
            from: parent.clone(),
            to: link.id.clone(),
            kind: link.kind,
            evidence: vec![evidence.clone()],
        });
        if link.kind == EdgeKind::Imports {
            for test in &tests {
                fragment.edges.push(Edge {
                    from: test.clone(),
                    to: link.id.clone(),
                    kind: EdgeKind::Tests,
                    evidence: vec![evidence.clone()],
                });
            }
        }
    }
    Ok(fragment)
}

fn primary_entity(path: &Path, facts: &Facts) -> Result<Entity, Error> {
    let key = path_key(path)?;
    let (kind, title, payload, evidence) = if let Some(doc) = &facts.document {
        let title = doc
            .title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&key)
            .to_owned();
        (
            EntityKind::Document,
            title.clone(),
            EntityPayload::Document {
                path: path.into(),
                title,
                kind: "document".into(),
                authority: "observed".into(),
            },
            Evidence::Document {
                path: path.into(),
                content_hash: facts.hash.to_hex(),
            },
        )
    } else {
        (
            EntityKind::Module,
            key.clone(),
            EntityPayload::Module {
                path: path.into(),
                language: facts
                    .language
                    .as_ref()
                    .map(|l| language_name(l.language).into()),
                responsibility: String::new(),
                public_surface: facts
                    .language
                    .as_ref()
                    .map_or_else(Vec::new, |l| l.exports.clone()),
            },
            Evidence::Document {
                path: path.into(),
                content_hash: facts.hash.to_hex(),
            },
        )
    };
    Ok(Entity::new(
        file_id(path, facts.kind)?,
        kind,
        Status::Active,
        title,
        payload,
        vec![evidence],
    )?)
}

fn declarations(
    fragment: &mut Fragment,
    parent: &EntityId,
    path: &Path,
    facts: &Facts,
    budget: Budget,
) -> Result<Vec<EntityId>, Error> {
    let key = path_key(path)?;
    let mut tests = Vec::new();
    if facts.kind == FileKind::Test {
        let id = EntityId::derived("test", &[&key])?;
        add_test(fragment, &mut tests, id, parent, path, &key, facts, 1)?;
    }
    let mut occurrences = std::collections::BTreeMap::new();
    if let Some(language) = &facts.language {
        for declaration in &language.declarations {
            if fragment
                .entities
                .len()
                .saturating_add(1 + usize::from(declaration.is_test))
                > budget.max_nodes
            {
                return Err(Error::Budget("nodes"));
            }
            let occurrence = occurrences
                .entry((&declaration.kind, &declaration.name))
                .or_insert(0u32);
            *occurrence += 1;
            let parts = [
                &key,
                declaration.kind.as_str(),
                declaration.name.as_str(),
                &occurrence.to_string(),
            ];
            let id = EntityId::derived("sym", &parts)?;
            let line = u32::try_from(declaration.line).map_err(|_| Error::Budget("lines"))?;
            fragment.entities.push(Entity::new(
                id.clone(),
                EntityKind::Symbol,
                Status::Active,
                declaration.name.clone(),
                EntityPayload::Symbol {
                    path: path.into(),
                    span: Some(format!("L{line}")),
                    symbol_kind: declaration.kind.clone(),
                    signature: Some(declaration.signature.clone()),
                    visibility: match declaration.visibility {
                        Visibility::Public => "public",
                        Visibility::Private => "private",
                        Visibility::Protected => "protected",
                        Visibility::Internal => "internal",
                    }
                    .into(),
                },
                vec![facts.evidence(path, line)],
            )?);
            fragment.edges.push(Edge {
                from: parent.clone(),
                to: id,
                kind: EdgeKind::Owns,
                evidence: vec![facts.evidence(path, line)],
            });
            if declaration.is_test {
                add_test(
                    fragment,
                    &mut tests,
                    EntityId::derived("test", &parts)?,
                    parent,
                    path,
                    &declaration.name,
                    facts,
                    line,
                )?;
            }
        }
    }
    Ok(tests)
}

fn add_test(
    fragment: &mut Fragment,
    tests: &mut Vec<EntityId>,
    id: EntityId,
    parent: &EntityId,
    path: &Path,
    title: &str,
    facts: &Facts,
    line: u32,
) -> Result<(), Error> {
    fragment.entities.push(Entity::new(
        id.clone(),
        EntityKind::Test,
        Status::Active,
        title.into(),
        EntityPayload::Test {
            path: path.into(),
            scope: path_key(path)?,
            command: String::new(),
            covered: Vec::new(),
        },
        vec![facts.evidence(path, line)],
    )?);
    fragment.edges.push(Edge {
        from: parent.clone(),
        to: id.clone(),
        kind: EdgeKind::Owns,
        evidence: vec![facts.evidence(path, line)],
    });
    tests.push(id);
    Ok(())
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "rust",
        Language::Php => "php",
        Language::Elm => "elm",
        Language::TypeScript => "typescript",
        Language::JavaScript => "javascript",
        Language::Python => "python",
        Language::Go => "go",
    }
}
