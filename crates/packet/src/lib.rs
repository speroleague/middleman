//! Context packet rendering for `Middleman`.
//!
//! Turns the core's selected context into exactly three output shapes:
//! Context IR (compact lines), Markdown, and JSON. One pipeline:
//! selected nodes -> typed packet -> one renderer per format.
//!
//! Boundary rules:
//! - Token budgets are enforced here against the deterministic
//!   estimator; a renderer never overflows its budget silently.
//! - Every rendered item keeps its selection reason so `explain` can
//!   show why it was included or left out.
//! - Never invents content: renders core data only.

mod codec;
pub mod guides;
mod model;
mod selection;

pub use codec::{estimate, parse_cir, render};
pub use model::{Error, Format, Header, Item, Packet, Reference};
pub use selection::{Limits, Output, prepare};
