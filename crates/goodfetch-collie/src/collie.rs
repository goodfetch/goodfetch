//! The connection, which is the Product (ADRs 0078, 0090 and 0095): what a
//! Rust program opens to drive Collie, and the scope that opens and closes
//! it around a body. Each connection starts its own linked engine, the
//! engine this crate composes, in the program's own process (ADR 0087).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use goodfetch_collie_client::{Connection, ConnectionRuntime, Error, Start};
use goodfetch_engine::Dependencies;
use tokio::runtime::Handle;

use crate::linked::{self, LinkedEngine};

/// What [`Collie::connect`] opens a connection with.
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    environment: Option<BTreeMap<OsString, OsString>>,
}

impl ConnectOptions {
    /// The environment variables the linked engine reads, such as
    /// `GOODFETCH_HOME`, in place of the process's own.
    #[must_use]
    pub fn environment(mut self, environment: BTreeMap<OsString, OsString>) -> Self {
        self.environment = Some(environment);
        self
    }

    /// The environment the linked engine reads: the one set, or the
    /// process's.
    #[expect(
        clippy::disallowed_methods,
        reason = "the SDK's connect(), given no environment map, reads the process environment once, the one reader besides main (ADR 0083)"
    )]
    pub(crate) fn into_environment(self) -> BTreeMap<OsString, OsString> {
        self.environment
            .unwrap_or_else(|| std::env::vars_os().collect())
    }
}

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
    pub(crate) connection: Connection,
    engine: Arc<LinkedEngine>,
}

impl Collie {
    /// Opens a connection to a linked engine of its own, which runs on a
    /// runtime the connection owns, so the connection works from any
    /// executor. The engine reads `options`' environment, or the process's,
    /// for its Goodfetch home and the machine's settings.
    ///
    /// # Errors
    ///
    /// [`Error::Engine`] when the linked engine cannot start, with no
    /// Goodfetch home among others, and what `hello` fails with.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes stops the linked engine it
    /// started.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let (runtime, dependencies) = linked::production(options.into_environment())?;
        let handle = runtime.handle().clone();
        Self::linked(ConnectionRuntime::owned(runtime), &handle, dependencies).await
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

    /// The start of a Run the program drives (`run.start`), which awaiting
    /// sends.
    pub fn start(&self) -> Start {
        self.connection.start()
    }

    /// Closes the connection for every clone: first every Run it started
    /// that still runs, then the linked engine, which checkpoints and stops;
    /// returns once it has stopped. Any later call fails with
    /// [`Error::Closed`]. Closing again does nothing.
    ///
    /// # Errors
    ///
    /// What closing a Run fails with, and [`Error::Engine`] when the linked
    /// engine failed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before it completes leaves the engine stopping
    /// on its own.
    pub async fn close(&self) -> Result<(), Error> {
        let closed = self.connection.close().await;
        let stopped = self.engine.stopped().await;
        closed.and(stopped)
    }

    /// A connection to a linked engine that runs on `handle`'s runtime,
    /// which the program drives, with `dependencies`.
    #[cfg(test)]
    pub(crate) async fn open(handle: Handle, dependencies: Dependencies) -> Result<Self, Error> {
        Self::linked(
            ConnectionRuntime::driven(handle.clone()),
            &handle,
            dependencies,
        )
        .await
    }

    /// A connection on `runtime` to a linked engine that runs on `handle`,
    /// that runtime's handle, with `dependencies`. When `hello` fails, the
    /// engine is left to stop at the end of its input, unwaited.
    pub(crate) async fn linked(
        runtime: ConnectionRuntime,
        handle: &Handle,
        dependencies: Dependencies,
    ) -> Result<Self, Error> {
        let (engine, network) = LinkedEngine::start(handle, dependencies)?;
        let connection = linked::open(runtime, &network).await?;
        Ok(Self {
            connection,
            engine: Arc::new(engine),
        })
    }
}

/// C-SEND-SYNC: a connection's handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Collie>();
    send_sync_static::<ConnectOptions>();
};

#[cfg(test)]
mod tests;
