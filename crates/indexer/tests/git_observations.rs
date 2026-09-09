#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use middleman_indexer::{git, process};

fn allowed(paths: &[&str]) -> BTreeSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn run(root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    let output = command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture Git command failed: {args:?}"
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn repository() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    run(root.path(), &["init", "--quiet"]);
    root
}

#[test]
fn histories_and_worktree_changes_are_repeatable_and_allowlisted() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("a.rs"), "first").unwrap();
    fs::write(root.join("b.rs"), "stable").unwrap();
    fs::write(root.join("omitted.md"), "synthetic fixture").unwrap();
    run(root, &["add", "."]);
    run(root, &["commit", "-qm", "test: initial fixture"]);
    let first = run(root, &["rev-parse", "HEAD"]);
    fs::write(root.join("a.rs"), "second").unwrap();
    run(root, &["add", "a.rs"]);
    run(root, &["commit", "-qm", "test: change fixture"]);
    let second = run(root, &["rev-parse", "HEAD"]);
    fs::write(root.join("a.rs"), "working tree change").unwrap();
    fs::remove_file(root.join("b.rs")).unwrap();
    fs::write(root.join("space name.rs"), "untracked").unwrap();
    let allowed = allowed(&["a.rs", "b.rs", "space name.rs"]);
    let before_index = fs::read(root.join(".git/index")).unwrap();
    let result = git::scan(root, &allowed, git::Limits::default()).unwrap();
    assert_eq!(
        result,
        git::scan(root, &allowed, git::Limits::default()).unwrap()
    );
    assert_eq!(before_index, fs::read(root.join(".git/index")).unwrap());
    assert_eq!(result.head.as_deref(), Some(second.as_str()));
    assert_eq!(result.last_touch[Path::new("a.rs")], second);
    assert_eq!(result.last_touch[Path::new("b.rs")], first);
    assert_eq!(
        result.commits[1].paths,
        [PathBuf::from("a.rs"), PathBuf::from("b.rs")]
    );
    assert_eq!(result.changes.len(), 3);
    assert_eq!(result.changes[1].worktree, git::ChangeStatus::Deleted);
    assert_eq!(result.changes[2].worktree, git::ChangeStatus::Untracked);
    assert!(!result.history_truncated);
    let limited = git::scan(
        root,
        &allowed,
        git::Limits {
            max_commits: 1,
            ..git::Limits::default()
        },
    )
    .unwrap();
    assert!(limited.history_truncated);
    assert_eq!(limited.commits.len(), 1);
}

#[test]
fn empty_repositories_and_invalid_inputs_have_explicit_results() {
    let dir = repository();
    let result = git::scan(dir.path(), &BTreeSet::new(), git::Limits::default()).unwrap();
    assert_eq!(result, git::Snapshot::default());
    assert_eq!(
        git::scan(
            dir.path(),
            &allowed(&["../outside"]),
            git::Limits::default()
        ),
        Err(git::Error::InvalidInput)
    );
    let not_repo = tempfile::tempdir().unwrap();
    assert!(git::scan(not_repo.path(), &BTreeSet::new(), git::Limits::default()).is_err());
    assert!(matches!(
        git::scan(
            dir.path(),
            &BTreeSet::new(),
            git::Limits {
                timeout: Duration::ZERO,
                ..git::Limits::default()
            }
        ),
        Err(git::Error::Process(process::Error::Timeout))
    ));
    assert!(matches!(
        git::scan(
            dir.path(),
            &BTreeSet::new(),
            git::Limits {
                max_output_bytes: 1,
                ..git::Limits::default()
            }
        ),
        Err(git::Error::Process(process::Error::OutputLimit))
    ));
}

#[test]
fn repository_filters_and_fsmonitor_are_not_executed() {
    let dir = repository();
    let root = dir.path();
    fs::write(root.join("a.rs"), "initial").unwrap();
    run(root, &["add", "."]);
    run(root, &["commit", "-qm", "test: filter fixture"]);
    fs::write(root.join(".gitattributes"), "*.rs filter=hostile\n").unwrap();
    let marker = "echo invoked > filter-ran; cat";
    run(root, &["config", "filter.hostile.clean", marker]);
    run(root, &["config", "filter.hostile.process", marker]);
    run(root, &["config", "filter.hostile.required", "true"]);
    run(
        root,
        &["config", "core.fsmonitor", "echo invoked > monitor-ran"],
    );
    fs::write(root.join("a.rs"), "changed and longer").unwrap();
    let result = git::scan(root, &allowed(&["a.rs"]), git::Limits::default()).unwrap();
    assert_eq!(result.changes.len(), 1);
    assert!(!root.join("filter-ran").exists());
    assert!(!root.join("monitor-ran").exists());
}

#[test]
fn nul_parsers_handle_renames_newlines_and_reject_truncation() {
    let allowed = allowed(&["new name.rs", "old.rs", "line\nname.rs"]);
    let result =
        git::parse_status(b"R  new name.rs\0old.rs\0?? line\nname.rs\0", &allowed).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[1].previous_path, Some(PathBuf::from("old.rs")));
    assert!(git::parse_status(b" M old.rs", &allowed).is_err());
    assert!(git::parse_status(b"R  new name.rs\0", &allowed).is_err());
    assert!(git::parse_paths(b"../outside\0", &allowed).is_err());
}

#[test]
fn process_limits_terminate_sleeping_and_noisy_children() {
    let executable = std::env::current_exe().unwrap();
    let child = |mode: &str| {
        let mut command = Command::new(&executable);
        command
            .args(["--exact", "process_child", "--nocapture"])
            .env("MIDDLEMAN_PROCESS_TEST_MODE", mode);
        command
    };
    let started = Instant::now();
    assert!(matches!(
        process::run(&mut child("sleep"), Duration::from_millis(100), 1024),
        Err(process::Error::Timeout)
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(matches!(
        process::run(&mut child("flood"), Duration::from_secs(2), 1024),
        Err(process::Error::OutputLimit)
    ));
    assert_eq!(
        process::run(&mut child("fail"), Duration::from_secs(2), 1024)
            .unwrap()
            .code,
        Some(7)
    );
}

#[test]
fn process_child() {
    use std::io::{self, Write};

    match std::env::var("MIDDLEMAN_PROCESS_TEST_MODE").as_deref() {
        Ok("sleep") => std::thread::sleep(Duration::from_secs(30)),
        Ok("flood") => {
            let mut output = io::stdout().lock();
            while output.write_all(&[b'x'; 4096]).is_ok() {}
        }
        Ok("fail") => std::process::exit(7),
        _ => (),
    }
}
