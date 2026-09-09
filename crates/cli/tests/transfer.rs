#![allow(clippy::unwrap_used)]
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

fn success(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = run(repo, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn transfers_preserve_log_and_rebuild_projection() {
    let source = tempfile::tempdir().unwrap();
    success(source.path(), &["init", "--name", "transfer-source"]);
    fs::write(source.path().join("README.md"), "# Transfer fixture\n").unwrap();
    success(source.path(), &["task", "start", "--task", "README.md"]);
    let log = success(source.path(), &["export", "--format", "jsonl"]);
    let backup = source.path().join("backup.jsonl");
    success(source.path(), &["backup", backup.to_str().unwrap()]);
    assert_eq!(fs::read(&backup).unwrap(), log);
    for command in ["import", "restore"] {
        let target = tempfile::tempdir().unwrap();
        if command == "import" {
            success(target.path(), &["init", "--name", "old-project"]);
        }
        success(target.path(), &[command, backup.to_str().unwrap()]);
        assert_eq!(success(target.path(), &["export"]), log);
        let target_store =
            middleman_store::Store::open_read_only(&target.path().join(".middleman")).unwrap();
        let source_store =
            middleman_store::Store::open_read_only(&source.path().join(".middleman")).unwrap();
        assert_eq!(target_store.state().unwrap(), source_store.state().unwrap());
    }
    for format in ["cir", "markdown"] {
        assert!(!success(source.path(), &["export", "--format", format]).is_empty());
        assert_eq!(success(source.path(), &["export"]), log);
    }
}

#[test]
fn invalid_input_and_existing_backup_preserve_destination() {
    let repo = tempfile::tempdir().unwrap();
    success(repo.path(), &["init", "--name", "original"]);
    let log = success(repo.path(), &["export"]);
    let input = repo.path().join("input.jsonl");
    let tampered = String::from_utf8(log.clone())
        .unwrap()
        .replace("original", "tampered");
    for invalid in ["", "not json", &tampered] {
        fs::write(&input, invalid).unwrap();
        assert!(
            !run(repo.path(), &["import", input.to_str().unwrap()])
                .status
                .success()
        );
        assert_eq!(success(repo.path(), &["export"]), log);
        let empty = tempfile::tempdir().unwrap();
        assert!(
            !run(empty.path(), &["restore", input.to_str().unwrap()])
                .status
                .success()
        );
        assert!(!empty.path().join(".middleman").exists());
    }
    fs::write(&input, b"existing backup").unwrap();
    assert!(
        !run(repo.path(), &["backup", input.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(fs::read(&input).unwrap(), b"existing backup");
    let db = repo.path().join(".middleman/context.sqlite3");
    assert!(
        !run(repo.path(), &["backup", db.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(success(repo.path(), &["export"]), log);
}

#[test]
fn oversized_input_is_rejected_without_creating_state() {
    let repo = tempfile::tempdir().unwrap();
    let input = repo.path().join("large.jsonl");
    fs::File::create(&input)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        !run(repo.path(), &["restore", input.to_str().unwrap()])
            .status
            .success()
    );
    assert!(!repo.path().join(".middleman").exists());
}
