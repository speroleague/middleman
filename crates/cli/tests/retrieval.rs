#![allow(clippy::unwrap_used)]

use middleman_core::{EventKind, RetrievalSignal};
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn command(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_middleman"))
        .arg("--repo")
        .arg(repo)
        .args(args)
        .output()
        .unwrap()
}

fn success(repo: &Path, args: &[&str]) -> Output {
    let output = command(repo, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn copy(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}

fn fixture() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/rust-workspace"),
        repo.path(),
    );
    success(repo.path(), &["init", "--name", "fixture"]);
    repo
}

#[test]
fn other_language_fixtures_render_without_executing_source() {
    for (name, task, expected) in [
        ("laravel-app", "Post", "app/Models/Post.php"),
        ("elm-frontend", "Model", "src/Model.elm"),
    ] {
        let repo = tempfile::tempdir().unwrap();
        copy(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures")
                .join(name),
            repo.path(),
        );
        success(repo.path(), &["init"]);
        let output = success(
            repo.path(),
            &[
                "prepare", "--task", task, "--format", "json", "--budget", "4000",
            ],
        );
        let packet: middleman_packet::Packet = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            packet
                .items
                .iter()
                .any(|item| item.path.as_deref() == Some(expected)),
            "{name}"
        );
    }
}

#[test]
fn prepare_routes_fixture_and_explains_across_sessions_without_storing_prompt() {
    let repo = fixture();
    let task = "lease renewal REQUEST_NOT_TO_BE_RETAINED";
    let output = success(
        repo.path(),
        &[
            "prepare", "--task", task, "--format", "json", "--budget", "4000",
        ],
    );
    let packet: middleman_packet::Packet = serde_json::from_slice(&output.stdout).unwrap();
    let paths: Vec<_> = packet
        .items
        .iter()
        .filter_map(|item| item.path.as_deref())
        .collect();
    assert!(paths.contains(&"crates/alpha/src/frontier.rs"), "{paths:?}");
    assert!(
        paths.contains(&"crates/alpha/tests/frontier_test.rs"),
        "{paths:?}"
    );
    assert!(paths.contains(&"docs/contracts/leases.md"), "{paths:?}");
    let metadata: Value = serde_json::from_slice(&output.stderr).unwrap();
    let id = metadata["packet_id"].as_str().unwrap();
    let explanation: Value =
        serde_json::from_slice(&success(repo.path(), &["explain", id]).stdout).unwrap();
    assert_eq!(
        explanation["record"]["selected"].as_array().unwrap().len(),
        packet.items.len()
    );
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    let events = store.events().unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        !serde_json::to_string(&events)
            .unwrap()
            .contains("REQUEST_NOT_TO_BE_RETAINED")
    );
    let state = middleman_core::project(&events).unwrap();
    assert!(
        state
            .retrieval
            .iter()
            .all(|record| record.signal == RetrievalSignal::Retrieved)
    );
    assert_eq!(state.retrieval.len(), packet.items.len());
    let search: Value = serde_json::from_slice(
        &success(
            repo.path(),
            &["search", "worker output", "--type", "module"],
        )
        .stdout,
    )
    .unwrap();
    assert!(
        search["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"] == "crates/beta/src/main.rs")
    );
}

#[test]
fn search_expand_and_deletion_use_current_files_without_query_events() {
    let repo = fixture();
    let search: Value = serde_json::from_slice(
        &success(
            repo.path(),
            &["search", "frontier", "--type", "module", "--limit", "1"],
        )
        .stdout,
    )
    .unwrap();
    assert_eq!(search["items"].as_array().unwrap().len(), 1);
    let id = search["items"][0]["id"].as_str().unwrap();
    success(repo.path(), &["explain", id]);
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    assert_eq!(store.events().unwrap().len(), 1);
    drop(store);
    let output = success(repo.path(), &["expand", id, "--format", "cir"]);
    let packet = middleman_packet::parse_cir(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert!(packet.items.iter().any(|item| item.id.as_str() == id));
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    assert!(matches!(
        store.events().unwrap()[1].kind,
        EventKind::PacketPrepared { expanded: true, .. }
    ));
    drop(store);
    let relative = search["items"][0]["path"].as_str().unwrap();
    fs::remove_file(repo.path().join(relative)).unwrap();
    let output = command(repo.path(), &["expand", id]);
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "not_found"
    );
}

#[test]
fn errors_are_json_and_failed_preparation_does_not_append() {
    let repo = fixture();
    for args in [
        vec!["prepare", "--task", "lease", "--budget", "0"],
        vec!["search", "lease", "--limit", "129"],
        vec!["search", "lease", "--type", "bad"],
        vec!["expand", "../../outside"],
        vec!["prepare", "--task", ""],
    ] {
        let output = command(repo.path(), &args);
        assert!(!output.status.success());
        assert!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"].is_string()
        );
    }
    let output = command(
        repo.path(),
        &["prepare", "--task", "lease", "--format", "invalid"],
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "invalid_arguments"
    );
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    assert_eq!(store.events().unwrap().len(), 1);
    let empty = tempfile::tempdir().unwrap();
    let output = command(empty.path(), &["search", "lease"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "not_initialized"
    );
    assert!(!empty.path().join(".middleman").exists());
}
