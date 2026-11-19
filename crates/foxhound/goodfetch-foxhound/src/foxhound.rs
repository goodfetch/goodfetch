//! The connection, which is the Product (ADRs 0078, 0090 and 0095): what a
//! Rust program opens to drive Foxhound, and the scope that opens and
//! closes it around a body. Each connection starts its own linked engine,
//! the Foxhound engine this crate composes, in the program's own process
//! (ADRs 0024 and 0087), through Collie's crate's linked connection.

use goodfetch_collie::ConnectOptions;
use goodfetch_collie::linked::{Composed, LinkedConnection};
use goodfetch_collie_client::{ClientConnection, Connection, Error, Start, UsageKind};
use goodfetch_protocol::catalog::Product;

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
    linked: LinkedConnection,
}

impl Foxhound {
    /// Opens a connection to a linked Foxhound engine of its own, which
    /// runs on a runtime the connection owns, so the connection works from
    /// any executor; it returns once `hello` is answered. The engine reads
    /// `options`' environment, or the process's, for its Goodfetch home and
    /// the machine's settings, and never an engine-path variable.
    /// It sets `options`' Limits on the connection before it returns
    /// ([`ConnectOptions::limits`]), and waits for `hello`'s answer within
    /// `options`' `timeout`, `timeout.connect`'s 30 s when left out
    /// ([`ConnectOptions::timeout`]).
    ///
    /// # Errors
    ///
    /// [`Error::Engine`] when the linked engine cannot start, with no
    /// Goodfetch home among others, `setting_invalid` for a zero `timeout`,
    /// `deadline_exceeded` naming `timeout.connect` when `hello` is not
    /// answered within it, what `hello` fails with, and what `usage.set`
    /// fails with for a Limit the engine refuses, the connection closed
    /// then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes stops the linked engine it
    /// started.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let linked = LinkedConnection::connect(ENGINE, options).await?;
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

    /// The start of a Run the program drives (`run.start`), which awaiting
    /// sends.
    pub fn start(&self) -> Start {
        self.linked.connection().start()
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and
    /// stops; returns once it has stopped, or once `timeout.close` has run
    /// out. Any later call fails with [`Error::Closed`]. Closing again
    /// does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, [`Error::Engine`] when the linked
    /// engine failed, and [`Error::TimedOut`] when `timeout.close` ran out
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

    async fn close(&self) -> Result<(), Error> {
        Foxhound::close(self).await
    }
}

/// The client crate's connection, the way down to its `call` (ADR 0099)
/// until the public `call` joins the Connection with its own ticket.
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
