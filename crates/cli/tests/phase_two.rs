#![allow(clippy::unwrap_used)]
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};

fn run(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_middleman"))
        .arg("--repo")
        .arg(repo)
        .args(args)
        .output()
        .unwrap()
}

fn init(repo: &Path, name: &str) {
    let output = run(repo, &["init", "--name", name]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json(repo: &Path, args: &[&str]) -> Value {
    let output = run(repo, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn separate_sessions_continue_related_work_from_packet_and_reviewed_state() {
    let first = tempfile::tempdir().unwrap();
    init(first.path(), "handoff-fixture");
    fs::create_dir(first.path().join("src")).unwrap();
    fs::write(first.path().join("src/lease.rs"), "pub fn lease() {}\n").unwrap();
    json(first.path(), &["index"]);
    let initial = json(
        first.path(),
        &["prepare", "--task", "lease policy", "--format", "json"],
    );
    assert!(initial["items"].is_array());
    let task = json(first.path(), &["task", "start", "--task", "lease policy"]);
    let input = first.path().join("claims.json");
    fs::write(
        &input,
        serde_json::to_vec(&json!({"decisions": [{
            "label": "Lease policy",
            "statement": "Lease time has one authority.",
            "rationale": "Comparable expiry calculations.",
            "scope": [],
            "evidence": [{
                "type": "source_span", "path": "src/lease.rs", "start_line": 1,
                "end_line": 1,
                "content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }]
        }]}))
        .unwrap(),
    )
    .unwrap();
    let proposal = json(
        first.path(),
        &[
            "propose",
            "--task-id",
            task["id"].as_str().unwrap(),
            "--input",
            input.to_str().unwrap(),
        ],
    );
    let proposal_id = proposal["proposal_id"].as_str().unwrap();
    assert_eq!(
        json(first.path(), &["review", proposal_id])["status"],
        "pending_review"
    );
    assert_eq!(
        json(first.path(), &["apply", proposal_id])["status"],
        "accepted"
    );
    json(
        first.path(),
        &["task", "finish", task["id"].as_str().unwrap()],
    );
    let archive = first.path().join("handoff.jsonl");
    let output = run(first.path(), &["backup", archive.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let second = tempfile::tempdir().unwrap();
    init(second.path(), "temporary-local-name");
    fs::create_dir(second.path().join("src")).unwrap();
    fs::write(second.path().join("src/lease.rs"), "pub fn lease() {}\n").unwrap();
    assert!(
        run(second.path(), &["import", archive.to_str().unwrap()])
            .status
            .success()
    );
    json(second.path(), &["index"]);
    let packet = json(
        second.path(),
        &[
            "prepare",
            "--task",
            "lease policy regression",
            "--format",
            "json",
        ],
    );
    assert!(
        packet["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["title"] == "Lease policy")
    );
    let related = json(
        second.path(),
        &["task", "start", "--task", "lease policy regression"],
    );
    assert_eq!(
        json(
            second.path(),
            &["task", "finish", related["id"].as_str().unwrap()]
        )["status"],
        "completed"
    );
}
