#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};
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
    fs::write(dir.path().join("src/lib.rs"), "pub fn lease() {}\n").unwrap();
    dir
}

fn input(path: &Path, claims: &Value) -> String {
    let path = path.join("proposal.json");
    fs::write(&path, serde_json::to_vec(&claims).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

fn claim(statement: &str) -> Value {
    json!({
        "label": "Lease clock",
        "statement": statement,
        "rationale": "Maintains a single authority.",
        "scope": [],
        "evidence": [{
            "type": "source_span",
            "path": "src/lib.rs",
            "start_line": 1,
            "end_line": 1,
            "content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }]
    })
}

#[test]
fn structured_claims_are_validated_then_appended_as_one_pending_proposal() {
    let repo = repo();
    let task = success(repo.path(), &["task", "start", "--task", "lease behavior"]);
    let source = input(
        repo.path(),
        &json!({"decisions": [claim("PostgreSQL time is authoritative for leases.")]}),
    );

    let output = success(
        repo.path(),
        &[
            "propose",
            "--task-id",
            task["id"].as_str().unwrap(),
            "--input",
            &source,
        ],
    );
    assert!(output["proposal_id"].as_str().unwrap().starts_with("prop_"));
    assert_eq!(output["status"], "pending_review");
    assert_eq!(output["claims"][0]["kind"], "decision");

    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    let state = middleman_core::project(&store.events().unwrap()).unwrap();
    assert_eq!(state.proposals.len(), 1);
    assert_eq!(state.proposals.values().next().unwrap().claims.len(), 1);
}

#[test]
fn invalid_drafts_return_a_report_without_appending_events() {
    let repo = repo();
    let task = success(repo.path(), &["task", "start", "--task", "lease behavior"]);
    let source = input(
        repo.path(),
        &json!({"decisions": [
            claim("PostgreSQL time is authoritative for leases."),
            claim("PostgreSQL time is authoritative for leases.")
        ]}),
    );
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    let before = store.events().unwrap().len();
    drop(store);

    let output = run(
        repo.path(),
        &[
            "propose",
            "--task-id",
            task["id"].as_str().unwrap(),
            "--input",
            &source,
        ],
    );
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "invalid_proposal");
    assert_eq!(error["report"]["errors"][0]["code"], "duplicate_draft");

    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    assert_eq!(store.events().unwrap().len(), before);
}

#[test]
fn git_evidence_is_attached_without_executing_repository_source() {
    let repo = repo();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo.path())
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
    let task = success(repo.path(), &["task", "start", "--task", "lease behavior"]);
    let source = input(
        repo.path(),
        &json!({"invariants": [claim("Leases must use PostgreSQL time.")]}),
    );

    let output = success(
        repo.path(),
        &[
            "propose",
            "--task-id",
            task["id"].as_str().unwrap(),
            "--input",
            &source,
            "--from-git",
            "--format",
            "json",
        ],
    );
    let evidence = &output["claims"][0]["claim"]["evidence"];
    assert_eq!(evidence[1]["type"], "git_commit");
    assert_eq!(evidence[1]["sha"].as_str().unwrap().len(), 40);
    assert!(output["git_evidence_attached"].as_bool().unwrap());
}
