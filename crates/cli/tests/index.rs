#![allow(clippy::unwrap_used)]
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::Value;

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

fn init(repo: &Path, args: &[&str]) {
    let output = run(repo, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn index_persists_source_free_cache_and_refreshes_incrementally() {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path(), &["init", "--name", "indexed"]);
    fs::create_dir(repo.path().join("src")).unwrap();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn stable() {} // SOURCE_BODY_SENTINEL\n",
    )
    .unwrap();

    let first = success(repo.path(), &["index", "--full"]);
    assert_eq!(first["cache"], "rebuilt");
    assert_eq!(first["reparsed"], 1);
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    let cache = String::from_utf8(store.index_snapshot().unwrap().unwrap()).unwrap();
    assert!(!cache.contains("SOURCE_BODY_SENTINEL"));
    assert_eq!(store.state().unwrap().last_sequence, 2);
    drop(store);

    let unchanged = success(repo.path(), &["index", "--changed"]);
    assert_eq!(unchanged["cache"], "reused");
    assert_eq!(unchanged["reparsed"], 0);
    assert_eq!(unchanged["reused"], 1);

    fs::write(repo.path().join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
    let changed = success(repo.path(), &["index"]);
    assert_eq!(changed["cache"], "reused");
    assert_eq!(changed["reparsed"], 1);
    assert!(changed["rederived"].as_u64().unwrap() >= 1);
}

#[test]
fn incompatible_index_flags_leave_state_unchanged() {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path(), &["init"]);
    assert!(
        !run(repo.path(), &["index", "--full", "--changed"])
            .status
            .success()
    );
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    assert_eq!(store.state().unwrap().last_sequence, 1);
    assert!(store.index_snapshot().unwrap().is_none());
}

#[test]
fn status_separates_reviewed_memory_from_the_derived_index() {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path(), &["init", "--name", "indexed"]);
    fs::create_dir(repo.path().join("src")).unwrap();
    fs::write(repo.path().join("src/lib.rs"), "pub fn stable() {}\n").unwrap();
    success(repo.path(), &["index", "--full"]);

    let output = run(repo.path(), &["status"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("memory:    0 reviewed entities"), "{text}");
    assert!(
        text.contains("indexed:   1 modules, 1 symbols, 0 tests, 0 documents"),
        "{text}"
    );
}

#[test]
fn status_detects_susumu_without_making_its_optional_digest_a_failure() {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path(), &["init", "--name", "susumu fixture"]);
    fs::write(repo.path().join("susumu.toml"), "[portal]\n").unwrap();

    let output = run(repo.path(), &["status"]);
    let text = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success(), "{text}");
    assert!(text.contains("susumu:    "), "{text}");
}
