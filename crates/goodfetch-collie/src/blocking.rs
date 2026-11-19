//! Collie for a program that does not use async (ADR 0081), under the same
//! names: a call ends in `send()`. The connection owns the runtime its
//! linked engine runs on; each call is handed to it, and the calling thread
//! waits for its end on a channel. Nothing here enters a runtime, so it
//! works inside one too, where it blocks only the thread that calls it.

use goodfetch_collie_client::blocking;

pub use goodfetch_collie_client::blocking::{
    Call, Connection, Locator, Page, Query, Run, Session, Start,
};

use crate::Error;
use crate::linked::{Composed, ConnectOptions, LinkedConnection};

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
    linked: LinkedConnection,
    connection: blocking::Connection,
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
        let linked = LinkedConnection::connect_blocking(Composed::COLLIE, options)?;
        Ok(Self {
            connection: blocking::Connection::from(linked.connection().clone()),
            linked,
        })
    }

    /// Leaving the scope or `close()` stops the Run with a Checkpoint;
    /// `finish()` ends it. The start of a Run the program drives
    /// (`run.start`), which `send()` sends.
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
        self.linked.close_blocking()
    }
}

/// The blocking Connection, the type a helper written for either Product
/// takes (ADR 0095): `helper(collie.as_ref())`, until
/// ADR 0099's `Connection` trait lands (#1257).
impl AsRef<blocking::Connection> for Collie {
    fn as_ref(&self) -> &blocking::Connection {
        &self.connection
    }
}

/// C-SEND-SYNC: a blocking connection moves between threads, is shared
/// between them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Collie>();
};
