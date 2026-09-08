//! Deterministic indexing edge for `Middleman`.
//!
//! Six stages, each a pure function of its input: filesystem scan,
//! document scan, language scan (regex-based in v1), Git scan, derived
//! graph construction, and incremental refresh.
//!
//! Boundary rules:
//! - The only crate that reads repository files or invokes the Git CLI.
//! - Never executes repository code.
//! - All parsing is bounded by file size, line count, and time limits.
//! - Output is core types only: no filesystem handles, git handles, or
//!   raw parse state escapes this crate.
