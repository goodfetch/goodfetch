//! Collie's client crate (spec #606, ADR 0095): the Connection a Rust
//! program holds to talk to a Collie engine, with no engine inside. Its
//! `transport` module opens every socket and starts every Attached engine
//! on the client side, over the network it is handed: the OS's, or the
//! in-memory one an engine in the same process serves (ADR 0060). The
//! Connection takes its runtime as a value, and [`Call`] and [`Query`] are
//! the builders every call goes through (ADR 0081). The SDK crates build on
//! it; it depends on neither of them nor on any Foxhound crate.

#![warn(missing_docs)]

mod call;
mod connection;
mod error;
mod rpc;
pub mod transport;

pub use call::{Call, Query, Reply};
pub use connection::{Connection, ConnectionRuntime};
pub use error::Error;
