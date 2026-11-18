//! Collie for a program that does not use async (ADR 0081), under the same
//! names: a call ends in `send()`. The connection owns the runtime its
//! linked engine runs on; each call is handed to it, and the calling thread
//! waits for its end on a channel. Nothing here enters a runtime, so it
//! works inside one too, where it blocks only the thread that calls it.

use std::future::Future;
use std::sync::mpsc;

use goodfetch_collie_client::{ConnectionRuntime, blocking};
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;

pub use goodfetch_collie_client::blocking::{Call, Run, Start};

use crate::{ConnectOptions, Error, linked};

/// A connection to Collie whose calls block: `collie.start().send()`
/// starts a Run. Cloning it is cheap and every clone is the same
/// connection; `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_collie::ConnectOptions;
/// use goodfetch_collie::blocking::Collie;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// let collie = Collie::connect(options)?;
/// let run = collie.start().send()?;
/// let report = run.finish().send()?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// collie.close()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Collie {
    linked: crate::Collie,
    connection: blocking::Connection,
    handle: Handle,
}

impl Collie {
    /// Opens a connection to a linked engine of its own, as
    /// [`crate::Collie::connect`] does, and blocks the calling thread until
    /// it is open.
    ///
    /// # Errors
    ///
    /// As [`crate::Collie::connect`].
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let (runtime, dependencies) = linked::production(options.into_environment())?;
        let handle = runtime.handle().clone();
        let on = handle.clone();
        let linked = wait(&handle, async move {
            crate::Collie::linked(ConnectionRuntime::owned(runtime), &on, dependencies).await
        })?;
        Ok(Self {
            connection: blocking::Connection::from(linked.connection.clone()),
            linked,
            handle,
        })
    }

    /// The start of a Run the program drives (`run.start`), which `send()`
    /// sends.
    pub fn start(&self) -> Start {
        self.connection.start()
    }

    /// Closes the connection for every clone, as [`crate::Collie::close`]
    /// does, and returns once its linked engine has stopped.
    ///
    /// # Errors
    ///
    /// As [`crate::Collie::close`].
    pub fn close(&self) -> Result<(), Error> {
        let linked = self.linked.clone();
        wait(&self.handle, async move { linked.close().await })
    }
}

/// Runs `future` on `handle`'s runtime and blocks the calling thread until
/// it ends.
fn wait<T: Send + 'static>(
    handle: &Handle,
    future: impl Future<Output = Result<T, Error>> + Send + 'static,
) -> Result<T, Error> {
    let tasks = TaskTracker::new();
    let (done, ended) = mpsc::sync_channel(1);
    tasks.spawn_on(
        async move {
            let _gone = done.send(future.await);
        },
        handle,
    );
    tasks.close();
    // The task ends without sending only when the runtime shut down.
    ended.recv().unwrap_or(Err(Error::Closed))
}

/// C-SEND-SYNC: a blocking connection moves between threads, is shared
/// between them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Collie>();
};
