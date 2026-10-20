//! Protocol fuzzing (ADR 0067): calls generated from the committed JSON
//! Schema bundle, read as files, sent over a transport the caller passes,
//! as `proptest-state-machine` sequences that proptest shrinks. Every
//! invalid call must get a typed error and leave the state the caller's
//! read returns unchanged; every valid call must get its result or a typed
//! error, and its state becomes the one the next calls are checked against.

mod calls;
mod runner;
mod schema;

pub use calls::{FuzzedCall, Invalidity};
pub use runner::{Broken, ProtocolFuzz, ProtocolFuzzError, ProtocolTransport, TransportFailure};
pub use schema::{ProtocolSchema, SchemaUnreadable};

#[cfg(test)]
mod tests;
