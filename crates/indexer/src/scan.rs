//! Bounded repository reads. Content lives only in the scan result, not in storage.

use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use middleman_core::{Config, Hash};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Source,
    Test,
    Document,
    Manifest,
    Configuration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: PathBuf,
    pub kind: FileKind,
    pub hash: Hash,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Ignored,
    LinkedOrSpecial,
    Unsupported,
    TooLarge,
    TooManyLines,
    Binary,
    Depth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub files: Vec<SourceFile>,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Clone, Copy)]
pub struct ScanBudget {
    pub max_total_bytes: u64,
    pub max_entries: usize,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self {
            max_total_bytes: 32 * 1024 * 1024,
            max_entries: 100_000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("repository scan I/O failed ({0:?})")]
    Io(std::io::ErrorKind),
    #[error("repository root must be an ordinary directory")]
    InvalidRoot,
    #[error("ignore rules are invalid, linked, or exceed parser limits")]
    InvalidRules,
    #[error("repository scan exceeded its {0} budget")]
    BudgetExceeded(&'static str),
}

impl From<std::io::Error> for ScanError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.kind())
    }
}

pub fn scan(root: &Path, config: &Config) -> Result<ScanReport, ScanError> {
    scan_with_budget(root, config, ScanBudget::default())
}

pub fn scan_with_budget(
    root: &Path,
    config: &Config,
    budget: ScanBudget,
) -> Result<ScanReport, ScanError> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(ScanError::InvalidRoot);
    }
    let root = root.canonicalize()?;
    let mut scanner = Scanner {
        root: &root,
        config,
        budget,
        started: Instant::now(),
        bytes: 0,
        entries: 0,
        report: ScanReport::default(),
    };
    scanner.check_time()?;
    let include = patterns(&root, &config.ignore.include)?;
    let exclude = patterns(&root, &config.ignore.exclude)?;
    let broker_dir = root.join(".middleman");
    let broker = match fs::symlink_metadata(&broker_dir) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {
            scanner.rules(&broker_dir.join("ignore"), &root)?
        }
        Ok(_) => return Err(ScanError::InvalidRules),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => patterns(&root, &[])?,
        Err(error) => return Err(error.into()),
    };
    scanner.walk(
        &root,
        0,
        &mut Vec::new(),
        &Filters {
            include,
            exclude,
            broker,
        },
    )?;
    scanner.check_time()?;
    scanner.report.files.sort_by(|a, b| a.path.cmp(&b.path));
    scanner.report.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(scanner.report)
}

struct Filters {
    include: Gitignore,
    exclude: Gitignore,
    broker: Gitignore,
}

struct Scanner<'a> {
    root: &'a Path,
    config: &'a Config,
    budget: ScanBudget,
    started: Instant,
    bytes: u64,
    entries: usize,
    report: ScanReport,
}

