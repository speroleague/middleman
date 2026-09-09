//! Store integration tests: WAL mode, transactional append,
//! single-writer locking, busy retry, and crash recovery.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::{Duration, Instant};

use middleman_core::event::{Actor, Event, EventKind};
use middleman_core::ids::{EventId, Hash, ProjectId};
use time::OffsetDateTime;

use middleman_store::Error;
use middleman_store::Store;

#[test]
fn read_only_open_does_not_rebuild_or_create_state() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Store::open_read_only(dir.path()).is_err());
    assert!(!dir.path().join("context.sqlite3").exists());
    let store = Store::open(dir.path()).unwrap();
    store.append(&init_event()).unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(dir.path().join("context.sqlite3")).unwrap();
    conn.execute(
        "INSERT INTO project_meta(key, value) VALUES ('read_only_probe', '1')",
        [],
    )
    .unwrap();
    let reader = Store::open_read_only(dir.path()).unwrap();
    assert_eq!(reader.events().unwrap().len(), 1);
    let probe: String = conn
        .query_row(
            "SELECT value FROM project_meta WHERE key = 'read_only_probe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(probe, "1");
}

#[test]
fn restore_repairs_corrupt_log_and_rejects_invalid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let event = init_event();
    let store = Store::open(dir.path()).unwrap();
    store.append(&event).unwrap();
    let mut invalid = event.clone();
    invalid.hash = Hash::genesis();
    assert!(store.replace_events(&[invalid]).is_err());
    assert_eq!(store.events().unwrap(), vec![event.clone()]);
    drop(store);
    let conn = rusqlite::Connection::open(dir.path().join("context.sqlite3")).unwrap();
    conn.execute("UPDATE events SET raw = '{}'", []).unwrap();
    drop(conn);
    assert!(Store::open(dir.path()).is_err());
    Store::restore_events(dir.path(), std::slice::from_ref(&event)).unwrap();
    let restored = Store::open_read_only(dir.path()).unwrap();
    assert_eq!(restored.events().unwrap(), vec![event]);
    assert_eq!(restored.state().unwrap().project_name, "fixture");
}

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

fn project_id() -> ProjectId {
    ProjectId::new(id("proj_", 1)).expect("project id")
}

fn init_event() -> Event {
    Event::new(
        EventId::new(id("evt_", 1)).expect("event id"),
        project_id(),
        1,
        OffsetDateTime::UNIX_EPOCH,
        Actor::User,
        EventKind::ProjectInitialized {
            project_name: "fixture".into(),
        },
        Vec::new(),
        None,
        Hash::genesis(),
    )
    .expect("event seals")
}

fn indexed_event(sequence: u64, previous: Hash) -> Event {
    Event::new(
        EventId::new(id("evt_", sequence)).expect("event id"),
        project_id(),
        sequence,
        OffsetDateTime::UNIX_EPOCH,
        Actor::System,
        EventKind::SourceIndexed {
            git_commit: Some(format!("c{sequence}")),
            added: Vec::new(),
            changed: Vec::new(),
            removed: Vec::new(),
        },
        Vec::new(),
        None,
        previous,
    )
    .expect("event seals")
}

fn open_store(dir: &Path) -> Store {
    Store::open(dir).expect("store opens")
}

#[test]
fn batch_failure_rolls_back_every_event_and_projection() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let initial = init_event();
    store.append(&initial).unwrap();
    let second = indexed_event(2, initial.hash);
    let invalid = Event::new(
        EventId::new(id("evt_", 3)).unwrap(),
        project_id(),
        3,
        OffsetDateTime::UNIX_EPOCH,
        Actor::User,
        EventKind::TaskCompleted {
            task_id: middleman_core::TaskId::new(id("task_", 99)).unwrap(),
            summary: "invalid".into(),
            validation: vec![],
        },
        vec![],
        None,
        second.hash,
    )
    .unwrap();
    assert!(store.append_batch(&[second.clone(), invalid]).is_err());
    assert_eq!(store.tail().unwrap(), (1, Some(initial.hash)));
    assert_eq!(store.state().unwrap().last_indexed_commit, None);
    let third = indexed_event(3, second.hash);
    store.append_batch(&[second, third]).unwrap();
    assert_eq!(store.tail().unwrap().0, 3);
    drop(store);
    assert_eq!(Store::open(dir.path()).unwrap().events().unwrap().len(), 3);
}

