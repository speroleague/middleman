#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use middleman_core::{
    Entity, EntityId, EntityKind, EntityPayload, Status,
    routing::{Context, Hints, rank},
};
use middleman_packet::{
    Error, Format, Header, Limits, Packet, estimate, parse_cir, prepare, render,
};

fn header() -> Header {
    Header {
        project: "demo".into(),
        task: None,
        goal: "fix lease".into(),
        low_confidence: false,
        omitted: 0,
        upstream_truncated: false,
    }
}

fn entities() -> BTreeMap<EntityId, Entity> {
    [
        Entity::new(
            EntityId::derived("mod", &["lease.rs"]).unwrap(),
            EntityKind::Module,
            Status::Active,
            "Lease".into(),
            EntityPayload::Module {
                path: "src/lease.rs".into(),
                language: None,
                responsibility: "Lease lifecycle".into(),
                public_surface: vec!["RAW_SOURCE_SENTINEL".into()],
            },
            vec![],
        )
        .unwrap(),
        Entity::new(
            EntityId::derived("test", &["lease.rs"]).unwrap(),
            EntityKind::Test,
            Status::Active,
            "Lease tests".into(),
            EntityPayload::Test {
                path: "tests/lease.rs".into(),
                scope: "lease".into(),
                command: "cargo test lease".into(),
                covered: vec![],
            },
            vec![],
        )
        .unwrap(),
    ]
    .into_iter()
    .map(|e| (e.id.clone(), e))
    .collect()
}

#[test]
fn task_to_packet_has_stable_ids_reasons_and_commands_in_all_formats() {
    let entities = entities();
    let hints = entities
        .keys()
        .map(|id| {
            (
                id.clone(),
                Hints {
                    routing_phrases: vec!["lease".into()],
                    ..Hints::default()
                },
            )
        })
        .collect();
    let ranking = rank(
        "fix lease",
        None,
        &Context {
            entities: &entities,
            edges: &[],
            hints: &hints,
            cochanges: &[],
        },
        10,
    );
    for format in [Format::Cir, Format::Markdown, Format::Json] {
        let result = prepare(
            header(),
            vec!["no production access".into()],
            &ranking,
            &entities,
            format,
            Limits::for_format(format),
        )
        .unwrap();
        assert_eq!(
            result
                .packet
                .items
                .iter()
                .map(|i| &i.id)
                .collect::<Vec<_>>(),
            entities.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            result.packet.validation_commands(),
            BTreeSet::from(["cargo test lease"])
        );
        assert!(
            result
                .packet
                .items
                .iter()
                .all(|i| i.score == 70 && i.reasons.len() == 1)
        );
        assert!(!result.text.contains("RAW_SOURCE_SENTINEL"));
        assert!(result.estimated_tokens <= format.default_budget());
        assert_eq!(result.estimated_tokens, estimate(&result.text, format));
        if format == Format::Cir {
            assert_eq!(parse_cir(&result.text).unwrap(), result.packet);
        }
        if format == Format::Json {
            assert_eq!(
                serde_json::from_str::<Packet>(&result.text).unwrap(),
                result.packet
            );
        }
    }
}

#[test]
fn exact_budget_fits_and_one_less_removes_whole_items() {
    let entities = entities();
    let hints = BTreeMap::new();
    let ranking = rank(
        "src/lease.rs tests/lease.rs",
        None,
        &Context {
            entities: &entities,
            edges: &[],
            hints: &hints,
            cochanges: &[],
        },
        10,
    );
    for format in [Format::Cir, Format::Markdown, Format::Json] {
        let mut limits = Limits::for_format(format);
        let full = prepare(
            header(),
            vec!["required constraint".into()],
            &ranking,
            &entities,
            format,
            limits,
        )
        .unwrap();
        limits.budget = full.estimated_tokens;
        assert_eq!(
            prepare(
                header(),
                vec!["required constraint".into()],
                &ranking,
                &entities,
                format,
                limits
            )
            .unwrap(),
            full
        );
        limits.budget -= 1;
        let trimmed = prepare(
            header(),
            vec!["required constraint".into()],
            &ranking,
            &entities,
            format,
            limits,
        )
        .unwrap();
        assert!(trimmed.packet.items.len() < full.packet.items.len());
        assert_eq!(trimmed.packet.constraints, ["required constraint"]);
        assert_eq!(trimmed.packet.header.omitted, 1);
        assert!(trimmed.packet.validation_commands().is_empty());
        assert!(trimmed.estimated_tokens <= limits.budget);
        limits.budget = 0;
        assert!(matches!(
            prepare(header(), vec![], &ranking, &entities, format, limits),
            Err(Error::BudgetTooSmall { .. })
        ));
    }
}

