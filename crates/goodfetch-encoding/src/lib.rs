//! The Encoding a document's own bytes declare, read as Chrome reads it
//! (ADR 0119): the essence and charset of its `Content-Type` value,
//! Chrome's `<meta>` prescan for HTML and the XML declaration. Pure: no
//! I/O and no task, so a benchmark compiles it in (ADR 0071).
//!
//! Its own crate because the compiler must keep one reading on both sides
//! of a dependency direction (STYLE.md, reason 2): Foxhound's fetch tiers,
//! under `crates/foxhound/`, and Collie's client crate, which no Foxhound
//! crate may reach (ADR 0087), each read a body's Encoding with it.

mod declaration;
mod media_type;

pub use declaration::{Prescanned, UNCONDITIONAL, declared, prescan, prescanned, xml_declaration};
pub use media_type::{Kind, content_type};
