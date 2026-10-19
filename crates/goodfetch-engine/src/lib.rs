//! The engine both Products' executables run (ADR 0087): a library each
//! executable crate composes, at start, from the contributions of the
//! domain crates it links, and runs as an Attached engine over stdio, and
//! later as a Daemon. It is its own crate because both Products'
//! executables and their published SDK crates link it, and V8 must stay out
//! of a crate built without the `executable` feature (STYLE.md, reason 2).
//! It never names a Foxhound type: where Foxhound adds behaviour, it fills
//! an interface defined here.

mod composition;
mod engine;
mod info;
mod product;
pub mod rpc;
pub mod tasks;
pub mod transport;

pub use composition::{
    CallContext, CallError, Composition, CompositionError, Contribution, Handler,
};
pub use engine::Engine;
pub use info::contribution;
pub use product::{Channel, Identity};
