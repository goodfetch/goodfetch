//! The connection, which is the Product (ADRs 0078, 0090 and 0095): what a
//! Rust program opens to drive Collie, and the scope that opens and closes
//! it around a body. Each connection starts its own linked engine, the
//! engine this crate composes, in the program's own process (ADR 0087), or
//! reaches a Daemon ([`crate::EngineChoice`]).

use goodfetch_collie_client::{ClientConnection, Connection, Error, Query, RawCall, Start};
use goodfetch_protocol::settings::{ConfigExplainParams, SettingsExplanation};

use crate::EngineInfo;
use crate::linked::{Composed, ConnectOptions, ProductConnection};

/// A connection to Collie, which is the Product: `collie.start()` starts a
/// Run. Cloning it is cheap and every clone is the same connection;
/// `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_collie::{Collie, ConnectOptions};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
/// let collie = Collie::connect(options).await?;
/// let run = collie.start().await?;
/// let report = run.finish().await?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// collie.close().await?;
/// # Ok::<(), goodfetch_collie::Error>(())
/// # })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Collie {
    linked: ProductConnection,
}

impl Collie {
    /// Opens a connection to the engine `options` choose
    /// ([`ConnectOptions::engine`]): at `Auto`, the remote Daemon
    /// `GOODFETCH_URL` names, else the Daemon running in the Goodfetch
    /// home, else a linked engine of its own. The connection runs on a
    /// runtime it owns, so it works from any executor, and reads
    /// `options`' environment, or the process's, for its Goodfetch home, a
    /// remote Daemon and the machine's settings; it logs the engine it
    /// reached at info.
    /// It sets `options`' Limits on the connection before it returns
    /// ([`ConnectOptions::limits`]), and waits for `hello`'s answer within
    /// `options`' `timeout`, `timeout.connect`'s 30 s when left out
    /// ([`ConnectOptions::timeout`]).
    ///
    /// # Errors
    ///
    /// `engine_exited` when the linked engine cannot start, with no
    /// Goodfetch home among others, `setting_invalid` for a zero `timeout`,
    /// a zero `max_listener_queue` or a remote Daemon's URL that names no loopback `ws://` address, or
    /// `GOODFETCH_URL` without `GOODFETCH_TOKEN` or a deployment variable
    /// set but empty or not Unicode, `setting_unknown` for a `GOODFETCH_`
    /// variable off the closed list, naming the closest, `setting_invalid`
    /// at `project_configuration` or `format_version_unsupported` for a
    /// project file that cannot be read ([`ConnectOptions::project`]),
    /// `daemon_unreachable` when
    /// the Daemon chosen cannot be reached, which `Daemon` never starts
    /// (ADR 0098), `deadline_exceeded` naming `timeout.connect` when `hello` is not
    /// answered within it, what `hello` fails with, and what `usage.set`
    /// fails with for a Limit the engine refuses, the connection closed
    /// then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes stops the linked engine it
    /// started.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let linked = ProductConnection::connect(Composed::COLLIE, options).await?;
        Ok(Self { linked })
    }

    /// Opens a connection with `options`, lends it to `body`, and closes it
    /// whatever `body` returns. Its future is `Send` whenever `body`'s is
    /// (ADR 0112).
    ///
    /// ```
    /// use goodfetch_collie::{Collie, ConnectOptions};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let home = tempfile::tempdir()?;
    /// # let options = ConnectOptions::default()
    /// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
    /// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
    /// let report = Collie::scope(options, async |collie| {
    ///     collie.start().await?.finish().await
    /// })
    /// .await?;
    /// # Ok::<(), goodfetch_collie::Error>(())
    /// # })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A body that holds a value which is not `Send` across an `.await`
    /// makes the scope's future not `Send`, so a multi-threaded runtime
    /// cannot spawn it:
    ///
    /// ```compile_fail,E0277
    /// use std::rc::Rc;
    ///
    /// use goodfetch_collie::{Collie, ConnectOptions};
    ///
    /// fn spawnable(_: impl Future + Send + 'static) {}
    ///
    /// spawnable(Collie::scope(ConnectOptions::default(), async |collie| {
    ///     let held = Rc::new(());
    ///     let run = collie.start().await?;
    ///     drop(held);
    ///     run.finish().await
    /// }));
    /// ```
    ///
    /// The same body without it is spawnable:
    ///
    /// ```
    /// use goodfetch_collie::{Collie, ConnectOptions};
    ///
    /// fn spawnable(_: impl Future + Send + 'static) {}
    ///
    /// spawnable(Collie::scope(ConnectOptions::default(), async |collie| {
    ///     let run = collie.start().await?;
    ///     run.finish().await
    /// }));
    /// ```
    ///
    /// # Errors
    ///
    /// What [`Collie::connect`] fails with, then `body`'s error, then what
    /// [`Collie::close`] fails with.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes drops the connection without
    /// waiting, as dropping a [`Collie`] does.
    pub async fn scope<R>(
        options: ConnectOptions,
        body: impl AsyncFnOnce(&Collie) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let collie = Self::connect(options).await?;
        let result = body(&collie).await;
        let closed = collie.close().await;
        let value = result?;
        closed.map(|()| value)
    }

    /// The engine the connection reached, fixed when it opened: the linked
    /// engine or a Daemon ([`crate::EngineChoice`]). It never holds a
    /// Daemon's token.
    pub fn engine(&self) -> &EngineInfo {
        self.linked.engine()
    }

    /// The raw call of `operation` (ADR 0099, G8), the way down to an
    /// operation the SDK does not generate, such as one a newer engine
    /// added: its params and its result are the wire's JSON, in
    /// snake case, and awaiting sends it.
    ///
    /// ```
    /// use goodfetch_collie::{ConnectOptions, Collie};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let home = tempfile::tempdir()?;
    /// # let options = ConnectOptions::default()
    /// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
    /// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
    /// let connection = Collie::connect(options).await?;
    /// let listed = connection
    ///     .call("run.list")
    ///     .params(serde_json::json!({ "lifecycle": "running" }))
    ///     .await?;
    /// assert_eq!(listed["runs"], serde_json::json!([]));
    /// connection.close().await?;
    /// # Ok::<(), goodfetch_collie::Error>(())
    /// # })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Once the connection is closed it fails with `closed`; the engine's
    /// error, such as `invalid_params`, and `deadline_exceeded` past
    /// `.timeout(d)`, which is sent as the operation's own, fail it too.
    /// It is never retried.
    pub fn call(&self, operation: &str) -> RawCall {
        self.linked.connection().raw_call(operation)
    }

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
    /// sends: a resume is logged at info and a start that found its Run
    /// ended at warning, through `tracing`.
    pub fn start(&self) -> Start {
        self.linked.connection().start()
    }

    /// Shows each setting's effective value, the layer that set it and the
    /// layer that locked it, from the built-in defaults, the Operator
    /// policy, user configuration, the project configuration given and the
    /// run layer: the Connection's own settings with the run values given
    /// over them, or a live Run's as it opened when `run_id` names one; and
    /// lists the keys only the other Product honours as that Product's.
    ///
    /// What the connection runs with (ADR 0083), as `config explain`
    /// shows it (`config.explain`), which awaiting asks: each setting, or
    /// those `.keys(…)` names, with the origin that set it, the
    /// connection's own settings ([`ConnectOptions::settings`]), which each
    /// Run it starts inherits. A Run's own
    /// explanation is
    /// [`Run::explain_settings`](crate::Run::explain_settings). Once the connection is
    /// closed it fails with `closed`.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        self.linked.connection().explain_settings()
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and stops;
    /// returns once it has stopped, or once `timeout.close` has run out.
    /// Any later call fails with `closed`. Closing again does
    /// nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, `engine_exited` when the linked
    /// engine failed, and `deadline_exceeded` when `timeout.close` ran out
    /// first.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes leaves the engine stopping
    /// on its own.
    pub async fn close(&self) -> Result<(), Error> {
        self.linked.close().await
    }

    /// A connection to a linked engine that runs on `handle`'s runtime,
    /// which the program drives, with `dependencies`.
    #[cfg(test)]
    pub(crate) async fn open(
        handle: tokio::runtime::Handle,
        dependencies: goodfetch_engine::Dependencies,
    ) -> Result<Self, Error> {
        let linked = ProductConnection::open(
            goodfetch_collie_client::ConnectionRuntime::driven(handle.clone()),
            handle,
            Composed::COLLIE,
            ProductConnection::hello(Composed::COLLIE.product),
            dependencies,
            crate::linked::Asked::default(),
            None,
        )
        .await?;
        Ok(Self { linked })
    }
}

/// The Connection both Products' connections implement (ADRs 0095 and
/// 0099), so a helper taking `&impl Connection` takes a `Collie`; its
/// members are the inherent ones above.
impl Connection for Collie {
    fn start(&self) -> Start {
        Collie::start(self)
    }

    fn call(&self, operation: &str) -> RawCall {
        Collie::call(self, operation)
    }

    fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        Collie::explain_settings(self)
    }

    async fn close(&self) -> Result<(), Error> {
        Collie::close(self).await
    }
}

/// The client crate's connection, the way down to its typed `call` of a
/// runtime method, which the raw `call` (ADR 0099) does not type.
impl AsRef<ClientConnection> for Collie {
    fn as_ref(&self) -> &ClientConnection {
        self.linked.connection()
    }
}

/// C-SEND-SYNC: a connection's handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Collie>();
};

#[cfg(test)]
mod tests;
