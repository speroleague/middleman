use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use middleman_core::{EdgeKind, EntityId};

use super::{
    Diagnostic, Error, Reason,
    facts::{Facts, file_id},
};
use crate::language::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Link {
    pub path: PathBuf,
    pub id: EntityId,
    pub kind: EdgeKind,
    pub line: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Resolution {
    pub links: Vec<Link>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) struct Catalog<'a> {
    files: &'a BTreeMap<PathBuf, Arc<Facts>>,
    aliases: BTreeMap<String, BTreeSet<PathBuf>>,
}

impl<'a> Catalog<'a> {
    pub fn new(files: &'a BTreeMap<PathBuf, Arc<Facts>>) -> Self {
        let mut aliases: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
        for (path, facts) in files {
            if let Some(package) = &facts.package {
                if let Some(library) = normalize(
                    path.parent().unwrap_or(Path::new("")),
                    &package.library.to_string_lossy(),
                ) {
                    aliases
                        .entry(format!("rust:{}", package.name))
                        .or_default()
                        .insert(library);
                }
            }
            if let Some(language) = &facts.language {
                if let Some(name) = &language.module_name {
                    if language.language == Language::Elm {
                        aliases
                            .entry(format!("elm:{name}"))
                            .or_default()
                            .insert(path.clone());
                    } else if language.language == Language::Php {
                        for declaration in &language.declarations {
                            if matches!(
                                declaration.kind.as_str(),
                                "class" | "interface" | "trait" | "enum"
                            ) {
                                aliases
                                    .entry(format!("php:{name}\\{}", declaration.name))
                                    .or_default()
                                    .insert(path.clone());
                            }
                        }
                    }
                }
            }
        }
        Self { files, aliases }
    }

    pub fn resolve(&self, source: &Path, facts: &Facts) -> Result<Resolution, Error> {
        let mut output = Resolution::default();
        if let Some(document) = &facts.document {
            for link in &document.links {
                let target = link.target.split('#').next().unwrap_or("");
                let candidate = if target.is_empty() {
                    Some(source.into())
                } else {
                    normalize(source.parent().unwrap_or(Path::new("")), target)
                };
                self.link(
                    source,
                    link.line,
                    EdgeKind::Documents,
                    vec![candidate.into_iter().collect()],
                    &mut output,
                )?;
            }
            for rule in &document.routing {
                for reference in &rule.references {
                    let target = reference.split('#').next().unwrap_or("");
                    self.link(
                        source,
                        rule.line,
                        EdgeKind::Documents,
                        vec![normalize(Path::new(""), target).into_iter().collect()],
                        &mut output,
                    )?;
                }
            }
        }
        if let Some(language) = &facts.language {
            for import in &language.imports {
                let candidates = match language.language {
                    Language::TypeScript | Language::JavaScript
                        if import.target.starts_with('.') =>
                    {
                        let base =
                            normalize(source.parent().unwrap_or(Path::new("")), &import.target);
                        vec![base.map_or_else(Vec::new, |p| {
                            extensions(&p, &["ts", "tsx", "js", "jsx"], "index")
                        })]
                    }
                    Language::Rust => self.rust(source, &import.target),
                    Language::Elm => vec![
                        self.aliases
                            .get(&format!("elm:{}", import.target))
                            .map_or_else(Vec::new, |p| p.iter().cloned().collect()),
                    ],
                    Language::Php => {
                        let target = import
                            .target
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_start_matches('\\');
                        vec![
                            self.aliases
                                .get(&format!("php:{target}"))
                                .map_or_else(Vec::new, |p| p.iter().cloned().collect()),
                        ]
                    }
                    Language::Python => vec![python(source, &import.target)],
                    _ => Vec::new(),
                };
                self.link(
                    source,
                    import.line,
                    EdgeKind::Imports,
                    candidates,
                    &mut output,
                )?;
            }
            if language.language == Language::Rust {
                for declaration in &language.declarations {
                    if declaration.kind == "module" && declaration.signature.ends_with(';') {
                        let base = rust_directory(source).join(&declaration.name);
                        self.link(
                            source,
                            declaration.line,
                            EdgeKind::Imports,
                            vec![extensions(&base, &["rs"], "mod")],
                            &mut output,
                        )?;
                    }
                }
            }
        }
        Ok(output)
    }

