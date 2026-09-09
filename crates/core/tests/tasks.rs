#![allow(clippy::unwrap_used)]
use middleman_core::{
    Actor, Event, EventId, EventKind, Hash, ProjectId, TaskId,
    entity::{Task, TaskStatus},
    task::{self, Phase, Snapshot},
};
use std::collections::{BTreeMap, BTreeSet};

fn id(prefix: &str, n: u128) -> String {
    format!("{prefix}_{}", ulid::Ulid::from(n))
}
fn snapshot(entries: &[(&str, &str)]) -> Snapshot {
    Snapshot {
        files: entries
            .iter()
            .map(|(path, content)| ((*path).into(), Hash::of(content.as_bytes())))
            .collect(),
        symbols: BTreeSet::new(),
        git_commit: None,
        history_truncated: false,
    }
}
fn push(log: &mut Vec<Event>, kind: EventKind) {
    log.push(
        Event::new(
            EventId::new(id("evt", log.len() as u128 + 1)).unwrap(),
            ProjectId::new(id("proj", 1)).unwrap(),
            log.len() as u64 + 1,
            time::OffsetDateTime::UNIX_EPOCH,
            Actor::User,
            kind,
            vec![],
            None,
            log.last().map_or(Hash::genesis(), |event| event.hash),
        )
        .unwrap(),
    );
}
fn started() -> (Vec<Event>, TaskId) {
    let mut log = vec![];
    let task_id = TaskId::new(id("task", 1)).unwrap();
    push(
        &mut log,
        EventKind::ProjectInitialized {
            project_name: "fixture".into(),
        },
    );
    push(
        &mut log,
        EventKind::TaskStarted {
            task: Task {
                id: task_id.clone(),
                objective: "Work".into(),
                status: TaskStatus::InProgress,
                scope: vec![],
                validation: vec![],
                handoff: String::new(),
            },
        },
    );
    (log, task_id)
}

#[test]
fn snapshots_distinguish_endpoint_changes_and_unknown_baselines() {
    let before = snapshot(&[
        ("same.rs", "dirty already"),
        ("edit.rs", "old"),
        ("gone.rs", "old"),
    ]);
    let after = snapshot(&[
        ("same.rs", "dirty already"),
        ("edit.rs", "new"),
        ("new.rs", "new"),
    ]);
    let diff = task::compare(Some(&before), &after);
    assert_eq!(diff.added, vec![std::path::PathBuf::from("new.rs")]);
    assert_eq!(diff.changed, vec![std::path::PathBuf::from("edit.rs")]);
    assert_eq!(diff.unavailable, vec![std::path::PathBuf::from("gone.rs")]);
    assert!(!task::compare(None, &after).baseline_available);
    assert!(task::compare(None, &after).added.is_empty());
    assert!(!task::valid(&snapshot(&[("../outside", "data")])));
}

#[test]
fn terminal_and_duplicate_observation_transitions_are_rejected() {
    let (mut log, id) = started();
    let baseline = EventKind::TaskObserved {
        task_id: id.clone(),
        phase: Phase::Baseline,
        snapshot: snapshot(&[("lib.rs", "old")]),
    };
    push(&mut log, baseline.clone());
    let mut duplicate = log.clone();
    push(&mut duplicate, baseline);
    assert!(middleman_core::project(&duplicate).is_err());
    push(
        &mut log,
        EventKind::TaskObserved {
            task_id: id.clone(),
            phase: Phase::Finish,
            snapshot: snapshot(&[("lib.rs", "new")]),
        },
    );
    let completed = EventKind::TaskCompleted {
        task_id: id.clone(),
        summary: "done".into(),
        validation: vec![],
    };
    push(&mut log, completed.clone());
    let state = middleman_core::project(&log).unwrap();
    assert_eq!(state.tasks[&id].status, TaskStatus::Completed);
    assert_eq!(
        state.tasks[&id]
            .observations
            .as_ref()
            .unwrap()
            .changed
            .len(),
        1
    );
    push(&mut log, completed);
    assert!(middleman_core::project(&log).is_err());
}

#[test]
fn legacy_records_without_snapshot_fields_still_deserialize() {
    let (log, id) = started();
    let state = middleman_core::project(&log).unwrap();
    let mut value = serde_json::to_value(&state.tasks[&id]).unwrap();
    value.as_object_mut().unwrap().remove("baseline");
    value.as_object_mut().unwrap().remove("observations");
    let record: middleman_core::TaskRecord = serde_json::from_value(value).unwrap();
    assert!(record.baseline.is_none() && record.observations.is_none());
    assert_eq!(snapshot(&[]).files, BTreeMap::new());
}
