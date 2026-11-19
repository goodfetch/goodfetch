//! The connection, which is the Product (ADRs 0078, 0090 and 0095): what a
//! Rust program opens to drive Collie, and the scope that opens and closes
//! it around a body. Each connection starts its own linked engine, the
//! engine this crate composes, in the program's own process (ADR 0087).

use goodfetch_collie_client::{ClientConnection, Connection, Error, Start};

use crate::linked::{Composed, ConnectOptions, LinkedConnection};

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
    linked: LinkedConnection,
}

impl Collie {
    /// Opens a connection to a linked engine of its own, which runs on a
    /// runtime the connection owns, so the connection works from any
    /// executor. The engine reads `options`' environment, or the process's,
    /// for its Goodfetch home and the machine's settings.
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
        let linked = LinkedConnection::connect(Composed::COLLIE, options).await?;
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

    /// Leaving the scope or `close()` stops the Run with a Checkpoint;
    /// `finish()` ends it. The start of a Run the program drives
    /// (`run.start`), which awaiting sends: a resume is logged at info and
    /// a start that found its Run ended at warning, through `tracing`.
    pub fn start(&self) -> Start {
        self.linked.connection().start()
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and stops;
    /// returns once it has stopped, or once `timeout.close` has run out.
    /// Any later call fails with [`Error::Closed`]. Closing again does
    /// nothing.
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

    /// A connection to a linked engine that runs on `handle`'s runtime,
    /// which the program drives, with `dependencies`.
    #[cfg(test)]
    pub(crate) async fn open(
        handle: tokio::runtime::Handle,
        dependencies: goodfetch_engine::Dependencies,
    ) -> Result<Self, Error> {
        let linked = LinkedConnection::open(
            goodfetch_collie_client::ConnectionRuntime::driven(handle.clone()),
            handle,
            Composed::COLLIE,
            LinkedConnection::hello(Composed::COLLIE.product),
            dependencies,
            goodfetch_collie_client::Limits::new(),
            goodfetch_collie_client::ConnectTimeout::Default,
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

    async fn close(&self) -> Result<(), Error> {
        Collie::close(self).await
    }
}

/// The client crate's connection, the way down to its `call` (ADR 0099)
/// until the public `call` joins the Connection with its own ticket.
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