#[test]
fn derived_entity_ids_survive_event_replay_and_sqlite_reopen() {
    use middleman_core::{Entity, EntityId, EntityKind, EntityPayload, Evidence, Status};

    let dir = tempfile::tempdir().unwrap();
    let initial = init_event();
    let entity = Entity::new(
        EntityId::derived("doc", &["docs/guide.md"]).unwrap(),
        EntityKind::Document,
        Status::Active,
        "Guide".into(),
        EntityPayload::Document {
            path: "docs/guide.md".into(),
            title: "Guide".into(),
            kind: "document".into(),
            authority: "observed".into(),
        },
        vec![Evidence::Document {
            path: "docs/guide.md".into(),
            content_hash: Hash::of(b"# Guide").to_hex(),
        }],
    )
    .unwrap();
    let declared = Event::new(
        EventId::new(id("evt_", 2)).unwrap(),
        project_id(),
        2,
        OffsetDateTime::UNIX_EPOCH,
        Actor::System,
        EventKind::EntityDeclared {
            entity: entity.clone(),
        },
        Vec::new(),
        None,
        initial.hash,
    )
    .unwrap();
    let events = vec![initial, declared];
    {
        let store = open_store(dir.path());
        for event in &events {
            store.append(event).unwrap();
        }
    }
    let store = open_store(dir.path());
    assert_eq!(store.events().unwrap(), events);
    assert_eq!(
        store.state().unwrap(),
        middleman_core::project(&events).unwrap()
    );
    assert_eq!(store.state().unwrap().entities[&entity.id], entity);
}

/// Spawns the sibling `store_cli` test binary, which holds the file
/// lock at `lock` for `ms` milliseconds in its own process.
fn spawn_lock_holder(lock: &Path, ms: u64) -> std::process::Child {
    let exe_dir = std::env::current_exe()
        .expect("current exe")
        .parent()
        .expect("exe dir")
        .to_path_buf();
    let current_ext = std::env::current_exe()
        .expect("current exe")
        .extension()
        .map(std::ffi::OsStr::to_os_string);
    let is_executable = |path: &Path| -> bool {
        let looks_like_helper = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("store_cli"));
        let same_extension = path.extension().map(std::ffi::OsStr::to_os_string) == current_ext;
        same_extension && looks_like_helper
    };
    let helper = std::fs::read_dir(&exe_dir)
        .expect("exe dir lists")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .find(|path| is_executable(path))
        .expect("store_cli helper exists");
    std::process::Command::new(helper)
        .arg(lock)
        .arg(ms.to_string())
        .spawn()
        .expect("helper spawns")
}

#[test]
fn open_creates_wal_database_and_schema() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(dir.path());

    let conn =
        rusqlite::Connection::open(dir.path().join("context.sqlite3")).expect("raw db opens");
    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .expect("journal mode readable");
    assert_eq!(mode.to_ascii_lowercase(), "wal");

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("sqlite master")
        .query_map([], |r| r.get(0))
        .expect("table list")
        .map(|row| row.expect("table name"))
        .collect();
    for expected in [
        "events",
        "entities",
        "edges",
        "tasks",
        "proposals",
        "retrieval",
        "project_meta",
        "schema_migrations",
    ] {
        assert!(
            tables.contains(&expected.to_string()),
            "missing table `{expected}`"
        );
    }
    assert_eq!(store.tail().expect("tail"), (0, None));
}

#[test]
fn appended_events_survive_reopen_and_match_core_fold() {
    let dir = tempfile::tempdir().expect("temp dir");
    let e1 = init_event();
    let e2 = indexed_event(2, e1.hash);
    let e3 = indexed_event(3, e2.hash);
    {
        let store = open_store(dir.path());
        store.append(&e1).expect("append one");
        store.append(&e2).expect("append two");
        store.append(&e3).expect("append three");
    }

    let store = open_store(dir.path());
    let events = store.events().expect("events read");
    assert_eq!(events.len(), 3);

    let state = store.state().expect("state reads");
    assert_eq!(state.project_name, "fixture");
    assert_eq!(state.last_indexed_commit.as_deref(), Some("c3"));
    assert_eq!(state.last_sequence, 3);
    // The persisted projection must equal a fresh fold of the log.
    assert_eq!(
        state,
        middleman_core::projection::project(&events).expect("core fold")
    );
}

#[test]
fn stale_appends_are_rejected_without_touching_the_log() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(dir.path());
    let e1 = init_event();
    store.append(&e1).expect("append one");

    let gap = indexed_event(3, e1.hash);
    assert!(matches!(
        store.append(&gap),
        Err(Error::StaleSequence {
            expected: 2,
            found: 3
        })
    ));

    let bad_chain = Event::new(
        EventId::new(id("evt_", 9)).expect("event id"),
        project_id(),
        2,
        OffsetDateTime::UNIX_EPOCH,
        Actor::System,
        EventKind::SourceIndexed {
            git_commit: None,
            added: Vec::new(),
            changed: Vec::new(),
            removed: Vec::new(),
        },
        Vec::new(),
        None,
        Hash::of(b"not the tail"),
    )
    .expect("event seals");
    assert!(matches!(store.append(&bad_chain), Err(Error::StaleHash)));

    let (sequence, _) = store.tail().expect("tail");
    assert_eq!(sequence, 1);
    let e2 = indexed_event(2, e1.hash);
    store.append(&e2).expect("valid append still works");
    assert_eq!(store.tail().expect("tail").0, 2);
}

