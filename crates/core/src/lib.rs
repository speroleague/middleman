//! Pure domain core of Middleman.
//!
//! This crate owns every domain type the rest of the system persists,
//! indexes, or renders: the event model with its hash chain, the
//! projection fold, entity and edge types, evidence, configuration,
//! task classification, retrieval scoring, packet selection, and
//! proposal validation.
//!
//! Boundary rules:
//! - No I/O: no filesystem, no sockets, no process execution, no Git.
//! - No clocks: time arrives as data, never read here.
//! - No hidden randomness: identifiers are built deterministically from
//!   caller-supplied input.
//! - Pure functions in, owned types out; untrusted input is rejected at
//!   this boundary with typed errors, never a panic.
