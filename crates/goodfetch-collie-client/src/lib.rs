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

mod answer;
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
mod listener_queue;
pub mod logging;
mod markers;
mod published;
mod raw_call;
mod response;
mod rpc;
mod run;
mod schema;
pub mod settings;
mod sync;
pub mod transport;

pub use answer::Answer;
pub use call::{Call, Query, Reply};
pub use connect_timeout::ConnectTimeout;
pub use connection::{ClientConnection, ConnectionRuntime, SaveFile};
pub use connection_type::Connection;
pub use error::{BoxError, Error};
pub use event_stream::EventStream;
pub use field_values::FieldValues;
pub use generated::events;
pub use generated::handles::{Locator, Page, Run, Session};
pub use goodfetch_protocol::error_code::{ErrorCode, ErrorDetails};
pub use handler::{HandlerCall, HandlerRequests};
pub use limits::{Limits, Money, MoneyError, UsageKind};
pub use listener_queue::ListenerQueue;
pub use raw_call::RawCall;
pub use response::{Exchange, Response};
pub use run::Start;
pub use schema::{Schema, SchemaError};
/// The `schemars` whose `JsonSchema` a [`Schema`] derives from, re-exported
/// so that a program's derive is the SDK's version (ADR 0082).
pub use schemars;
