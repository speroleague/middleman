//! Property tests for the projection fold (spec section 15):
//! projection is deterministic and the resulting state round-trips
//! through its serialization form.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;
use time::OffsetDateTime;

fn ulid_bytes(sequence: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    let ts = 1_700_000_000_000u64 + sequence;
    bytes[..6].copy_from_slice(&ts.to_be_bytes()[2..]);
    bytes[6..14].copy_from_slice(&sequence.to_be_bytes());
    bytes
}

fn id(prefix: &str, sequence: u64) -> String {
    format!("{prefix}{}", ulid::Ulid::from_bytes(ulid_bytes(sequence)))
}

fn build_log(count: usize, label: &str) -> Vec<middleman_core::event::Event> {
    use middleman_core::entity::{Task, TaskStatus};
    use middleman_core::event::{Actor, Event, EventKind, RetrievalSignal};
    use middleman_core::ids::{EntityId, EventId, Hash, ProjectId, TaskId};

    let project_id = ProjectId::new(id("proj_", 1)).unwrap();
    let init = Event::new(
        EventId::new(id("evt_", 0)).unwrap(),
        project_id.clone(),
        1,
        OffsetDateTime::UNIX_EPOCH,
        Actor::User,
        EventKind::ProjectInitialized {
            project_name: label.to_owned(),
        },
        Vec::new(),
        None,
        Hash::genesis(),
    )
    .unwrap();
    let mut events = vec![init];
    for i in 1..=count {
        let n = i as u64;
        let kind = match i % 3 {
            0 => EventKind::SourceIndexed {
                git_commit: Some(label.to_owned()),
                added: Vec::new(),
                changed: Vec::new(),
                removed: Vec::new(),
            },
            1 => EventKind::TaskStarted {
                task: Task {
                    id: TaskId::new(id("task_", 500 + n)).unwrap(),
                    objective: label.to_owned(),
                    status: TaskStatus::InProgress,
                    scope: Vec::new(),
                    validation: Vec::new(),
                    handoff: "none".into(),
                },
            },
            _ => EventKind::RetrievalObserved {
                task_id: None,
                node_id: EntityId::new(id("ent_", 600 + n)).unwrap(),
                signal: RetrievalSignal::Retrieved,
            },
        };
        let previous_hash = events.last().expect("log has an init event").hash;
        events.push(
            Event::new(
                EventId::new(id("evt_", 100 + n)).unwrap(),
                project_id.clone(),
                1 + n,
                OffsetDateTime::UNIX_EPOCH,
                Actor::System,
                kind,
                Vec::new(),
                None,
                previous_hash,
            )
            .unwrap(),
        );
    }
    events
}

proptest! {
    #[test]
    fn projection_is_deterministic_and_serde_stable(
        count in 1usize..=12,
        label in "[a-z0-9]{0,10}",
    ) {
        let log = build_log(count, &label);
        let state = middleman_core::projection::project(&log)
            .expect("a well-formed log always projects");
        let repeated = middleman_core::projection::project(&log)
            .expect("a well-formed log always projects");
        prop_assert_eq!(&state, &repeated);

        let json = serde_json::to_string(&state).expect("state serializes");
        let restored: middleman_core::projection::State =
            serde_json::from_str(&json).expect("state deserializes");
        prop_assert_eq!(state, restored);
    }
}
