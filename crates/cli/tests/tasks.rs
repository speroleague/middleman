#![allow(clippy::unwrap_used)]
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn run(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_middleman"))
        .arg("--repo")
        .arg(repo)
        .args(args)
        .output()
        .unwrap()
}
fn success(repo: &Path, args: &[&str]) -> Value {
    let output = run(repo, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["init"]).status.success());
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub fn before() {}\n").unwrap();
    dir
}

#[test]
fn lifecycle_persists_observations_without_prompt_or_source() {
    let repo = repo();
    fs::write(repo.path().join(".env"), "SECRET_SENTINEL=excluded").unwrap();
    let started = success(
        repo.path(),
        &[
            "task",
            "start",
            "--task",
            "src/lib.rs RAW_PROMPT_SENTINEL",
            "--validation",
            "cargo test",
        ],
    );
    let id = started["id"].as_str().unwrap();
    assert_eq!(started["status"], "in_progress");
    fs::write(repo.path().join("src/lib.rs"), "pub fn after() {}\n").unwrap();
    fs::write(repo.path().join("src/new.rs"), "pub fn new_file() {}\n").unwrap();
    let finished = success(
        repo.path(),
        &["task", "finish", id, "--passed", "cargo test"],
    );
    assert_eq!(finished["status"], "completed");
    assert_eq!(finished["observations"]["changed"][0], "src/lib.rs");
    assert_eq!(finished["observations"]["added"][0], "src/new.rs");
    assert_eq!(finished["validation_results"][0]["status"], "passed");
    let shown = success(repo.path(), &["task", "show", id]);
    assert_eq!(shown["summary"], finished["summary"]);
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    let events = serde_json::to_string(&store.events().unwrap()).unwrap();
    for forbidden in [
        "RAW_PROMPT_SENTINEL",
        "SECRET_SENTINEL",
        "pub fn before",
        "pub fn after",
    ] {
        assert!(!events.contains(forbidden));
    }
}

#[test]
fn unchanged_dirty_baseline_and_unreported_validation_are_not_claimed_as_changes() {
    let repo = repo();
    let started = success(
        repo.path(),
        &[
            "task",
            "start",
            "--task",
            "src/lib.rs",
            "--objective",
            "Curated objective",
        ],
    );
    let finished = success(
        repo.path(),
        &["task", "finish", started["id"].as_str().unwrap()],
    );
    assert_eq!(finished["objective"], "Curated objective");
    assert!(
        finished["observations"]["changed"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        finished["validation_results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let unknown = success(
        repo.path(),
        &["task", "start", "--task", "unmatched request"],
    );
    assert!(
        unknown["handoff"]
            .as_str()
            .unwrap()
            .contains("confidence is low")
    );
}

#[test]
fn duplicate_finish_and_conflicting_validation_do_not_append() {
    let repo = repo();
    let started = success(repo.path(), &["task", "start", "--task", "src/lib.rs"]);
    let id = started["id"].as_str().unwrap();
    let failed = run(
        repo.path(),
        &["task", "finish", id, "--passed", "test", "--failed", "test"],
    );
    assert!(!failed.status.success());
    assert_eq!(
        success(repo.path(), &["task", "show", id])["status"],
        "in_progress"
    );
    success(repo.path(), &["task", "finish", id, "--failed", "test"]);
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    let count = store.events().unwrap().len();
    let failed = run(repo.path(), &["task", "finish", id]);
    assert_eq!(
        serde_json::from_slice::<Value>(&failed.stderr).unwrap()["error"]["code"],
        "task_terminal"
    );
    assert_eq!(store.events().unwrap().len(), count);
}

#[test]
fn from_git_records_heads_without_running_hooks() {
    let repo = repo();
    let hooks = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("-c")
            .arg(format!("core.hooksPath={}", hooks.path().display()))
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--quiet"]);
    git(&["add", "src/lib.rs"]);
    git(&["commit", "--quiet", "-m", "fixture: establish baseline"]);
    let started = success(repo.path(), &["task", "start", "--task", "src/lib.rs"]);
    git(&[
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "fixture: advance head",
    ]);
    let finished = success(
        repo.path(),
        &[
            "task",
            "finish",
            started["id"].as_str().unwrap(),
            "--from-git",
        ],
    );
    let observations = &finished["observations"];
    assert_ne!(observations["git_before"], observations["git_after"]);
    assert_eq!(observations["git_before"].as_str().unwrap().len(), 40);
    assert!(observations["changed"].as_array().unwrap().is_empty());
}