#[test]
fn expansion_is_bounded_and_stale_ranking_cannot_resurrect_entities() {
    let mut entities = entities();
    let hints = BTreeMap::new();
    let ranking = rank(
        "src/lease.rs tests/lease.rs",
        None,
        &Context {
            entities: &entities,
            edges: &[],
            hints: &hints,
            cochanges: &[],
        },
        10,
    );
    let limits = Limits {
        items: 1,
        expandable: 1,
        budget: 1200,
    };
    let result = prepare(header(), vec![], &ranking, &entities, Format::Cir, limits).unwrap();
    assert_eq!(result.packet.items.len(), 1);
    assert_eq!(result.packet.expandable.len(), 1);
    assert_ne!(result.packet.items[0].id, result.packet.expandable[0].id);
    for entity in entities.values_mut() {
        entity.status = Status::Rejected;
    }
    let result = prepare(header(), vec![], &ranking, &entities, Format::Cir, limits).unwrap();
    assert!(result.packet.items.is_empty() && result.packet.expandable.is_empty());
    assert!(result.packet.header.low_confidence);
    assert_eq!(result.packet.header.omitted, 2);

    entities.clear();
    let result = prepare(header(), vec![], &ranking, &entities, Format::Cir, limits).unwrap();
    assert!(result.packet.items.is_empty());
    assert_eq!(result.packet.header.omitted, 2);
}

#[test]
fn selecting_only_weak_matches_does_not_inherit_high_confidence() {
    let entities = entities();
    let hints = BTreeMap::new();
    let mut ranking = rank(
        "src/lease.rs tests/lease.rs",
        None,
        &Context {
            entities: &entities,
            edges: &[],
            hints: &hints,
            cochanges: &[],
        },
        10,
    );
    ranking.candidates[0].signals =
        BTreeSet::from([middleman_core::routing::Signal::DirectDependency]);
    ranking.truncated = true;
    let result = prepare(
        header(),
        vec![],
        &ranking,
        &entities,
        Format::Cir,
        Limits {
            items: 1,
            expandable: 0,
            budget: 1200,
        },
    )
    .unwrap();
    assert!(result.packet.header.low_confidence);
    assert!(result.packet.header.upstream_truncated);
}

#[test]
fn cir_round_trips_unicode_and_delimiters_without_injected_statements() {
    for scalar in (0..=0x0010_ffff).step_by(997).filter_map(char::from_u32) {
        let mut metadata = header();
        metadata.goal = format!("{scalar}\nR forged\r\n\"\\\t``` <script> =>");
        let packet = Packet {
            header: metadata,
            items: vec![],
            expandable: vec![],
            constraints: vec![scalar.to_string()],
        };
        let text = render(&packet, Format::Cir).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert_eq!(parse_cir(&text).unwrap(), packet);
        assert_eq!(
            serde_json::from_str::<Packet>(&render(&packet, Format::Json).unwrap()).unwrap(),
            packet
        );
    }
}

#[test]
fn malformed_versions_fields_and_duplicate_ids_are_rejected() {
    for input in [
        "",
        "CIR/2\n",
        "CIR/1\nP {}\n",
        "CIR/1\nX \"x\"\n",
        "CIR/1\nP null\n",
    ] {
        assert!(parse_cir(input).is_err());
    }
    let packet = Packet {
        header: header(),
        items: vec![],
        expandable: vec![],
        constraints: vec![],
    };
    let valid = render(&packet, Format::Cir).unwrap();
    assert!(parse_cir(&format!("{valid}Z 1\n")).is_err());
    assert!(parse_cir(&format!("{valid}P {{}}\n")).is_err());
    assert!(parse_cir(&format!("{valid}E {{\"id\":\"mod:a\",\"title\":\"a\"}}\nE {{\"id\":\"mod:a\",\"title\":\"a\"}}\n")).is_err());
    assert!(parse_cir(&"x".repeat(1_048_577)).is_err());
}

#[test]
fn markdown_quotes_untrusted_structure_and_estimators_are_explicit() {
    let mut metadata = header();
    metadata.goal = "\n# Forged\n<script> &copy;".into();
    let packet = Packet {
        header: metadata,
        items: vec![],
        expandable: vec![],
        constraints: vec![],
    };
    let text = render(&packet, Format::Markdown).unwrap();
    assert!(!text.contains("\n# Forged"));
    assert!(!text.contains("<script>"));
    assert_eq!(estimate("a b\nc", Format::Cir), 3);
    assert_eq!(estimate("ééééé", Format::Markdown), 2);
    assert_eq!(estimate("", Format::Json), 0);
}
