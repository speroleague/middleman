#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use middleman_core::{Config, Hash};
use middleman_indexer::document::parse_document;
use middleman_indexer::scan::{FileKind, ScanBudget, ScanError, scan, scan_with_budget};

fn write(root: &Path, path: &str, text: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn paths(root: &Path, config: &Config) -> Vec<PathBuf> {
    scan(root, config)
        .unwrap()
        .files
        .into_iter()
        .map(|f| f.path)
        .collect()
}

#[test]
fn fixtures_are_sorted_repeatable_and_have_real_indexing_signals() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    for fixture in ["rust-workspace", "laravel-app", "elm-frontend"] {
        let root = fixtures.join(fixture);
        let first = scan(&root, &Config::default()).unwrap();
        assert_eq!(first, scan(&root, &Config::default()).unwrap());
        assert!(
            first
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(first.files.iter().any(|f| f.kind == FileKind::Source));
        assert!(first.files.iter().all(|f| !f.content.is_empty()));
    }

    let root = fixtures.join("rust-workspace");
    let report = scan(&root, &Config::default()).unwrap();
    let source = report
        .files
        .iter()
        .find(|f| f.path.ends_with("frontier.rs"))
        .unwrap();
    assert!(source.content.contains("pub fn renew"));
    assert_eq!(source.hash, Hash::of(source.content.as_bytes()));
    assert!(report.files.iter().any(|f| f.kind == FileKind::Test));

    let source = report
        .files
        .iter()
        .find(|f| f.path.ends_with("architecture.md"))
        .unwrap();
    let doc = parse_document(&source.content, &Config::default().limits).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Frontier architecture"));
    assert_eq!(doc.frontmatter["id"], "frontier-architecture");
    assert_eq!(doc.routing.len(), 2);
    assert_eq!(doc.routing[0].condition, "lease renewal");
    assert_eq!(
        doc.routing[0].references,
        ["docs/contracts/leases.md", "crates/alpha/src/frontier.rs"]
    );
    assert!(
        doc.links
            .iter()
            .any(|link| link.target == "contracts/leases.md#invariants")
    );
    assert!(
        !doc.links
            .iter()
            .any(|link| link.target == "not-a-real-document.md")
    );
    assert_eq!(doc.headings.len(), 3);
}

#[test]
fn layered_ignore_rules_work_without_git_metadata_and_cannot_override_safety() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, ".gitignore", "*.rs\n!keep.rs\nblocked/\n");
    write(root, "src/.gitignore", "!nested.rs\n");
    write(root, ".middleman/ignore", "broker.md\n!target/\n");
    for name in [
        "keep.rs",
        "skip.rs",
        "src/nested.rs",
        "src/skip.rs",
        "blocked/keep.rs",
        "broker.md",
        "target/keep.rs",
        "vendor/keep.rs",
        ".env",
        "identity.key",
        "secrets.toml",
        "credentials.yaml",
        ".git-credentials",
        "allowed.md",
        "excluded.md",
    ] {
        write(root, name, "synthetic fixture");
    }
    let mut config = Config::default();
    config.ignore.include = vec!["*".into()];
    config.ignore.exclude = vec!["excluded.md".into()];
    assert_eq!(
        paths(root, &config),
        ["allowed.md", "keep.rs", "src/nested.rs"].map(PathBuf::from)
    );
}

#[test]
fn bounded_reads_skip_binary_large_and_long_files_and_total_limits_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "good.rs", "pub fn good() {}\n");
    write(root, "binary.rs", [0, 1, 2]);
    write(root, "invalid.rs", [255, 254]);
    write(root, "large.rs", vec![b'x'; 1025]);
    write(root, "long.rs", "a\nb\nc\n");
    let mut config = Config::default();
    config.limits.max_file_kb = 1;
    config.limits.max_lines = 2;
    let report = scan(root, &config).unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.skipped.len(), 4);
    assert!(matches!(
        scan_with_budget(
            root,
            &config,
            ScanBudget {
                max_total_bytes: 1,
                max_entries: 100
            }
        ),
        Err(ScanError::BudgetExceeded(_))
    ));
    assert!(matches!(
        scan_with_budget(
            root,
            &config,
            ScanBudget {
                max_total_bytes: 10000,
                max_entries: 1
            }
        ),
        Err(ScanError::BudgetExceeded(_))
    ));
    config.limits.time_budget_secs = 0;
    assert!(matches!(
        scan(root, &config),
        Err(ScanError::BudgetExceeded(_))
    ));
}

