//! The endpoint table and its runner (ADR 0067): the committed table of
//! refusal rules, a row per rule and a column per listener, and
//! `EndpointRunner`, which applies every row of a listener's column to that
//! listener, asserts each refusal and checks it in the listener's Audit log
//! through the product's verifier.

mod connection;
mod http;
mod http_proxy;
mod json_rpc;
mod runner;
mod sign_in;
mod table;
mod websocket;

pub use connection::ListenerAddress;
pub use runner::{
    AuditUnreadable, AuditVerifier, Endpoint, EndpointRunError, EndpointRunner, Failure, Hello,
    Listener, Observed, RowFailure,
};
pub(crate) use table::EndpointTable;
pub use table::{Expected, ListenerKind, Rule};
