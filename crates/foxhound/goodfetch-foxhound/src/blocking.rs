//! Foxhound for a program that does not use async (ADR 0081), under the
//! same names as Collie's `blocking`: a call ends in `send()`. The
//! connection owns the runtime its linked engine runs on; each call is
//! handed to it, and the calling thread waits for its end on a channel.
//! Nothing here enters a runtime, so it works inside one too, where it
//! blocks only the thread that calls it.

use goodfetch_collie::ConnectOptions;
use goodfetch_collie::linked::LinkedConnection;
use goodfetch_collie_client::Error;

pub use goodfetch_collie_client::blocking::{
    Call, Connection, Locator, Page, Query, Run, Session, Start,
};

use crate::foxhound::ENGINE;

/// A connection to Foxhound whose calls block: `foxhound.start().send()`
/// starts a Run. Cloning it is cheap and every clone is the same
/// connection; `close()` on any clone closes it for all.
///
/// ```
/// use goodfetch_foxhound::ConnectOptions;
/// use goodfetch_foxhound::blocking::Foxhound;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let home = tempfile::tempdir()?;
/// # let options = ConnectOptions::default()
/// #     .environment([("GOODFETCH_HOME".into(), home.path().into())].into());
/// let foxhound = Foxhound::connect(options)?;
/// let run = foxhound.start().send()?;
/// let report = run.finish().send()?;
/// println!("{} ended {:?}", report.run_id, report.lifecycle);
/// foxhound.close()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Foxhound {
    linked: LinkedConnection,
    connection: Connection,
}

impl Foxhound {
    /// Opens a connection to a linked Foxhound engine of its own, as
    /// [`crate::Foxhound::connect`] does, and blocks the calling thread
    /// until `hello` is answered.
    ///
    /// # Errors
    ///
    /// As [`crate::Foxhound::connect`].
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let linked = LinkedConnection::connect_blocking(ENGINE, options)?;
        Ok(Self {
            connection: Connection::from(linked.connection().clone()),
            linked,
        })
    }

    /// The start of a Run the program drives (`run.start`), which `send()`
    /// sends.
    pub fn start(&self) -> Start {
        self.connection.start()
    }

    /// Closes the connection for every clone, as
    /// [`crate::Foxhound::close`] does, and returns once its linked engine
    /// has stopped.
    ///
    /// # Errors
    ///
    /// As [`crate::Foxhound::close`].
    pub fn close(&self) -> Result<(), Error> {
        self.linked.close_blocking()
    }
}

/// The blocking Connection, the type a helper written for either Product
/// takes (ADR 0095): `helper(foxhound.as_ref())`, until
/// ADR 0099's `Connection` trait lands (#1257).
impl AsRef<Connection> for Foxhound {
    fn as_ref(&self) -> &Connection {
        &self.connection
    }
}

/// C-SEND-SYNC: a blocking connection moves between threads, is shared
/// between them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Foxhound>();
};