#[test]
fn depth_and_invalid_rules_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "root.rs", "root");
    write(dir.path(), "deep/nested.rs", "nested");
    let mut config = Config::default();
    config.limits.max_depth = 0;
    assert_eq!(paths(dir.path(), &config), [PathBuf::from("root.rs")]);
    write(dir.path(), ".gitignore", "[z-a]\n");
    assert!(matches!(
        scan(dir.path(), &config),
        Err(ScanError::InvalidRules)
    ));
}

#[test]
fn document_parser_ignores_fenced_examples_and_enforces_bounds() {
    let mut config = Config::default();
    let text =
        "# Real\r\n~~~md\r\n# Fake\r\n~~~\r\n## Next\r\n[read](docs/a.md) `mod:frontier`\r\n";
    let doc = parse_document(text, &config.limits).unwrap();
    assert_eq!(doc.headings.len(), 2);
    assert_eq!(doc.headings[1].line, 5);
    assert_eq!(doc.identifiers, ["mod:frontier"]);
    config.limits.max_lines = 1;
    assert!(parse_document(text, &config.limits).is_err());
}

#[cfg(unix)]
#[test]
fn links_and_linked_ignore_files_are_never_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "outside.rs", "outside");
    write(outside.path(), "rules", "*.rs");
    symlink(outside.path(), dir.path().join("linked")).unwrap();
    symlink(
        outside.path().join("outside.rs"),
        dir.path().join("linked.rs"),
    )
    .unwrap();
    symlink(outside.path().join("rules"), dir.path().join(".gitignore")).unwrap();
    assert!(scan(dir.path(), &Config::default()).is_err());
    fs::remove_file(dir.path().join(".gitignore")).unwrap();
    assert!(paths(dir.path(), &Config::default()).is_empty());
}

#[cfg(windows)]
#[test]
fn windows_junctions_are_not_traversed() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "outside.rs", "synthetic fixture");
    let link = dir.path().join("linked");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:MIDDLEMAN_TEST_LINK -Target $env:MIDDLEMAN_TEST_TARGET | Out-Null"])
        .env("MIDDLEMAN_TEST_LINK", &link)
        .env("MIDDLEMAN_TEST_TARGET", outside.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "junction setup failed");
    let report = scan(dir.path(), &Config::default()).unwrap();
    assert!(report.files.is_empty());
    assert_eq!(
        report.skipped[0].reason,
        middleman_indexer::scan::SkipReason::LinkedOrSpecial
    );
    assert!(matches!(
        scan(&link, &Config::default()),
        Err(ScanError::InvalidRoot)
    ));
    fs::remove_dir(&link).unwrap();
    assert!(outside.path().join("outside.rs").exists());
}

#[test]
fn changed_content_changes_its_hash_without_disturbing_other_files() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "first");
    write(dir.path(), "b.rs", "stable");
    let before = scan(dir.path(), &Config::default()).unwrap();
    write(dir.path(), "a.rs", "second");
    let after = scan(dir.path(), &Config::default()).unwrap();
    assert_ne!(before.files[0].hash, after.files[0].hash);
    assert_eq!(before.files[1], after.files[1]);
}

#[test]
fn ignore_files_are_bounded_and_broker_denials_beat_nested_git_allow_rules() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".middleman/ignore", "private.md\n");
    write(dir.path(), "docs/.gitignore", "!private.md\n");
    write(dir.path(), "docs/private.md", "synthetic fixture");
    write(dir.path(), "docs/public.md", "public fixture");
    assert_eq!(
        paths(dir.path(), &Config::default()),
        [PathBuf::from("docs/public.md")]
    );
    write(dir.path(), ".middleman/ignore", vec![b'x'; 1025]);
    let mut config = Config::default();
    config.limits.max_file_kb = 1;
    assert!(matches!(
        scan(dir.path(), &config),
        Err(ScanError::InvalidRules)
    ));
}
