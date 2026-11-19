//! Collie's client crate (spec #606, ADR 0095): the [`ClientConnection`] a
//! Rust program holds to talk to a Collie engine, with no engine inside,
//! and the [`Connection`] both Products' connections implement. Its
//! `transport` module opens every socket and starts every Attached engine
//! on the client side, over the network it is handed: the OS's, or the
//! in-memory one an engine in the same process serves (ADR 0060). The
//! Connection takes its runtime as a value, and [`Call`] and [`Query`] are
//! the builders every call goes through (ADR 0081). A [`Run`] is what a
//! Connection starts; it and the handles it opens, [`Session`], [`Page`]
//! and [`Locator`], have the members the catalog gives them, one form per
//! declared kind (ADR 0099), and [`blocking`] repeats the API for a
//! program that does not use async. [`file_saving`] writes a file to the
//! program's path whole or not at all (ADR 0116). The SDK crates build on
//! it; it depends on neither of them nor on any Foxhound crate.

#![warn(missing_docs)]

pub mod blocking;
mod call;
mod connect_timeout;
mod connection;
mod connection_type;
mod error;
mod event_stream;
mod field_values;
pub mod file_saving;
mod generated;
mod handler;
mod host;
mod limits;
mod line;
mod markers;
mod response;
mod rpc;
mod run;
mod sync;
pub mod transport;

pub use call::{Call, Query, Reply};
pub use connect_timeout::ConnectTimeout;
pub use connection::{ClientConnection, ConnectionRuntime, SaveFile};
pub use connection_type::Connection;
pub use error::{Error, FileFailure};
pub use event_stream::EventStream;
pub use field_values::FieldValues;
pub use generated::events;
pub use generated::handles::{Locator, Page, Run, Session};
pub use handler::{HandlerCall, HandlerRequests};
pub use limits::{Limits, Money, MoneyError, UsageKind};
pub use response::{Exchange, Response};
pub use run::Start;
