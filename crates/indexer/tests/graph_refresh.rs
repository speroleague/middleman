#![allow(clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;
use std::path::{Path, PathBuf};

use middleman_core::{Config, EdgeKind, EntityId, EntityKind, Evidence, Hash};
use middleman_indexer::{
    git,
    graph::{self, Budget},
    scan::{self, SourceFile},
};

fn file(path: &str, content: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        kind: scan::classify(Path::new(path)).unwrap(),
        hash: Hash::of(content.as_bytes()),
        content: content.into(),
    }
}

fn build(files: &[SourceFile]) -> graph::Refresh {
    graph::refresh(
        None,
        files,
        None,
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap()
}

fn module(path: &str) -> EntityId {
    EntityId::derived("mod", &[path]).unwrap()
}

#[test]
fn fixture_graphs_have_stable_ids_evidence_and_no_dangling_edges() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    for fixture in [
        "rust-workspace",
        "laravel-app",
        "elm-frontend",
        "react-native-app",
    ] {
        let files = scan::scan(&root.join(fixture), &Config::default())
            .unwrap()
            .files;
        let result = build(&files);
        let graph = result.index.graph();
        let mut reversed = files.clone();
        reversed.reverse();
        assert_eq!(graph, build(&reversed).index.graph());
        assert!(
            graph
                .entities
                .values()
                .any(|e| e.kind == EntityKind::Symbol)
        );
        assert!(graph.entities.values().all(|e| !e.evidence.is_empty()));
        assert!(
            graph
                .edges
                .iter()
                .all(|edge| graph.entities.contains_key(&edge.from)
                    && graph.entities.contains_key(&edge.to)
                    && !edge.evidence.is_empty())
        );
        assert!(graph.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    }
}

#[test]
fn framework_fixtures_preserve_language_specific_symbols_and_imports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    for (fixture, symbol, from, to) in [
        (
            "rust-workspace",
            "renew",
            "crates/alpha/src/lib.rs",
            "crates/alpha/src/frontier.rs",
        ),
        (
            "laravel-app",
            "PostController",
            "routes/web.php",
            "app/Http/Controllers/PostController.php",
        ),
        ("react-native-app", "App", "App.tsx", "src/Greeting.tsx"),
    ] {
        let files = scan::scan(&root.join(fixture), &Config::default())
            .unwrap()
            .files;
        let graph = build(&files).index.graph().clone();
        assert!(
            graph
                .entities
                .values()
                .any(|entity| entity.kind == EntityKind::Symbol && entity.title == symbol),
            "{fixture}"
        );
        assert!(
            graph.edges.iter().any(|edge| {
                edge.kind == EdgeKind::Imports && edge.from == module(from) && edge.to == module(to)
            }),
            "{fixture}"
        );
    }
}

#[test]
fn document_links_and_test_dependencies_do_not_become_durable_claims() {
    let files = vec![
        file("src/a.ts", "export function renew() {}"),
        file("tests/a.test.ts", "import { renew } from '../src/a';"),
        file(
            "docs/guide.md",
            "---\nauthority: contract\n---\n# Renewal\n[implementation](../src/a.ts)\n",
        ),
    ];
    let result = build(&files);
    let graph = result.index.graph();
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Documents && e.to == module("src/a.ts"))
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Tests && e.to == module("src/a.ts"))
    );
    assert!(!graph.entities.values().any(|e| matches!(
        e.kind,
        EntityKind::Decision | EntityKind::Invariant | EntityKind::Contract
    )));
}

