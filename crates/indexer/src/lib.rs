//! Deterministic indexing edge for `Middleman`.
//!
//! Six stages with filesystem/process effects confined to adapters: filesystem scan,
//! document scan, language scan (lightweight lexical hints in v1), Git scan, derived
//! graph construction, and incremental refresh.
//!
//! Boundary rules:
//! - The only crate that reads repository files or invokes the Git CLI.
//! - Never executes repository code.
//! - All parsing is bounded by file size, line count, and time limits.
//! - Stage results are owned values with no filesystem or process handles.
//!   Source text is transient input to pure parsers, never durable memory.
//!   The final graph uses core entities, edges, and evidence.

pub mod document;
pub mod git;
pub mod graph;
pub mod language;
pub mod process;
pub mod routing;
pub mod scan;
