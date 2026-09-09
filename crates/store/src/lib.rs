//! `SQLite` persistence shell for `Middleman`.
//!
//! Single responsibility: turn the core's event and state types into
//! durable rows and back again. WAL mode, a bounded busy timeout, one
//! writer at a time (file lock around write transactions), and short
//! transactions; an event append and the projection update for that
//! event happen in a single transaction, so recovery after any crash
//! is "replay from the event log".
//!
//! The projection tables are rebuilt from the full verified log inside
//! every append transaction. For a single-repository event volume this
//! keeps the projection provably equal to `middleman_core::project(log)`
//! at all times, and crash recovery is a no-op.
//!
//! Boundary rules:
//! - The only crate that opens `.middleman/context.sqlite3`.
//! - Never decides business rules: validation happens in
//!   `middleman-core` before rows are written.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use middleman_core::projection::State;
use middleman_core::{Event, Hash, ProposalId};

/// Bounded wait for the writer lock and for busy `SQLite`.
///
/// The `SQLite` `busy_timeout` uses the same bound.
pub const WRITE_LOCK_TIMEOUT_MS: u64 = 3000;

/// Bounded busy-retry window handed to `SQLite` (milliseconds).
const BUSY_TIMEOUT_MS: i32 = 3000;

/// `.middleman/writer.lock`: held for the lifetime of every write
/// transaction. One writer process per repository (spec section 5).
const WRITER_LOCK_FILE: &str = "writer.lock";

const DB_FILE: &str = "context.sqlite3";

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("core: {0}")]
    Core(#[from] middleman_core::Error),
    #[error("the writer lock could not be acquired within {WRITE_LOCK_TIMEOUT_MS} ms")]
    LockBusy,
    #[error("event log integrity failed: {0}")]
    Corrupt(String),
    #[error("stale append: expected sequence {expected}, got {found}")]
    StaleSequence { expected: u64, found: u64 },
    #[error("event does not chain from the current log tail")]
    StaleHash,
}

/// Held open for the lifetime of a write transaction; releasing it
/// releases the repository's single-writer lock.
struct WriteLock {
    #[allow(dead_code)]
    file: File,
}

fn acquire_write_lock(path: &Path) -> Result<WriteLock, Error> {
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    let deadline = Instant::now() + Duration::from_millis(WRITE_LOCK_TIMEOUT_MS);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(WriteLock { file }),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return Err(Error::LockBusy),
        }
    }
}

/// Open handle to `.middleman/`: one `SQLite` connection in WAL mode
/// with a bounded busy timeout, a migrated schema, and projections
/// rebuilt from the verified log.
pub struct Store {
    conn: Connection,
    state_dir: PathBuf,
}

impl Store {
    /// Opens an existing store without migration or projection writes.
    /// Consumers needing a consistent snapshot should project `events()`.
    pub fn open_read_only(state_dir: &Path) -> Result<Self, Error> {
        let conn = Connection::open_with_flags(
            state_dir.join(DB_FILE),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)?;
        project_or_corrupt(&read_events(&conn)?)?;
        Ok(Self {
            conn,
            state_dir: state_dir.to_path_buf(),
        })
    }
    /// Opens (creating as needed) broker state. Verifies the event log
    /// hash chain and rebuilds projections, so opening after a crash
    /// always yields a consistent store: recovery is replay-from-log.
    pub fn open(state_dir: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(state_dir)?;
        let lock = acquire_write_lock(&state_dir.join(WRITER_LOCK_FILE))?;
        let conn = Connection::open(state_dir.join(DB_FILE))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)?;
        migrate(&conn)?;

        let tx = conn.unchecked_transaction()?;
        let events = read_events(&tx)?;
        let state = project_or_corrupt(&events)?;
        write_state(&tx, &state)?;
        tx.commit()?;
        drop(lock);

