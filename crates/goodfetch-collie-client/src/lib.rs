//! Collie's client crate (spec #606, ADR 0095): the Connection a Rust
//! program holds to talk to a Collie engine, with no engine inside. Its
//! `transport` module opens every socket and starts every Attached engine
//! on the client side, over the network it is handed: the OS's, or the
//! in-memory one an engine in the same process serves (ADR 0060). The
//! Connection takes its runtime as a value, and [`Call`] and [`Query`] are
//! the builders every call goes through (ADR 0081). A [`Run`] is what a
//! Connection starts, and [`blocking`] repeats the API for a program that
//! does not use async, and [`file_saving`] writes a file to the program's
//! path whole or not at all (ADR 0116). The SDK crates build on it; it depends on neither of
//! them nor on any Foxhound crate.

#![warn(missing_docs)]

pub mod blocking;
mod call;
mod connection;
mod error;
pub mod file_saving;
mod rpc;
mod run;
mod sync;
pub mod transport;

pub use call::{Call, Query, Reply};
pub use connection::{Connection, ConnectionRuntime, SaveFile};
pub use error::{Error, FileFailure};
pub use run::{Run, Start};
