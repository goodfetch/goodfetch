//! The connection, which is the Product (ADRs 0078, 0090 and 0095): what a
//! Rust program opens to drive Foxhound, and the scope that opens and
//! closes it around a body. Each connection starts its own linked engine,
//! the Foxhound engine this crate composes, in the program's own process
//! (ADRs 0024 and 0087), through Collie's crate's linked connection, or
//! reaches a Daemon ([`crate::EngineChoice`]).

use goodfetch_collie::linked::{Composed, ProductConnection};
use goodfetch_collie::{ConnectOptions, EngineInfo};
use goodfetch_collie_client::{
    ClientConnection, Connection, Error, Query, RawCall, Start, UsageKind,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{ConfigExplainParams, SettingsExplanation};

/// Foxhound's linked engine: Foxhound, of [`crate::composition`].
pub(crate) const ENGINE: Composed = Composed {
    product: Product::Foxhound,
    composition: crate::composition,
};

/// Foxhound's paid fetches, the Usage kind a Limit caps besides Collie's
/// (ADR 0087): a Collie engine refuses it with `not_in_product`.
pub const PAID_FETCHES: UsageKind =
    UsageKind::named(goodfetch_protocol::foxhound::usage::PAID_FETCH);

/// A connection to Foxhound, which is the Product: it carries every member
/// of a Collie connection under the same names, so `foxhound.start()`
/// starts a Run. Cloning it is cheap and every clone is the same
/// connection; `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_foxhound::{ConnectOptions, Foxhound};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
/// let foxhound = Foxhound::connect(options).await?;
/// let run = foxhound.start().await?;
/// let report = run.finish().await?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// foxhound.close().await?;
/// # Ok::<(), goodfetch_foxhound::Error>(())
/// # })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Foxhound {
    linked: ProductConnection,
}

impl Foxhound {
    /// Opens a connection to the engine `options` choose
    /// ([`ConnectOptions::engine`]): at `Auto`, the remote Daemon
    /// `GOODFETCH_URL` names, else the Daemon running in the Goodfetch
    /// home, else a linked Foxhound engine of its own. The connection runs
    /// on a runtime it owns, so it works from any executor, and returns
    /// once `hello` is answered. It reads `options`' environment, or the
    /// process's, for its Goodfetch home, a remote Daemon and the machine's
    /// settings, and never an engine-path variable; it logs the engine it
    /// reached at info.
    /// It sets `options`' Limits on the connection before it returns
    /// ([`ConnectOptions::limits`]), and waits for `hello`'s answer within
    /// `options`' `timeout`, `timeout.connect`'s 30 s when left out
    /// ([`ConnectOptions::timeout`]).
    ///
    /// # Errors
    ///
    /// `engine_exited` when the linked engine cannot start, with no
    /// Goodfetch home among others, `setting_invalid` for a zero `timeout`
    /// or a remote Daemon's URL that names no loopback `ws://` address, or
    /// `GOODFETCH_URL` without `GOODFETCH_TOKEN`, `daemon_unreachable` when
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
        let linked = ProductConnection::connect(ENGINE, options).await?;
        Ok(Self { linked })
    }

    /// Opens a connection with `options`, lends it to `body`, and closes it
    /// whatever `body` returns. Its future is `Send` whenever `body`'s is
    /// (ADR 0112).
    ///
    /// ```
    /// use goodfetch_foxhound::{ConnectOptions, Foxhound};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let home = tempfile::tempdir()?;
    /// # let options = ConnectOptions::default()
    /// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
    /// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
    /// let report = Foxhound::scope(options, async |foxhound| {
    ///     foxhound.start().await?.finish().await
    /// })
    /// .await?;
    /// # Ok::<(), goodfetch_foxhound::Error>(())
    /// # })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// What [`Foxhound::connect`] fails with, then `body`'s error, then
    /// what [`Foxhound::close`] fails with.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes drops the connection
    /// without waiting, as dropping a [`Foxhound`] does.
    pub async fn scope<R>(
        options: ConnectOptions,
        body: impl AsyncFnOnce(&Foxhound) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let foxhound = Self::connect(options).await?;
        let result = body(&foxhound).await;
        let closed = foxhound.close().await;
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
    /// use goodfetch_foxhound::{ConnectOptions, Foxhound};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let home = tempfile::tempdir()?;
    /// # let options = ConnectOptions::default()
    /// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
    /// # goodfetch_runtime::current_thread().enable_all().build()?.block_on(async {
    /// let connection = Foxhound::connect(options).await?;
    /// let listed = connection
    ///     .call("run.list")
    ///     .params(serde_json::json!({ "lifecycle": "running" }))
    ///     .await?;
    /// assert_eq!(listed["runs"], serde_json::json!([]));
    /// connection.close().await?;
    /// # Ok::<(), goodfetch_foxhound::Error>(())
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

    /// The start of a Run the program drives (`run.start`), which awaiting
    /// sends.
    pub fn start(&self) -> Start {
        self.linked.connection().start()
    }

    /// What the connection runs with (ADR 0083), as `config explain`
    /// shows it (`config.explain`), which awaiting asks: each setting, or
    /// those `.keys(…)` names, with its effective value, the layer and
    /// origin that set it and the layer that locked it, the connection's
    /// own settings ([`ConnectOptions::settings`]) over configuration and
    /// the Operator policy, which each Run it starts inherits. A Run's own
    /// explanation is
    /// [`Run::explain_settings`](crate::Run::explain_settings). Once the connection is
    /// closed it fails with `closed`.
    pub fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        self.linked.connection().explain_settings()
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and
    /// stops; returns once it has stopped, or once `timeout.close` has run
    /// out. Any later call fails with `closed`. Closing again
    /// does nothing.
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
}

/// The Connection both Products' connections implement (ADRs 0095 and
/// 0099), so a helper taking `&impl Connection` takes a `Foxhound`; its
/// members are the inherent ones above.
impl Connection for Foxhound {
    fn start(&self) -> Start {
        Foxhound::start(self)
    }

    fn call(&self, operation: &str) -> RawCall {
        Foxhound::call(self, operation)
    }

    fn explain_settings(&self) -> Query<SettingsExplanation, ConfigExplainParams> {
        Foxhound::explain_settings(self)
    }

    async fn close(&self) -> Result<(), Error> {
        Foxhound::close(self).await
    }
}

/// The client crate's connection, the way down to its typed `call` of a
/// runtime method, which the raw `call` (ADR 0099) does not type.
impl AsRef<ClientConnection> for Foxhound {
    fn as_ref(&self) -> &ClientConnection {
        self.linked.connection()
    }
}

/// C-SEND-SYNC: a connection's handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Foxhound>();
};
