//! stdio MCP facade for Middleman (phase 3).
//!
//! Exposes exactly three tools: `middleman_prepare`, `middleman_expand`,
//! `middleman_propose` — plus a small resource surface. No generic
//! database or filesystem tools; those would defeat the product.
//!
//! Deliberately a stub until phase 3: the CLI is the product contract
//! and must work before the facade exists.
