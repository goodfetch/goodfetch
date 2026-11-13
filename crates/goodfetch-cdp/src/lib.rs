//! Chrome's debugging protocol, CDP, as the engine speaks it to the
//! Chromium builds it pins (spec #577): the protocol's bindings, generated from the
//! pinned build's own protocol file, and the client's connection, which
//! carries NUL-delimited JSON messages over the pipe the browser was started
//! with (`--remote-debugging-pipe`, never a debugging port; ADR 0017). Its
//! own crate because the bindings are generated, heavy and rarely change
//! (STYLE.md, reason 3). No generated type reaches the SDK API: a program's
//! CDP session carries JSON values (ADRs 0085 and 0099).
//!
//! The connection is the translation point of every CDP error and every
//! disconnection (ADR 0127): each becomes a [`Failure`] variant here, and
//! only here is a CDP error's text read.

mod connection;
mod failure;
mod framing;
#[expect(
    clippy::module_name_repetitions,
    clippy::struct_excessive_bools,
    reason = "generated from the protocol file, whose names and shapes are Chromium's"
)]
pub mod generated;
mod net_error;

/// The fuzz target of the pipe's framing and message decoding (ADR 0067),
/// whose seeds and past crashers `fuzz_replay!` replays on every OS.
pub use connection::fuzz_pipe;
pub use connection::{
    Connection, Driver, Ended, Event as RawEvent, SessionId, Subscription, SubscriptionEnd,
};
pub use failure::{Failure, PartyText, Refusal};
pub use framing::{Framer, FramingError, MAX_FRAME};
pub use net_error::{BROWSER_REDIRECT_CHAIN, CertificateError, NetError};

/// A command of the protocol: its params, tied to its method's name and to
/// what it returns.
pub trait Command: serde::Serialize {
    /// `Domain.method`.
    const METHOD: &'static str;
    /// What a successful command returns.
    type Returns: serde::de::DeserializeOwned;
}

/// An event of the protocol: its params, tied to its method's name.
pub trait Event: serde::de::DeserializeOwned {
    /// `Domain.event`.
    const METHOD: &'static str;
}
