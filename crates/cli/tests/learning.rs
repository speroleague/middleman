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
    let repo = tempfile::tempdir().unwrap();
    assert!(run(repo.path(), &["init"]).status.success());
    fs::create_dir(repo.path().join("src")).unwrap();
    fs::write(repo.path().join("src/lib.rs"), "pub fn before() {}\n").unwrap();
    repo
}

#[test]
fn local_outcomes_generate_reviewable_tuning_and_maintenance_suggestions() {
    let repo = repo();
    let task = success(repo.path(), &["task", "start", "--task", "src/lib.rs"]);
    let node = task["scope"][0].as_str().unwrap();
    fs::write(repo.path().join("src/lib.rs"), "pub fn after() {}\n").unwrap();
    success(
        repo.path(),
        &[
            "task",
            "finish",
            task["id"].as_str().unwrap(),
            "--passed",
            "cargo test",
        ],
    );
    success(
        repo.path(),
        &[
            "learn",
            "observe",
            "--node",
            node,
            "--signal",
            "file-overlap",
        ],
    );
    let report = success(repo.path(), &["learn", "suggest"]);
    assert!(
        report["weight_suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["field"] == "file_overlap")
    );
    assert!(
        report["maintenance_suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "missing_documentation")
    );
    let applied = success(repo.path(), &["learn", "apply"]);
    assert!(!applied["applied"].as_array().unwrap().is_empty());
    let config = fs::read_to_string(repo.path().join(".middleman/middleman.toml")).unwrap();
    assert!(config.contains("file_overlap = 4.0"));
}

#[test]
fn optional_ai_proposals_require_opt_in_and_never_append_events() {
    let repo = repo();
    let task = success(repo.path(), &["task", "start", "--task", "src/lib.rs"]);
    let node = task["scope"][0].as_str().unwrap();
    let input = repo.path().join("proposal.json");
    fs::write(
        &input,
        format!(
            r#"{{"suggestions":[{{"title":"Document module","rationale":"Repeated retrieval outcome","node_ids":["{node}"]}}]}}"#
        ),
    )
    .unwrap();
    let before = middleman_store::Store::open_read_only(&repo.path().join(".middleman"))
        .unwrap()
        .events()
        .unwrap()
        .len();
    let disabled = run(
        repo.path(),
        &[
            "learn",
            "summarize",
            "--source",
            "harness",
            "--input",
            input.to_str().unwrap(),
        ],
    );
    assert!(!disabled.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&disabled.stderr).unwrap()["error"]["code"],
        "optional_ai_disabled"
    );
    let config_path = repo.path().join(".middleman/middleman.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    let enabled = config
        .replace("enabled = false", "enabled = true")
        .replace("mode = \"disabled\"", "mode = \"harness\"");
    fs::write(&config_path, enabled).unwrap();
    let proposal = success(
        repo.path(),
        &[
            "learn",
            "summarize",
            "--source",
            "harness",
            "--input",
            input.to_str().unwrap(),
        ],
    );
    assert_eq!(proposal["review_required"], true);
    assert_eq!(
        proposal["persistence"],
        "none; submit reviewed durable claims through middleman propose"
    );
    assert_eq!(
        middleman_store::Store::open_read_only(&repo.path().join(".middleman"))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        before
    );
}
