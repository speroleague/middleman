//! SQLite persistence shell for Middleman.
//!
//! Single responsibility: turn the core's event and state types into
//! durable rows and back again. WAL mode, a bounded busy timeout, one
//! writer at a time (file lock around write transactions), and short
//! transactions; an event append and the projection update for that
//! event happen in a single transaction, so recovery after any crash
//! is "replay from the event log".
//!
//! Boundary rules:
//! - The only crate that opens `.middleman/context.sqlite3`.
//! - Never decides business rules: validation happens in
//!   `middleman-core` before rows are written.
