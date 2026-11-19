//! Foxhound for a program that does not use async (ADR 0081), under the
//! same names as Collie's `blocking`: a call ends in `send()`. The
//! connection owns the runtime its linked engine runs on; each call is
//! handed to it, and the calling thread waits for its end on a channel.
//! Nothing here enters a runtime, so it works inside one too, where it
//! blocks only the thread that calls it.

use goodfetch_collie::linked::ProductConnection;
use goodfetch_collie::{ConnectOptions, EngineInfo};
use goodfetch_collie_client::{Error, blocking};

pub use goodfetch_collie_client::blocking::{
    Call, Connection, EventIter, Exchange, Locator, Page, Query, RawCall, Response, Run, Session,
    Start,
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
    linked: ProductConnection,
    connection: blocking::ClientConnection,
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
        let linked = ProductConnection::connect_blocking(ENGINE, options)?;
        Ok(Self {
            connection: blocking::ClientConnection::from(linked.connection().clone()),
            linked,
        })
    }

    /// The engine the connection reached, as [`crate::Foxhound::engine`]
    /// reports it.
    pub fn engine(&self) -> &EngineInfo {
        self.linked.engine()
    }

    /// The raw call of `operation`, as [`crate::Foxhound::call`]'s, which
    /// `send()` sends.
    pub fn call(&self, operation: &str) -> RawCall {
        self.connection.raw_call(operation)
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

/// The blocking Connection both Products' blocking connections implement
/// (ADRs 0095 and 0099), so a helper taking `&impl blocking::Connection`
/// takes a `Foxhound`; its members are the inherent ones above.
impl Connection for Foxhound {
    fn start(&self) -> Start {
        Foxhound::start(self)
    }

    fn call(&self, operation: &str) -> RawCall {
        Foxhound::call(self, operation)
    }

    fn close(&self) -> Result<(), Error> {
        Foxhound::close(self)
    }
}

/// The client crate's connection, the way down to its typed `call` of a
/// runtime method, which the raw `call` (ADR 0099) does not type.
impl AsRef<blocking::ClientConnection> for Foxhound {
    fn as_ref(&self) -> &blocking::ClientConnection {
        &self.connection
    }
}

/// C-SEND-SYNC: a blocking connection moves between threads, is shared
/// between them and outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Foxhound>();
};
