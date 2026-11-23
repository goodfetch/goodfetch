//! The Connection both Products' connections implement (ADRs 0095 and
//! 0099): a helper a library serving both Products writes takes
//! `&impl Connection`, so it accepts a `Collie` and a `Foxhound`. Each
//! keeps inherent methods of the same names, so a program that drives one
//! Product calls them with no trait import. [`crate::blocking::Connection`]
//! is its blocking twin; TypeScript's `connection.ts` and Python's
//! `connection_type.py` are its twins in the other SDKs.

use std::future::Future;

use goodfetch_protocol::run::{CatalogTarget, RunId};
use goodfetch_protocol::settings::{ConfigExplainParams, SettingsExplanation};

use crate::call::Query;
use crate::error::Error;
use crate::handlerless::HandlerlessStart;
use crate::raw_call::RawCall;
use crate::run::Start;
use crate::run_view::RunView;

/// A Client's open link to one engine, through which a program reaches its
/// Runs: the members both Products' connections share.
///
/// ```
/// use goodfetch_collie_client::{Answer, Connection, Error};
/// use goodfetch_protocol::run::RunReport;
///
/// /// A library's helper, written once for either Product.
/// async fn start_and_finish(connection: &impl Connection) -> Result<Answer<RunReport>, Error> {
///     connection.start().await?.finish().await
/// }
/// ```
pub trait Connection {
    /// Leaving the scope or `close()` stops the Run with a Checkpoint;
    /// `finish()` ends it.
    ///
    /// Starts a Run its program drives, and answers once it is started. A
    /// Run ID names the Run for good, and a start under one acts on what
    /// the Run index holds (ADR 0102): no such Run begins it, `new`; an
    /// interrupted Run, stopped, failed, or running with no live writer,
    /// resumes from its committed state at the next run epoch, `resumed`,
    /// as `run.resume` does; a finished Run is answered as it ended with no
    /// handler called, `ended`. A Run live with a writer, in this engine or
    /// another, fails with `run_in_use`; a Run started with other inputs or
    /// by another caller with `run_conflict` listing each difference; a Run
    /// the Run index lists whose state is missing or unreadable with
    /// `run_state_unavailable`, never starting afresh.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a Run the program drives (`run.start`), which awaiting
    /// sends.
    fn start(&self) -> Start;

    /// Starts a Recipe's run, a Run with no handler in the program, and
    /// answers once it is started. Its Run ID works as `run.start`'s. With
    /// `detached`, the Run outlives the program that started it: closing
    /// its value frees only the value, `run.finish` waits for its end and
    /// `run.stop` stops it.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a Recipe's run (`recipe.start`), which awaiting sends.
    fn start_recipe(&self, recipe: &str) -> HandlerlessStart;

    /// Starts a code-free catalog run, a Run with no handler in the program
    /// whose verbs come from the catalog (ADR 0036): an Extraction of a
    /// page by a schema, `extract`, an Agent task, `agent`, or a Plan
    /// replayed with no model call, `plan_replay`; and answers once it is
    /// started. Its Run ID works as `run.start`'s. With `detached`, the Run
    /// outlives the program that started it: closing its value frees only
    /// the value, `run.finish` waits for its end and `run.stop` stops it.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// The start of a code-free catalog run (`catalog_run.start`), which
    /// awaiting sends.
    fn start_catalog_run(&self, target: CatalogTarget) -> HandlerlessStart;

    /// Opens a view of the Run `run_id` names, one another program started
    /// such as a detached Run, with no round trip (ADR 0102). The view takes
    /// no writer role, and closing it frees only the view.
    ///
    /// # Errors
    ///
    /// `closed` once the connection is closed.
    fn open_run(&self, run_id: &RunId) -> Result<RunView, Error>;

    /// The raw call of `operation` (ADR 0099, G8), the way down to an
    /// operation the SDK does not generate: its params and its result are
    /// the wire's JSON, in snake case. Awaiting sends it; it is never
    /// retried, and once the connection is closed it fails with `closed`.
    fn call(&self, operation: &str) -> RawCall;

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the connection runs with (ADR 0083), as `config explain`
    /// shows it (`config.explain`), which awaiting asks: each setting, or
    /// those [`Query::keys`] names, with the origin that set it. Once the
    /// connection is closed it fails with `closed`.
    fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams>;

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then its engine; returns once it has stopped. Any
    /// later call fails with `closed`. Closing again does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, and `engine_exited` when the
    /// engine failed.
    ///
    /// # Cancel safety
    ///
    /// Each implementation says what dropping the future before it
    /// completes leaves: Collie's and Foxhound's leave the engine stopping
    /// on its own.
    fn close(&self) -> impl Future<Output = Result<(), Error>> + Send;
}