#[test]
fn an_uncommitted_write_leaves_no_trace() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(dir.path());
    let e1 = init_event();
    store.append(&e1).expect("append one");

    // Simulate a crash: another connection writes an event inside a
    // transaction and dies (drops) without committing.
    let conn =
        rusqlite::Connection::open(dir.path().join("context.sqlite3")).expect("raw db opens");
    {
        let tx = conn.unchecked_transaction().expect("raw transaction");
        tx.execute(
            "INSERT INTO events(sequence, id, project_id, kind, proposal_id, hash, raw)
             VALUES (2, 'ghost', 'ghost', 'source_indexed', NULL, 'ghost', '{}')",
            [],
        )
        .expect("ghost write");
        // No commit — dropping `tx` rolls the write back, as SQLite does
        // after a hard crash mid-transaction.
    }

    assert_eq!(store.tail().expect("tail").0, 1);
    let e2 = indexed_event(2, e1.hash);
    store
        .append(&e2)
        .expect("append at the true tail still works");
    assert_eq!(store.tail().expect("tail").0, 2);
}

#[test]
fn a_corrupted_log_is_refused_at_open() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let store = open_store(dir.path());
        let e1 = init_event();
        store.append(&e1).expect("append one");
        let e2 = indexed_event(2, e1.hash);
        store.append(&e2).expect("append two");
    }

    // Tamper with the hash-covered `raw` JSON itself: the chain must
    // no longer verify when the store reopens.
    let conn =
        rusqlite::Connection::open(dir.path().join("context.sqlite3")).expect("raw db opens");
    conn.execute(
        "UPDATE events SET raw = json_set(raw, '$.sequence', 99) WHERE sequence = 2",
        [],
    )
    .expect("corrupts the stored event");
    drop(conn);

    assert!(matches!(Store::open(dir.path()), Err(Error::Corrupt(_))));
}

#[test]
fn append_waits_for_the_writer_lock_then_succeeds() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(dir.path());
    let e1 = init_event();
    store.append(&e1).expect("append one");

    let lock = dir.path().join("writer.lock");
    let ready = lock.with_extension("ready");
    let mut holder = spawn_lock_holder(&lock, 700);
    // Let the child win the lock before we start appending.
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            Instant::now() < ready_deadline,
            "child never reported the lock"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let started = Instant::now();
    let e2 = indexed_event(2, e1.hash);
    store
        .append(&e2)
        .expect("append succeeds after the lock frees");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(300),
        "append should have waited for the lock: {waited:?}"
    );

    assert!(holder.wait().expect("helper exits cleanly").success());
    assert_eq!(store.tail().expect("tail").0, 2);
}

#[test]
fn append_times_out_when_the_writer_lock_stays_held() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(dir.path());
    let e1 = init_event();
    store.append(&e1).expect("append one");

    let lock = dir.path().join("writer.lock");
    let ready = lock.with_extension("ready");
    let mut holder = spawn_lock_holder(&lock, 5000);
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            Instant::now() < ready_deadline,
            "child never reported the lock"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let started = Instant::now();
    let e2 = indexed_event(2, e1.hash);
    assert!(matches!(store.append(&e2), Err(Error::LockBusy)));
    assert!(
        started.elapsed()
            >= Duration::from_millis(middleman_store::WRITE_LOCK_TIMEOUT_MS.saturating_sub(200))
    );

    holder.kill().expect("helper killed");
    let _ = holder.wait();
}

#[test]
fn append_retries_sqlite_busy_until_the_other_writer_commits() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = open_store(dir.path());
    let e1 = init_event();
    store.append(&e1).expect("append one");

    // A second connection takes a write transaction and holds it for a
    // moment; the store's append must busy-retry until it commits. The
    // channel makes the handoff deterministic: the append starts only
    // after the probe write holds the SQLite write lock.
    let conn =
        rusqlite::Connection::open(dir.path().join("context.sqlite3")).expect("raw db opens");
    let (probe_tx, probe_rx) = std::sync::mpsc::channel();
    let hold = std::thread::spawn(move || {
        conn.pragma_update(None, "busy_timeout", 2000i32)
            .expect("busy timeout set");
        let tx = conn.unchecked_transaction().expect("raw transaction");
        tx.execute(
            "INSERT INTO project_meta(key, value) VALUES ('busy_probe', '1')",
            [],
        )
        .expect("probe write");
        probe_tx.send(()).expect("probe signals readiness");
        std::thread::sleep(Duration::from_millis(600));
        tx.commit().expect("probe commit");
    });

    probe_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("probe holds the write lock");
    let started = Instant::now();
    let e2 = indexed_event(2, e1.hash);
    store.append(&e2).expect("append succeeds after busy retry");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(250),
        "append should have busy-retried: {waited:?}"
    );
    hold.join().expect("holder thread ends");
    assert_eq!(store.tail().expect("tail").0, 2);
}
