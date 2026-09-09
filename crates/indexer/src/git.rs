//! Restricted Git commands produce bounded, allowlisted observations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::process;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Process(#[from] process::Error),
    #[error("Git command failed")]
    Failed,
    #[error("Git produced invalid output")]
    UnusableOutput,
    #[error("Git scan requires a repository root and safe relative paths")]
    InvalidInput,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub max_commits: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_output_bytes: 4 * 1024 * 1024,
            max_commits: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub previous_path: Option<PathBuf>,
    pub index: ChangeStatus,
    pub worktree: ChangeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitTouch {
    pub sha: String,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub head: Option<String>,
    pub changes: Vec<Change>,
    /// Newest first, along first-parent history. Groups support later co-change edges.
    pub commits: Vec<CommitTouch>,
    pub last_touch: BTreeMap<PathBuf, String>,
    pub history_truncated: bool,
}

pub fn probe() -> Result<String, Error> {
    let output = process::run(
        command(None, &[]).arg("--version"),
        Duration::from_secs(2),
        1024,
    )?;
    if output.code != Some(0) {
        return Err(Error::Failed);
    }
    let line = std::str::from_utf8(&output.stdout)
        .map_err(|_| Error::UnusableOutput)?
        .trim();
    if !line.starts_with("git version ") || line.chars().any(char::is_control) {
        return Err(Error::UnusableOutput);
    }
    Ok(line.to_owned())
}

pub fn scan(root: &Path, allowed: &BTreeSet<PathBuf>, limits: Limits) -> Result<Snapshot, Error> {
    if allowed.iter().any(|path| !safe_path(path)) || limits.max_commits > 1024 {
        return Err(Error::InvalidInput);
    }
    let root = root.canonicalize().map_err(|_| Error::InvalidInput)?;
    let mut runner = Runner {
        root: &root,
        limits,
        started: Instant::now(),
        bytes: 0,
        overrides: Vec::new(),
    };
    let top = runner.run(&["rev-parse", "--show-toplevel"], &[0])?;
    let top = std::str::from_utf8(&top)
        .map_err(|_| Error::UnusableOutput)?
        .trim_end();
    if Path::new(top)
        .canonicalize()
        .map_err(|_| Error::InvalidInput)?
        != root
    {
        return Err(Error::InvalidInput);
    }
    let names = runner.run(
        &[
            "config",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|smudge|process|required)$",
        ],
        &[0, 1],
    )?;
    runner.overrides = filter_overrides(&names)?;
    let status = runner.run(
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=all",
            "--no-renames",
        ],
        &[0],
    )?;
    let mut snapshot = Snapshot {
        changes: parse_status(&status, allowed)?,
        ..Snapshot::default()
    };
    let head = runner.run(&["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1])?;
    if head.is_empty() {
        return Ok(snapshot);
    }
    let head = std::str::from_utf8(&head)
        .map_err(|_| Error::UnusableOutput)?
        .trim();
    if !valid_sha(head) {
        return Err(Error::UnusableOutput);
    }
    snapshot.head = Some(head.to_owned());
    let count = format!("--max-count={}", limits.max_commits + 1);
    let revisions = runner.run(&["rev-list", "--first-parent", &count, "HEAD", "--"], &[0])?;
    let revisions = std::str::from_utf8(&revisions).map_err(|_| Error::UnusableOutput)?;
    let commits: Vec<_> = revisions.lines().collect();
    if commits.iter().any(|sha| !valid_sha(sha)) {
        return Err(Error::UnusableOutput);
    }
    snapshot.history_truncated = commits.len() > limits.max_commits;
    for sha in commits.into_iter().take(limits.max_commits) {
        let output = runner.run(
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-only",
                "-r",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--first-parent",
                "-m",
                sha,
                "--",
            ],
            &[0],
        )?;
        let paths = parse_paths(&output, allowed)?;
        for path in &paths {
            snapshot
                .last_touch
                .entry(path.clone())
                .or_insert_with(|| sha.to_owned());
        }
        snapshot.commits.push(CommitTouch {
            sha: sha.to_owned(),
            paths,
        });
    }
    Ok(snapshot)
}

struct Runner<'a> {
    root: &'a Path,
    limits: Limits,
    started: Instant,
    bytes: usize,
    overrides: Vec<String>,
}