#[test]
fn no_op_refresh_reuses_all_facts_and_fragments() {
    let files = vec![
        file("a.ts", "import './b';\nexport function a() {}"),
        file("b.ts", "export function b() {}"),
    ];
    let initial = build(&files);
    let next = graph::refresh(
        Some(&initial.index),
        &files,
        None,
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(initial.index.graph(), next.index.graph());
    assert!(next.changes.reparsed.is_empty());
    assert!(next.changes.rederived.is_empty());
    assert_eq!(next.changes.reused.len(), files.len());
}

#[test]
fn edits_refresh_only_direct_neighbors_and_preserve_symbol_identity() {
    let mut files = vec![
        file("a.ts", "import './b';\nexport function a() {}"),
        file("b.ts", "import './c';\nexport function b() {}"),
        file("c.ts", "export function c() {}"),
        file("unrelated.ts", "export function unrelated() {}"),
    ];
    let initial = build(&files);
    let id = initial
        .index
        .graph()
        .entities
        .values()
        .find(|e| e.kind == EntityKind::Symbol && e.title == "c")
        .unwrap()
        .id
        .clone();
    files[2] = file("c.ts", "// changed line\nexport function c() {}\n");
    let next = graph::refresh(
        Some(&initial.index),
        &files,
        None,
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(next.index.graph(), build(&files).index.graph());
    assert_eq!(next.changes.reparsed, [PathBuf::from("c.ts")]);
    assert_eq!(
        next.changes.rederived,
        [PathBuf::from("b.ts"), PathBuf::from("c.ts")]
    );
    let symbol = &next.index.graph().entities[&id];
    assert!(matches!(
        &symbol.evidence[0],
        Evidence::SourceSpan { start_line: 2, .. }
    ));
}

#[test]
fn additions_deletions_and_ambiguity_match_full_rebuilds() {
    let mut files = vec![
        file("a.ts", "import './missing';"),
        file("unrelated.ts", "export const other = 1;"),
    ];
    let mut state = build(&files).index;
    for replacement in [
        Some(file("missing.ts", "export const item = 1;")),
        Some(file("missing.js", "export const item = 1;")),
        None,
    ] {
        if let Some(file) = replacement {
            files.push(file);
        } else {
            files.retain(|f| f.path != Path::new("missing.ts"));
        }
        let next = graph::refresh(
            Some(&state),
            &files,
            None,
            &Config::default().limits,
            Budget::default(),
        )
        .unwrap();
        assert_eq!(next.index.graph(), build(&files).index.graph());
        assert!(next.changes.rederived.contains(&PathBuf::from("a.ts")));
        assert!(!next.changes.reparsed.contains(&PathBuf::from("a.ts")));
        state = next.index;
    }
    files.retain(|f| f.path != Path::new("missing.js"));
    let next = graph::refresh(
        Some(&state),
        &files,
        None,
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(next.index.graph(), build(&files).index.graph());
    assert!(
        !next
            .index
            .graph()
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Imports)
    );
}

#[test]
fn invalid_snapshots_and_tight_budgets_leave_prior_state_unchanged() {
    let files = vec![file("a.rs", "pub fn a() {}")];
    let initial = build(&files);
    let expected = initial.index.graph().clone();
    let mut bad = files.clone();
    bad[0].content.push('x');
    assert!(
        graph::refresh(
            Some(&initial.index),
            &bad,
            None,
            &Config::default().limits,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        graph::refresh(
            None,
            &[files[0].clone(), files[0].clone()],
            None,
            &Config::default().limits,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        graph::refresh(
            None,
            &[file("../outside.rs", "fn x() {}")],
            None,
            &Config::default().limits,
            Budget::default()
        )
        .is_err()
    );
    assert!(
        graph::refresh(
            Some(&initial.index),
            &files,
            None,
            &Config::default().limits,
            Budget {
                max_nodes: 1,
                ..Budget::default()
            }
        )
        .is_err()
    );
    assert_eq!(initial.index.graph(), &expected);
}

#[test]
fn configuration_changes_invalidate_facts_and_cannot_bypass_limits() {
    let files = vec![file("a.rs", "pub fn a() {}\n\npub fn b() {}")];
    let initial = build(&files);
    let mut limits = Config::default().limits;
    limits.max_lines = 100;
    let next = graph::refresh(
        Some(&initial.index),
        &files,
        None,
        &limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(next.changes.reparsed.len(), 1);
    limits.max_lines = 1;
    assert!(
        graph::refresh(
            Some(&initial.index),
            &files,
            None,
            &limits,
            Budget::default()
        )
        .is_err()
    );
}

#[test]
fn cochange_evidence_is_separate_and_has_a_hard_expansion_limit() {
    let files = vec![
        file("a.rs", "fn a() {}"),
        file("b.rs", "fn b() {}"),
        file("c.rs", "fn c() {}"),
    ];
    let history = git::Snapshot {
        commits: vec![git::CommitTouch {
            sha: "a".repeat(40),
            paths: files
                .iter()
                .map(|f| f.path.clone())
                .chain([PathBuf::from("not-indexed.rs")])
                .collect(),
        }],
        history_truncated: true,
        ..git::Snapshot::default()
    };
    let next = graph::refresh(
        None,
        &files,
        Some(&history),
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(next.index.graph().cochanges.len(), 3);
    assert!(next.index.graph().history_truncated);
    assert!(
        !next
            .index
            .graph()
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::DependsOn)
    );
    assert!(
        graph::refresh(
            None,
            &files,
            Some(&history),
            &Config::default().limits,
            Budget {
                max_cochanges: 1,
                ..Budget::default()
            }
        )
        .is_err()
    );
}

#[test]
fn custom_rust_libraries_and_manifest_alias_changes_refresh_callers() {
    let mut files = vec![
        file(
            "Cargo.toml",
            "[package]\nname = 'alpha'\n[lib]\npath = 'custom/entry.rs'\n",
        ),
        file(
            "custom/entry.rs",
            "use crate::detail::renew;\npub fn entry() {}",
        ),
        file("custom/detail.rs", "pub fn renew() {}"),
        file("tests/owner.rs", "use alpha::entry;"),
    ];
    let initial = build(&files);
    assert!(
        initial
            .index
            .graph()
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Imports
                && e.from == module("custom/entry.rs")
                && e.to == module("custom/detail.rs"))
    );
    assert!(
        initial
            .index
            .graph()
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Imports
                && e.from == module("tests/owner.rs")
                && e.to == module("custom/entry.rs"))
    );
    files[0] = file(
        "Cargo.toml",
        "[package]\nname = 'beta'\n[lib]\npath = 'custom/entry.rs'\n",
    );
    let next = graph::refresh(
        Some(&initial.index),
        &files,
        None,
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(next.index.graph(), build(&files).index.graph());
    assert!(
        next.changes
            .rederived
            .contains(&PathBuf::from("tests/owner.rs"))
    );
    assert_eq!(next.changes.reparsed, [PathBuf::from("Cargo.toml")]);
}

#[test]
fn renames_remove_prior_entities_and_links_and_add_new_identities() {
    let mut files = vec![
        file("a.ts", "import './b';"),
        file("b.ts", "export function b() {}"),
    ];
    let before = build(&files);
    files[1].path = "renamed.ts".into();
    let next = graph::refresh(
        Some(&before.index),
        &files,
        None,
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert_eq!(next.index.graph(), build(&files).index.graph());
    assert_eq!(next.changes.removed, [PathBuf::from("b.ts")]);
    assert_eq!(next.changes.added, [PathBuf::from("renamed.ts")]);
    assert!(!next.index.graph().entities.contains_key(&module("b.ts")));
    assert!(
        next.index
            .graph()
            .entities
            .contains_key(&module("renamed.ts"))
    );
}

#[test]
fn unsafe_document_references_are_diagnostics_and_never_dangling_edges() {
    let result = build(&[file(
        "docs/a.md",
        "# Guide\n[escape](../../outside.rs)\n[external](https://example.invalid/path)\n[missing](missing.md)\n",
    )]);
    assert!(result.index.graph().edges.is_empty());
    assert_eq!(result.index.graph().diagnostics.len(), 3);
    assert_eq!(
        result.index.graph().diagnostics[0].reason,
        graph::Reason::Unsupported
    );
}

#[test]
fn exact_node_budget_and_new_git_history_do_not_require_reparsing() {
    let files = vec![file("a.rs", "pub fn a() {}")];
    let first = graph::refresh(
        None,
        &files,
        None,
        &Config::default().limits,
        Budget {
            max_nodes: 2,
            ..Budget::default()
        },
    )
    .unwrap();
    let history = git::Snapshot {
        history_truncated: true,
        ..git::Snapshot::default()
    };
    let next = graph::refresh(
        Some(&first.index),
        &files,
        Some(&history),
        &Config::default().limits,
        Budget::default(),
    )
    .unwrap();
    assert!(next.index.graph().history_truncated);
    assert!(next.changes.reparsed.is_empty());
    assert!(next.changes.rederived.is_empty());
    assert!(
        graph::refresh(
            Some(&first.index),
            &files,
            None,
            &Config::default().limits,
            Budget {
                max_edges: 0,
                ..Budget::default()
            }
        )
        .is_err()
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn arbitrary_edit_sequences_match_full_rebuilds(operations in prop::collection::vec((0u8..5, 0u8..8, any::<bool>()), 1..32)) {
        let mut files = std::collections::BTreeMap::new();
        let mut state = build(&[]).index;
        for (name, revision, remove) in operations {
            let path = format!("file{name}.ts");
            if remove { files.remove(&path); } else {
                let text = format!("import './file{}';\nexport function item{name}() {{ return {revision}; }}", (name + 1) % 5);
                files.insert(path.clone(), file(&path, &text));
            }
            let snapshot: Vec<_> = files.values().cloned().collect();
            let next = graph::refresh(Some(&state), &snapshot, None, &Config::default().limits, Budget::default()).unwrap();
            let full = build(&snapshot);
            prop_assert_eq!(next.index.graph(), full.index.graph());
            state = next.index;
        }
    }
}
