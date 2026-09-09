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

fn success(repo: &Path, args: &[&str]) -> Output {
    let output = run(repo, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn preview_and_write_preserve_user_text_and_do_not_append_events() {
    let repo = tempfile::tempdir().unwrap();
    success(repo.path(), &["init"]);
    fs::write(
        repo.path().join("AGENTS.md"),
        "User rules\r\nKeep these.\r\n",
    )
    .unwrap();
    let preview = success(repo.path(), &["render", "agents-md", "--format", "json"]);
    let preview: Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join("AGENTS.md")).unwrap(),
        "User rules\r\nKeep these.\r\n"
    );
    assert!(
        preview["content"]
            .as_str()
            .unwrap()
            .contains("middleman prepare")
    );
    success(repo.path(), &["render", "agents-md", "--write"]);
    let first = fs::read(repo.path().join("AGENTS.md")).unwrap();
    assert!(first.starts_with(b"User rules\r\nKeep these.\r\n"));
    let output = success(
        repo.path(),
        &["render", "agents-md", "--write", "--format", "json"],
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["changed"],
        false
    );
    assert_eq!(fs::read(repo.path().join("AGENTS.md")).unwrap(), first);
    let store = middleman_store::Store::open_read_only(&repo.path().join(".middleman")).unwrap();
    assert_eq!(store.events().unwrap().len(), 1);
}

#[test]
fn context_map_is_bounded_and_does_not_index_itself_on_regeneration() {
    let repo = tempfile::tempdir().unwrap();
    success(repo.path(), &["init", "--name", "Guide fixture"]);
    fs::create_dir(repo.path().join("src")).unwrap();
    for index in 0..12 {
        fs::write(
            repo.path().join(format!("src/module{index}.rs")),
            "pub fn example() {}\n",
        )
        .unwrap();
    }
    fs::create_dir(repo.path().join("docs")).unwrap();
    fs::write(repo.path().join("docs/architecture.md"), "# Architecture\n").unwrap();
    fs::write(
        repo.path().join("docs/agent-context.md"),
        "# Curated context\n\nUser architecture.\n",
    )
    .unwrap();
    success(repo.path(), &["render", "agent-context", "--write"]);
    let first = fs::read_to_string(repo.path().join("docs/agent-context.md")).unwrap();
    assert!(first.starts_with("# Curated context\n\nUser architecture.\n"));
    assert!(first.contains("4 additional entries omitted"));
    assert!(!first.contains("doc:docs/agent"));
    assert!(!first.contains("pub fn example"));
    success(repo.path(), &["render", "agent-context", "--write"]);
    assert_eq!(
        fs::read_to_string(repo.path().join("docs/agent-context.md")).unwrap(),
        first
    );
}

#[test]
fn invalid_markers_and_unsafe_destination_leave_originals_untouched() {
    let repo = tempfile::tempdir().unwrap();
    success(repo.path(), &["init"]);
    let original = "User rules\n<!-- middleman:begin -->\nIncomplete\n";
    fs::write(repo.path().join("AGENTS.md"), original).unwrap();
    let output = run(repo.path(), &["render", "agents-md", "--write"]);
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "invalid_guide"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("AGENTS.md")).unwrap(),
        original
    );
    fs::write(repo.path().join("docs"), "not a directory").unwrap();
    let output = run(repo.path(), &["render", "agent-context", "--write"]);
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "unsafe_output_path"
    );
}

#[test]
fn context_write_creates_missing_docs_directory() {
    let repo = tempfile::tempdir().unwrap();
    success(repo.path(), &["init"]);
    success(repo.path(), &["render", "agent-context", "--write"]);
    assert!(repo.path().join("docs/agent-context.md").is_file());
}

#[test]
#[cfg(any(windows, unix))]
fn linked_docs_directory_cannot_redirect_writes() {
    let repo = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    success(repo.path(), &["init"]);
    fs::write(outside.path().join("agent-context.md"), "outside content").unwrap();
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(repo.path().join("docs"))
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "junction creation failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), repo.path().join("docs")).unwrap();
    let output = run(repo.path(), &["render", "agent-context", "--write"]);
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "unsafe_output_path"
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("agent-context.md")).unwrap(),
        "outside content"
    );
}
