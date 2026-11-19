//! The Connection both Products' connections implement (ADRs 0095 and
//! 0099): a helper a library serving both Products writes takes
//! `&impl Connection`, so it accepts a `Collie` and a `Foxhound`. Each
//! keeps inherent methods of the same names, so a program that drives one
//! Product calls them with no trait import. [`crate::blocking::Connection`]
//! is its blocking twin; TypeScript's `connection.ts` and Python's
//! `connection_type.py` are its twins in the other SDKs.

use std::future::Future;

use crate::error::Error;
use crate::run::Start;

/// A Client's open link to one engine, through which a program reaches its
/// Runs: the members both Products' connections share.
///
/// ```
/// use goodfetch_collie_client::{Connection, Error};
/// use goodfetch_protocol::run::RunReport;
///
/// /// A library's helper, written once for either Product.
/// async fn start_and_finish(connection: &impl Connection) -> Result<RunReport, Error> {
///     connection.start().await?.finish().await
/// }
/// ```
pub trait Connection {
    /// Leaving the scope or `close()` stops the Run with a Checkpoint;
    /// `finish()` ends it. The start of a Run the program drives
    /// (`run.start`), which awaiting sends: a Run ID naming an interrupted
    /// Run resumes it, and one naming a finished Run returns it as it ended
    /// (ADR 0102).
    fn start(&self) -> Start;

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then its engine; returns once it has stopped. Any
    /// later call fails with [`Error::Closed`]. Closing again does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, and [`Error::Engine`] when the
    /// engine failed.
    ///
    /// # Cancel safety
    ///
    /// Each implementation says what dropping the future before it
    /// completes leaves: Collie's and Foxhound's leave the engine stopping
    /// on its own.
    fn close(&self) -> impl Future<Output = Result<(), Error>> + Send;
}
