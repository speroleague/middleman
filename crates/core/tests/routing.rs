#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use middleman_core::routing::{Context, Exclusion, Hints, Mode, Signal, classify, rank};
use middleman_core::{Edge, EdgeKind, Entity, EntityId, EntityKind, EntityPayload, Status};

fn module(name: &str) -> Entity {
    Entity::new(
        EntityId::derived("mod", &[name]).unwrap(),
        EntityKind::Module,
        Status::Active,
        name.into(),
        EntityPayload::Module {
            path: name.into(),
            language: None,
            responsibility: String::new(),
            public_surface: vec![],
        },
        vec![],
    )
    .unwrap()
}

fn edge(from: &Entity, to: &Entity, kind: EdgeKind) -> Edge {
    Edge {
        from: from.id.clone(),
        to: to.id.clone(),
        kind,
        evidence: vec![],
    }
}

#[test]
fn classification_has_boundaries_and_explicit_override() {
    assert_eq!(
        classify("Fix authentication tests", None).modes,
        BTreeSet::from([Mode::Bug, Mode::Security, Mode::Test])
    );
    assert!(classify("prefix contest", None).modes.is_empty());
    assert_eq!(
        classify("fix tests", Some(Mode::Documentation)).modes,
        BTreeSet::from([Mode::Documentation])
    );
}

#[test]
fn weights_match_specification() {
    assert_eq!(
        [
            Signal::ExactPath,
            Signal::ExactSymbol,
            Signal::ContractOrInvariant,
            Signal::ExplicitRouting,
            Signal::DirectDependency,
            Signal::DirectTest,
            Signal::RecentTask,
            Signal::CoChange,
            Signal::Recency
        ]
        .map(Signal::weight),
        [100, 90, 80, 70, 50, 45, 25, 15, 5]
    );
}

#[test]
fn explicit_expansion_keeps_target_first_and_excludes_ineligible_nodes() {
    let root = module("z.rs");
    let neighbor = module("a.rs");
    let entities = [root.clone(), neighbor.clone()]
        .into_iter()
        .map(|entity| (entity.id.clone(), entity))
        .collect();
    let edges = [edge(&root, &neighbor, EdgeKind::Imports)];
    let mut hints = BTreeMap::new();
    let result = middleman_core::routing::expand(
        &root.id,
        &Context {
            entities: &entities,
            edges: &edges,
            hints: &hints,
            cochanges: &[],
        },
    );
    assert_eq!(result.candidates[0].id, root.id);
    assert_eq!(result.candidates[0].score, 70);
    assert_eq!(result.candidates[1].score, 50);
    hints.insert(
        root.id.clone(),
        Hints {
            ignored: true,
            ..Hints::default()
        },
    );
    let result = middleman_core::routing::expand(
        &root.id,
        &Context {
            entities: &entities,
            edges: &edges,
            hints: &hints,
            cochanges: &[],
        },
    );
    assert!(result.candidates.is_empty() && result.low_confidence);
}

#[test]
fn paths_are_exact_case_sensitive_and_portable() {
    let entity = module("src/lib.rs");
    let entities = BTreeMap::from([(entity.id.clone(), entity)]);
    let hints = BTreeMap::new();
    let context = Context {
        entities: &entities,
        edges: &[],
        hints: &hints,
        cochanges: &[],
    };
    assert_eq!(
        rank("fix `src\\lib.rs`", None, &context, 10).candidates[0].score,
        100
    );
    for request in ["other/src/lib.rs", "src/lib.rs.bak", "SRC/lib.rs", ""] {
        let result = rank(request, None, &context, 10);
        assert!(result.candidates.is_empty(), "{request}");
        assert!(result.low_confidence);
    }
}

#[test]
fn neighbors_are_one_hop_and_duplicate_evidence_does_not_multiply_scores() {
    let a = module("a.rs");
    let b = module("b.rs");
    let c = module("c.rs");
    let edges = vec![
        edge(&a, &a, EdgeKind::Imports),
        edge(&a, &b, EdgeKind::Imports),
        edge(&a, &b, EdgeKind::Imports),
        edge(&b, &c, EdgeKind::Imports),
    ];
    let entities = [a.clone(), b.clone(), c]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let hints = BTreeMap::new();
    let pairs = [
        (a.id.clone(), a.id.clone()),
        (a.id.clone(), b.id.clone()),
        (b.id.clone(), a.id.clone()),
    ];
    let context = Context {
        entities: &entities,
        edges: &edges,
        hints: &hints,
        cochanges: &pairs,
    };
    let result = rank("a.rs", None, &context, 10);
    assert_eq!(result.candidates.len(), 2);
    assert_eq!(result.candidates[0].score, 100);
    assert_eq!(result.candidates[1].id, b.id);
    assert_eq!(result.candidates[1].score, 65);
}