        Ok(Self {
            conn,
            state_dir: state_dir.to_path_buf(),
        })
    }

    /// Path of the `.middleman` directory this store manages.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Tail of the log: last sequence and last hash (zero/None when empty).
    pub fn tail(&self) -> Result<(u64, Option<Hash>), Error> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT sequence, hash FROM events ORDER BY sequence DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((sequence, hash)) => {
                let sequence = u64::try_from(sequence).map_err(|_| {
                    Error::Corrupt("event sequence does not fit in the store".into())
                })?;
                let hash = hash
                    .parse()
                    .map_err(|_| Error::Corrupt("tail hash is not a blake3 digest".into()))?;
                Ok((sequence, Some(hash)))
            }
            None => Ok((0, None)),
        }
    }

    /// The full event log, ordered by sequence.
    pub fn events(&self) -> Result<Vec<Event>, Error> {
        read_events(&self.conn)
    }

    /// Materialized state for commands and rendering.
    pub fn state(&self) -> Result<State, Error> {
        let mut state = State::default();
        if let Some(raw) = get_meta(&self.conn, "project_name")? {
            state.project_name = serde_json::from_str(&raw)?;
        }
        if let Some(raw) = get_meta(&self.conn, "project_id")? {
            state.project_id = serde_json::from_str(&raw)?;
        }
        if let Some(raw) = get_meta(&self.conn, "last_indexed_commit")? {
            state.last_indexed_commit = serde_json::from_str(&raw)?;
        }
        if let Some(raw) = get_meta(&self.conn, "last_sequence")? {
            state.last_sequence = serde_json::from_str(&raw)?;
        }
        if let Some(raw) = get_meta(&self.conn, "last_hash")? {
            state.last_hash = serde_json::from_str(&raw)?;
        }

        let mut stmt = self
            .conn
            .prepare("SELECT id, entity FROM entities ORDER BY id")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, raw) = row?;
            let entity: middleman_core::Entity = serde_json::from_str(&raw)?;
            let entity_id = middleman_core::EntityId::new(&id)?;
            state.entities.insert(entity_id, entity);
        }
        let mut stmt = self.conn.prepare("SELECT edge FROM edges ORDER BY id")?;
        for raw in stmt.query_map([], |r| r.get::<_, String>(0))? {
            state.edges.push(serde_json::from_str(&raw?)?);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id, task FROM tasks ORDER BY id")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, raw) = row?;
            let task: middleman_core::TaskRecord = serde_json::from_str(&raw)?;
            let task_id = middleman_core::TaskId::new(&id)?;
            state.tasks.insert(task_id, task);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id, proposal FROM proposals ORDER BY id")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, raw) = row?;
            let proposal: middleman_core::ProposalRecord = serde_json::from_str(&raw)?;
            let proposal_id = middleman_core::ProposalId::new(&id)?;
            state.proposals.insert(proposal_id, proposal);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT record FROM retrieval ORDER BY id")?;
        for raw in stmt.query_map([], |r| r.get::<_, String>(0))? {
            state.retrieval.push(serde_json::from_str(&raw?)?);
        }
        Ok(state)
    }

    /// Appends one event and updates the projection in a single
    /// transaction, under the repository's single-writer lock.
    pub fn append(&self, event: &Event) -> Result<(), Error> {
        self.append_batch(std::slice::from_ref(event))
    }

    /// Appends a bounded batch and its final projection atomically.
    pub fn append_batch(&self, batch: &[Event]) -> Result<(), Error> {
        if batch.is_empty() {
            return Ok(());
        }
        if batch.len() > 256 {
            return Err(Error::Corrupt("event batch exceeds 256 entries".into()));
        }
        let lock = acquire_write_lock(&self.state_dir.join(WRITER_LOCK_FILE))?;
        let (mut sequence, hash) = self.tail()?;
        let mut previous = hash.unwrap_or(Hash::genesis());
        let tx = self.conn.unchecked_transaction()?;
        for event in batch {
            sequence += 1;
            if event.sequence != sequence {
                return Err(Error::StaleSequence {
                    expected: sequence,
                    found: event.sequence,
                });
            }
            if event.previous_hash != previous {
                return Err(Error::StaleHash);
            }
            let raw = serde_json::to_string(event)?;
            tx.execute(
                "INSERT INTO events(sequence, id, project_id, kind, proposal_id, hash, raw)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    i64::try_from(event.sequence).map_err(|_| Error::Corrupt(
                        "event sequence does not fit in the store".into()
                    ))?,
                    event.id.as_str(),
                    event.project_id.as_str(),
                    kind_tag(event),
                    event.proposal_id.as_ref().map(ProposalId::as_str),
                    event.hash.to_hex(),
                    raw
                ],
            )?;
            previous = event.hash;
        }
        let events = read_events(&tx)?;
        rebuild_projection(&tx, &events)?;
        tx.commit()?;
        drop(lock);
        Ok(())
    }

    /// Restores a verified archive even when the existing event chain is corrupt.
    /// Physical `SQLite` corruption is still reported without removing database files.
    pub fn restore_events(state_dir: &Path, events: &[Event]) -> Result<(), Error> {
        project_or_corrupt(events)?;
        std::fs::create_dir_all(state_dir)?;
        let lock = acquire_write_lock(&state_dir.join(WRITER_LOCK_FILE))?;
        let conn = Connection::open(state_dir.join(DB_FILE))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)?;
        migrate(&conn)?;
        drop(lock);
        Self {
            conn,
            state_dir: state_dir.to_path_buf(),
        }
        .replace_events(events)
    }

    /// Replaces the log and projection only after the complete incoming chain verifies.
    pub fn replace_events(&self, events: &[Event]) -> Result<(), Error> {
        let state = project_or_corrupt(events)?;
        let lock = acquire_write_lock(&self.state_dir.join(WRITER_LOCK_FILE))?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM events", [])?;
        for event in events {
            tx.execute(
                "INSERT INTO events(sequence, id, project_id, kind, proposal_id, hash, raw)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    i64::try_from(event.sequence).map_err(|_| Error::Corrupt(
                        "event sequence does not fit in the store".into()
                    ))?,
                    event.id.as_str(),
                    event.project_id.as_str(),
                    kind_tag(event),
                    event.proposal_id.as_ref().map(ProposalId::as_str),
                    event.hash.to_hex(),
                    serde_json::to_string(event)?
                ],
            )?;
        }
        write_state(&tx, &state)?;
        tx.commit()?;
        drop(lock);
        Ok(())
    }
}