    fn link(
        &self,
        source: &Path,
        line: usize,
        kind: EdgeKind,
        groups: Vec<Vec<PathBuf>>,
        output: &mut Resolution,
    ) -> Result<(), Error> {
        let line = u32::try_from(line).map_err(|_| Error::Budget("lines"))?;
        let mut reason = if groups.iter().all(Vec::is_empty) {
            Reason::Unsupported
        } else {
            Reason::Missing
        };
        for candidates in groups {
            let found: BTreeSet<_> = candidates
                .into_iter()
                .filter(|p| self.files.contains_key(p))
                .collect();
            if found.len() > 1 {
                reason = Reason::Ambiguous;
                break;
            }
            if let Some(path) = found.into_iter().next() {
                if path != source {
                    output.links.push(Link {
                        id: file_id(&path, self.files[&path].kind)?,
                        path,
                        kind,
                        line,
                    });
                }
                return Ok(());
            }
        }
        output.diagnostics.push(Diagnostic {
            source: source.into(),
            line,
            reason,
        });
        Ok(())
    }

    fn rust(&self, source: &Path, target: &str) -> Vec<Vec<PathBuf>> {
        let target = target
            .split(" as ")
            .next()
            .unwrap_or(target)
            .split('{')
            .next()
            .unwrap_or("")
            .trim_end_matches(':');
        let parts: Vec<_> = target.split("::").filter(|s| !s.is_empty()).collect();
        let Some(first) = parts.first() else {
            return Vec::new();
        };
        let (roots, skip, fallback) = if *first == "crate" {
            let manifest = source
                .ancestors()
                .skip(1)
                .map(|p| p.join("Cargo.toml"))
                .find(|p| self.files.contains_key(p));
            let library = manifest
                .as_ref()
                .and_then(|path| {
                    let library = self.files[path]
                        .package
                        .as_ref()
                        .map_or(Path::new("src/lib.rs"), |p| p.library.as_path());
                    normalize(
                        path.parent().unwrap_or(Path::new("")),
                        &library.to_string_lossy(),
                    )
                })
                .unwrap_or_else(|| source.parent().unwrap_or(Path::new("")).join("lib.rs"));
            (
                vec![library.parent().unwrap_or(Path::new("")).into()],
                1,
                vec![library],
            )
        } else if *first == "self" {
            (vec![rust_directory(source)], 1, vec![source.into()])
        } else if *first == "super" {
            let count = parts.iter().take_while(|p| **p == "super").count();
            let mut root = rust_directory(source);
            for _ in 0..count {
                if !root.pop() {
                    return Vec::new();
                }
            }
            (vec![root], count, Vec::new())
        } else if let Some(paths) = self.aliases.get(&format!("rust:{first}")) {
            (
                paths
                    .iter()
                    .filter_map(|p| p.parent().map(Path::to_path_buf))
                    .collect(),
                1,
                paths.iter().cloned().collect(),
            )
        } else {
            (vec![rust_directory(source)], 0, Vec::new())
        };
        let mut groups = Vec::new();
        for length in (skip + 1..=parts.len()).rev() {
            let suffix = parts[skip..length].join("/");
            groups.push(
                roots
                    .iter()
                    .flat_map(|root| {
                        normalize(root, &suffix)
                            .map_or_else(Vec::new, |p| extensions(&p, &["rs"], "mod"))
                    })
                    .collect(),
            );
        }
        if !fallback.is_empty() {
            groups.push(fallback);
        }
        groups
    }
}

fn rust_directory(source: &Path) -> PathBuf {
    if source
        .file_name()
        .is_some_and(|n| n == "lib.rs" || n == "main.rs" || n == "mod.rs")
    {
        source.parent().unwrap_or(Path::new("")).into()
    } else {
        source.with_extension("")
    }
}

fn extensions(base: &Path, extensions: &[&str], entry: &str) -> Vec<PathBuf> {
    if base.extension().is_some() {
        return vec![base.into()];
    }
    let mut paths = vec![base.into()];
    for extension in extensions {
        paths.push(base.with_extension(extension));
        paths.push(base.join(entry).with_extension(extension));
    }
    paths
}

fn python(source: &Path, target: &str) -> Vec<PathBuf> {
    let depth = target.bytes().take_while(|b| *b == b'.').count();
    let mut root = if depth == 0 {
        PathBuf::new()
    } else {
        source.parent().unwrap_or(Path::new("")).into()
    };
    for _ in 1..depth {
        if !root.pop() {
            return Vec::new();
        }
    }
    normalize(&root, &target[depth..].replace('.', "/"))
        .map_or_else(Vec::new, |base| extensions(&base, &["py"], "__init__"))
}

pub(super) fn normalize(base: &Path, target: &str) -> Option<PathBuf> {
    if target.is_empty() || target.contains([':', '\\', '\0']) || target.starts_with('/') {
        return None;
    }
    let mut path = base.to_path_buf();
    for component in Path::new(target).components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                if !path.pop() {
                    return None;
                }
            }
            Component::Normal(name) => path.push(name),
            _ => return None,
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}