#[test]
fn ignored_inactive_and_stale_nodes_do_not_seed_neighbors() {
    let a = module("a.rs");
    let b = module("b.rs");
    let edges = [edge(&a, &b, EdgeKind::Tests)];
    for status in [
        Status::Proposed,
        Status::Rejected,
        Status::Superseded,
        Status::Active,
    ] {
        let mut source = a.clone();
        source.status = status;
        let entities = BTreeMap::from([(source.id.clone(), source), (b.id.clone(), b.clone())]);
        let hints = BTreeMap::from([(
            a.id.clone(),
            Hints {
                ignored: status == Status::Active,
                ..Hints::default()
            },
        )]);
        let result = rank(
            "a.rs",
            None,
            &Context {
                entities: &entities,
                edges: &edges,
                hints: &hints,
                cochanges: &[],
            },
            10,
        );
        assert!(result.candidates.is_empty());
        assert_eq!(
            result.excluded[&a.id],
            if status == Status::Active {
                Exclusion::Ignored
            } else {
                Exclusion::Inactive
            }
        );
    }
    let entities = BTreeMap::from([(a.id.clone(), a.clone()), (b.id.clone(), b)]);
    let hints = BTreeMap::from([(
        a.id.clone(),
        Hints {
            stale_penalty: 50,
            ..Hints::default()
        },
    )]);
    let result = rank(
        "a.rs",
        None,
        &Context {
            entities: &entities,
            edges: &edges,
            hints: &hints,
            cochanges: &[],
        },
        10,
    );
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].score, 50);
    assert!(result.low_confidence);
}

#[test]
fn routing_history_and_limits_are_explicit() {
    let a = module("a.rs");
    let b = module("b.rs");
    let entities = [b, a]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect::<BTreeMap<_, _>>();
    let hints = entities
        .keys()
        .map(|id| {
            (
                id.clone(),
                Hints {
                    routing_phrases: vec!["AUTH-42".into(), "AUTH-42".into()],
                    recent_task: true,
                    recent: true,
                    modes: BTreeSet::from([Mode::Security]),
                    ..Hints::default()
                },
            )
        })
        .collect();
    let context = Context {
        entities: &entities,
        edges: &[],
        hints: &hints,
        cochanges: &[],
    };
    let result = rank("fix AUTH-42", None, &context, 1);
    assert_eq!(result.candidates[0].score, 100);
    assert_eq!(result.candidates[0].id, *entities.keys().next().unwrap());
    assert!(result.truncated);
    assert_eq!(result.confidence, 70);
    assert_eq!(
        rank("", Some(Mode::Security), &context, 10).candidates[0].score,
        100
    );
    assert!(rank("security", None, &context, 10).low_confidence);
    let empty = rank("AUTH-42", None, &context, 0);
    assert!(empty.truncated && empty.low_confidence && empty.candidates.is_empty());
}

#[test]
fn symbols_contracts_and_test_links_have_distinct_reasons() {
    let mut symbol = module("src/a.rs");
    symbol.kind = EntityKind::Symbol;
    symbol.title = "renew".into();
    symbol.payload = EntityPayload::Symbol {
        path: "src/a.rs".into(),
        span: None,
        symbol_kind: "function".into(),
        signature: None,
        visibility: "public".into(),
    };
    let contract = Entity::new(
        EntityId::derived("con", &["lease"]).unwrap(),
        EntityKind::Contract,
        Status::Active,
        "Lease expiry".into(),
        EntityPayload::Contract {
            what: "lease".into(),
            input: String::new(),
            output: String::new(),
            compatibility: None,
            owner: None,
            path: None,
        },
        vec![],
    )
    .unwrap();
    let test = module("test.rs");
    let edges = [edge(&test, &symbol, EdgeKind::Tests)];
    let entities = [symbol.clone(), contract.clone(), test.clone()]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let hints = BTreeMap::new();
    let context = Context {
        entities: &entities,
        edges: &edges,
        hints: &hints,
        cochanges: &[],
    };
    let result = rank("renew: LEASE EXPIRY", None, &context, 10);
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|c| c.score)
            .collect::<Vec<_>>(),
        [90, 80, 45]
    );
    assert!(rank("renewal", None, &context, 10).candidates.is_empty());
}