/// Reads and deserializes the whole event log in sequence order.
fn read_events(conn: &Connection) -> Result<Vec<Event>, Error> {
    let mut stmt = conn.prepare("SELECT raw FROM events ORDER BY sequence")?;
    let mut events = Vec::new();
    for raw in stmt.query_map([], |r| r.get::<_, String>(0))? {
        events.push(serde_json::from_str(&raw?)?);
    }
    Ok(events)
}

/// Projects the log, mapping core verification failures to `Corrupt`.
fn project_or_corrupt(events: &[Event]) -> Result<State, Error> {
    middleman_core::projection::project(events).map_err(|e| Error::Corrupt(e.to_string()))
}

/// Replaces the projection tables with a fresh fold of the given log.
fn rebuild_projection(conn: &Connection, events: &[Event]) -> Result<(), Error> {
    let state = project_or_corrupt(events)?;
    write_state(conn, &state)
}

fn write_state(conn: &Connection, state: &State) -> Result<(), Error> {
    conn.execute("DELETE FROM retrieval", [])?;
    conn.execute("DELETE FROM proposals", [])?;
    conn.execute("DELETE FROM tasks", [])?;
    conn.execute("DELETE FROM edges", [])?;
    conn.execute("DELETE FROM entities", [])?;
    conn.execute("DELETE FROM project_meta", [])?;

    for (id, entity) in &state.entities {
        conn.execute(
            "INSERT INTO entities(id, entity) VALUES (?1, ?2)",
            params![id.as_str(), serde_json::to_string(entity)?],
        )?;
    }
    for edge in &state.edges {
        conn.execute(
            "INSERT INTO edges(from_id, to_id, edge) VALUES (?1, ?2, ?3)",
            params![
                edge.from.as_str(),
                edge.to.as_str(),
                serde_json::to_string(edge)?
            ],
        )?;
    }
    for (id, task) in &state.tasks {
        conn.execute(
            "INSERT INTO tasks(id, task) VALUES (?1, ?2)",
            params![id.as_str(), serde_json::to_string(task)?],
        )?;
    }
    for (id, proposal) in &state.proposals {
        conn.execute(
            "INSERT INTO proposals(id, proposal) VALUES (?1, ?2)",
            params![id.as_str(), serde_json::to_string(proposal)?],
        )?;
    }
    for record in &state.retrieval {
        let signal = serde_json::to_value(record.signal)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        conn.execute(
            "INSERT INTO retrieval(node_id, signal, record) VALUES (?1, ?2, ?3)",
            params![
                record.node_id.as_str(),
                signal,
                serde_json::to_string(record)?
            ],
        )?;
    }

    set_meta(
        conn,
        "project_id",
        &serde_json::to_value(&state.project_id)?,
    )?;
    set_meta(
        conn,
        "project_name",
        &serde_json::to_value(state.project_name.clone())?,
    )?;
    set_meta(
        conn,
        "last_indexed_commit",
        &serde_json::to_value(state.last_indexed_commit.clone())?,
    )?;
    set_meta(
        conn,
        "last_sequence",
        &serde_json::to_value(state.last_sequence)?,
    )?;
    set_meta(conn, "last_hash", &serde_json::to_value(state.last_hash)?)?;
    Ok(())
}

fn set_meta(conn: &Connection, key: &str, value: &serde_json::Value) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO project_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

fn get_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM project_meta WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

/// Stable `snake_case` tag of an event kind, used for indexing.
fn kind_tag(event: &Event) -> String {
    serde_json::to_value(&event.kind)
        .ok()
        .and_then(|v| {
            v.get("kind")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}
/// One applied migration: version, name, and the script to run.
struct Migration {
    version: i64,
    name: &'static str,
    script: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial-schema",
    script: "
CREATE TABLE events (
    sequence INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    project_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    proposal_id TEXT,
    hash TEXT NOT NULL,
    raw TEXT NOT NULL
);
CREATE INDEX events_kind ON events(kind);
CREATE INDEX events_proposal ON events(proposal_id);

CREATE TABLE entities (id TEXT PRIMARY KEY, entity TEXT NOT NULL);
CREATE TABLE edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    edge TEXT NOT NULL
);
CREATE TABLE tasks (id TEXT PRIMARY KEY, task TEXT NOT NULL);
CREATE TABLE proposals (id TEXT PRIMARY KEY, proposal TEXT NOT NULL);
CREATE TABLE retrieval (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL,
    signal TEXT NOT NULL,
    record TEXT NOT NULL
);
CREATE TABLE project_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
",
}];

fn migrate(conn: &Connection) -> Result<(), Error> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        [],
    )?;
    let applied: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    for migration in MIGRATIONS {
        if applied >= migration.version {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(migration.script)?;
        tx.execute(
            "INSERT INTO schema_migrations(version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )?;
        tx.commit()?;
    }
    Ok(())
}