impl Runner<'_> {
    fn run(&mut self, args: &[&str], codes: &[i32]) -> Result<Vec<u8>, Error> {
        let remaining = self.limits.timeout.saturating_sub(self.started.elapsed());
        let output = process::run(
            command(Some(self.root), &self.overrides).args(args),
            remaining,
            self.limits.max_output_bytes.saturating_sub(self.bytes),
        )?;
        self.bytes += output.stdout.len();
        if !output.code.is_some_and(|code| codes.contains(&code)) {
            return Err(Error::Failed);
        }
        Ok(output.stdout)
    }
}

fn command(root: Option<&Path>, overrides: &[String]) -> Command {
    let mut cmd = Command::new("git");
    if let Some(root) = root {
        cmd.current_dir(root);
    }
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            cmd.env_remove(key);
        }
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .arg("--no-pager")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=",
            "-c",
            "log.showSignature=false",
            "-c",
            "diff.external=",
            "-c",
            "core.untrackedCache=false",
        ]);
    for value in overrides {
        cmd.arg("-c").arg(value);
    }
    cmd
}

fn filter_overrides(bytes: &[u8]) -> Result<Vec<String>, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::UnusableOutput)?;
    let mut overrides = BTreeSet::new();
    for key in text.lines() {
        if !key.starts_with("filter.") || key.contains('=') || key.chars().any(char::is_control) {
            return Err(Error::UnusableOutput);
        }
        let Some((prefix, suffix)) = key.rsplit_once('.') else {
            return Err(Error::UnusableOutput);
        };
        if !matches!(suffix, "clean" | "smudge" | "process" | "required") {
            return Err(Error::UnusableOutput);
        }
        for field in ["clean=", "smudge=", "process=", "required=false"] {
            overrides.insert(format!("{prefix}.{field}"));
        }
    }
    Ok(overrides.into_iter().collect())
}

fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn safe_path(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn path(bytes: &[u8]) -> Result<PathBuf, Error> {
    let value = std::str::from_utf8(bytes).map_err(|_| Error::UnusableOutput)?;
    if value.contains('\\') {
        return Err(Error::UnusableOutput);
    }
    let value = PathBuf::from(value);
    if !safe_path(&value) {
        return Err(Error::UnusableOutput);
    }
    Ok(value)
}

fn records(bytes: &[u8]) -> Result<impl Iterator<Item = &[u8]>, Error> {
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(Error::UnusableOutput);
    }
    Ok(bytes.split(|b| *b == 0).filter(|r| !r.is_empty()))
}

pub fn parse_paths(bytes: &[u8], allowed: &BTreeSet<PathBuf>) -> Result<Vec<PathBuf>, Error> {
    let mut paths = BTreeSet::new();
    for record in records(bytes)? {
        let value = path(record)?;
        if allowed.contains(&value) {
            paths.insert(value);
        }
    }
    Ok(paths.into_iter().collect())
}

pub fn parse_status(bytes: &[u8], allowed: &BTreeSet<PathBuf>) -> Result<Vec<Change>, Error> {
    let mut changes = Vec::new();
    let mut records = records(bytes)?;
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err(Error::UnusableOutput);
        }
        let index = change_status(record[0])?;
        let worktree = change_status(record[1])?;
        let path = path(&record[3..])?;
        let previous_path = if [index, worktree]
            .iter()
            .any(|s| matches!(s, ChangeStatus::Renamed | ChangeStatus::Copied))
        {
            Some(self::path(records.next().ok_or(Error::UnusableOutput)?)?)
        } else {
            None
        };
        if allowed.contains(&path) {
            changes.push(Change {
                path,
                previous_path: previous_path.filter(|p| allowed.contains(p)),
                index,
                worktree,
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

fn change_status(byte: u8) -> Result<ChangeStatus, Error> {
    Ok(match byte {
        b' ' => ChangeStatus::Unmodified,
        b'A' => ChangeStatus::Added,
        b'M' | b'T' => ChangeStatus::Modified,
        b'D' => ChangeStatus::Deleted,
        b'R' => ChangeStatus::Renamed,
        b'C' => ChangeStatus::Copied,
        b'U' => ChangeStatus::Unmerged,
        b'?' => ChangeStatus::Untracked,
        _ => return Err(Error::UnusableOutput),
    })
}