impl Scanner<'_> {
    fn check_time(&self) -> Result<(), ScanError> {
        if self.started.elapsed()
            >= Duration::from_secs(u64::from(self.config.limits.time_budget_secs))
        {
            return Err(ScanError::BudgetExceeded("time"));
        }
        Ok(())
    }

    fn read(&mut self, path: &Path) -> Result<Vec<u8>, ScanError> {
        self.check_time()?;
        let remaining = self.budget.max_total_bytes.saturating_sub(self.bytes);
        let limit = u64::from(self.config.limits.max_file_kb) * 1024;
        let mut bytes = Vec::new();
        File::open(path)?
            .take(limit.min(remaining).saturating_add(1))
            .read_to_end(&mut bytes)?;
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        if self.bytes > self.budget.max_total_bytes {
            return Err(ScanError::BudgetExceeded("total bytes"));
        }
        self.check_time()?;
        Ok(bytes)
    }

    fn rules(&mut self, path: &Path, base: &Path) -> Result<Gitignore, ScanError> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return patterns(base, &[]);
            }
            Err(error) => return Err(error.into()),
        };
        if is_link(&meta)
            || !meta.is_file()
            || meta.len() > u64::from(self.config.limits.max_file_kb) * 1024
        {
            return Err(ScanError::InvalidRules);
        }
        let bytes = self.read(path)?;
        if bytes.len() as u64 > u64::from(self.config.limits.max_file_kb) * 1024 {
            return Err(ScanError::InvalidRules);
        }
        let text = String::from_utf8(bytes).map_err(|_| ScanError::InvalidRules)?;
        if text.contains('\0') || text.lines().count() as u64 > self.config.limits.max_lines {
            return Err(ScanError::InvalidRules);
        }
        let mut builder = GitignoreBuilder::new(base);
        for line in text.lines() {
            builder
                .add_line(None, line)
                .map_err(|_| ScanError::InvalidRules)?;
        }
        builder.build().map_err(|_| ScanError::InvalidRules)
    }

    fn skip(&mut self, path: PathBuf, reason: SkipReason) {
        self.report.skipped.push(SkippedFile { path, reason });
    }

    fn walk(
        &mut self,
        directory: &Path,
        depth: u8,
        inherited: &mut Vec<Gitignore>,
        filters: &Filters,
    ) -> Result<(), ScanError> {
        self.check_time()?;
        inherited.push(self.rules(&directory.join(".gitignore"), directory)?);
        let mut entries = Vec::new();
        for entry in fs::read_dir(directory)? {
            self.check_time()?;
            self.entries += 1;
            if self.entries > self.budget.max_entries {
                return Err(ScanError::BudgetExceeded("directory entries"));
            }
            entries.push(entry?.path());
        }
        entries.sort();
        for path in entries {
            self.check_time()?;
            let relative = path
                .strip_prefix(self.root)
                .map_err(|_| ScanError::InvalidRoot)?
                .to_path_buf();
            let meta = fs::symlink_metadata(&path)?;
            let is_dir = meta.is_dir();
            if is_link(&meta) || (!is_dir && !meta.is_file()) {
                self.skip(relative, SkipReason::LinkedOrSpecial);
                continue;
            }
            let git_ignored = inherited
                .iter()
                .rev()
                .find_map(|rules| {
                    let matched = rules.matched(&path, is_dir);
                    if matched.is_none() {
                        None
                    } else {
                        Some(matched.is_ignore())
                    }
                })
                .unwrap_or(false);
            if hard_excluded(&relative)
                || git_ignored
                || filters
                    .broker
                    .matched_path_or_any_parents(&path, is_dir)
                    .is_ignore()
                || filters
                    .exclude
                    .matched_path_or_any_parents(&path, is_dir)
                    .is_ignore()
            {
                self.skip(relative, SkipReason::Ignored);
                continue;
            }
            if is_dir {
                if depth >= self.config.limits.max_depth {
                    self.skip(relative, SkipReason::Depth);
                } else {
                    self.walk(&path, depth + 1, inherited, filters)?;
                }
                continue;
            }
            if !self.config.ignore.include.is_empty()
                && !filters
                    .include
                    .matched_path_or_any_parents(&path, false)
                    .is_ignore()
            {
                self.skip(relative, SkipReason::Ignored);
                continue;
            }
            self.file(&path, relative, &meta)?;
        }
        inherited.pop();
        Ok(())
    }

    fn file(&mut self, path: &Path, relative: PathBuf, meta: &Metadata) -> Result<(), ScanError> {
        let Some(kind) = classify(&relative) else {
            self.skip(relative, SkipReason::Unsupported);
            return Ok(());
        };
        let limit = u64::from(self.config.limits.max_file_kb) * 1024;
        if meta.len() > limit {
            self.skip(relative, SkipReason::TooLarge);
            return Ok(());
        }
        let bytes = self.read(path)?;
        if bytes.len() as u64 > limit {
            self.skip(relative, SkipReason::TooLarge);
            return Ok(());
        }
        let hash = Hash::of(&bytes);
        let Ok(content) = String::from_utf8(bytes) else {
            self.skip(relative, SkipReason::Binary);
            return Ok(());
        };
        if content.contains('\0') {
            self.skip(relative, SkipReason::Binary);
        } else if content.lines().count() as u64 > self.config.limits.max_lines {
            self.skip(relative, SkipReason::TooManyLines);
        } else {
            self.report.files.push(SourceFile {
                path: relative,
                kind,
                hash,
                content,
            });
        }
        Ok(())
    }
}

fn patterns(root: &Path, patterns: &[String]) -> Result<Gitignore, ScanError> {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|_| ScanError::InvalidRules)?;
    }
    builder.build().map_err(|_| ScanError::InvalidRules)
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn hard_excluded(path: &Path) -> bool {
    path.components().any(|part| {
        let Some(name) = part.as_os_str().to_str() else {
            return true;
        };
        let name = name.to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".git"
                | ".jj"
                | ".middleman"
                | "node_modules"
                | "vendor"
                | "target"
                | "dist"
                | "build"
                | ".cache"
                | "__pycache__"
                | ".venv"
                | "venv"
                | "elm-stuff"
                | ".next"
                | "coverage"
                | ".ssh"
                | ".aws"
                | ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "credentials"
                | "credentials.json"
                | "secrets"
                | "secrets.json"
                | "id_rsa"
                | "id_ed25519"
        ) || name.starts_with(".env")
            || name.starts_with("secrets.")
            || name.starts_with("credentials.")
            || name == ".git-credentials"
            || [
                ".pem",
                ".key",
                ".p12",
                ".pfx",
                ".sqlite3",
                ".generated.rs",
                ".min.js",
                ".map",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
    })
}

pub fn classify(path: &Path) -> Option<FileKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "md" | "markdown" | "rst" | "adoc") || name == "readme" {
        return Some(FileKind::Document);
    }
    if matches!(
        name.as_str(),
        "cargo.toml"
            | "package.json"
            | "composer.json"
            | "elm.json"
            | "pyproject.toml"
            | "go.mod"
            | "requirements.txt"
    ) {
        return Some(FileKind::Manifest);
    }
    if matches!(
        extension.as_str(),
        "rs" | "php" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "elm"
    ) {
        let test_path = path.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some("test" | "tests" | "__tests__")
            )
        });
        let test_name = name.starts_with("test_")
            || name.contains("_test.")
            || name.contains(".test.")
            || name.contains(".spec.")
            || name.ends_with("test.php");
        return Some(if test_path || test_name {
            FileKind::Test
        } else {
            FileKind::Source
        });
    }
    matches!(extension.as_str(), "toml" | "json" | "yaml" | "yml" | "sql")
        .then_some(FileKind::Configuration)
}
