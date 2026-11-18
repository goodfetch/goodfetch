//! The API for a program that does not use async (ADR 0081), under the same
//! names: a call ends in `send()`. Each call is handed to the Connection's
//! runtime, which the Connection owns, and the calling thread waits for its
//! end on a channel. It never enters a runtime, so it works inside one too,
//! where it blocks only the thread that calls it.

use std::future::IntoFuture;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::run::{RunId, RunReport, StartedAs};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Error;

/// A Connection to one engine, whose calls block. Cloning it is cheap and
/// every clone is the same Connection.
#[derive(Clone, Debug)]
pub struct Connection(crate::Connection);

impl From<crate::Connection> for Connection {
    /// The blocking face of `connection`, whose runtime it owns or some
    /// other thread drives.
    fn from(connection: crate::Connection) -> Self {
        Self(connection)
    }
}

impl Connection {
    /// The call of `method` with `params`, whose reply reads as `T`.
    pub fn call<T>(&self, method: &str, params: &impl Serialize) -> Call<T> {
        Call {
            call: self.0.call(method, params),
            connection: self.0.clone(),
        }
    }

    /// The start of a Run this Connection drives (`run.start`), which
    /// `send()` sends.
    pub fn start(&self) -> Start {
        Start(self.0.start(), self.0.clone())
    }

    /// The save of a file to the program's `path`, as
    /// [`crate::Connection::save_file`]'s, which `send()` sends.
    pub fn save_file<T>(
        &self,
        method: &str,
        params: &impl Serialize,
        path: &Path,
        call: &str,
    ) -> SaveFile<T> {
        SaveFile {
            save: self.0.save_file(method, params, path, call),
            connection: self.0.clone(),
        }
    }

    /// Closes the Connection as [`crate::Connection::close`] does, and
    /// returns once it is closed.
    ///
    /// # Errors
    ///
    /// As [`crate::Connection::close`].
    pub fn close(&self) -> Result<(), Error> {
        let connection = self.0.clone();
        self.0.wait(async move { connection.close().await })
    }
}

/// A call that waits for its reply, `T`. Nothing is sent until
/// [`Call::send`].
#[must_use = "a call is sent only by send()"]
pub struct Call<T> {
    call: crate::Call<T>,
    connection: crate::Connection,
}

impl<T: DeserializeOwned + Send + 'static> Call<T> {
    /// Fails the call with [`Error::TimedOut`] if its reply has not come
    /// within `timeout`.
    pub fn timeout(self, timeout: Duration) -> Self {
        Self {
            call: self.call.timeout(timeout),
            ..self
        }
    }

    /// Waits for the reply as long as the Connection lasts.
    pub fn no_timeout(self) -> Self {
        Self {
            call: self.call.no_timeout(),
            ..self
        }
    }

    /// Sends the call and blocks the calling thread until its reply.
    ///
    /// # Errors
    ///
    /// What the call fails with.
    pub fn send(self) -> Result<T, Error> {
        self.connection.wait(self.call.into_future())
    }
}

/// The save of a file to the program's path, whose reply reads as `T`.
/// Nothing is sent until [`SaveFile::send`].
#[must_use = "a save is sent only by send()"]
pub struct SaveFile<T> {
    save: crate::SaveFile<T>,
    connection: crate::Connection,
}

impl<T: DeserializeOwned + Send + 'static> SaveFile<T> {
    /// Fails the save with [`Error::TimedOut`] if its reply has not come
    /// within `timeout`, the file it was writing removed.
    pub fn timeout(self, timeout: Duration) -> Self {
        Self {
            save: self.save.timeout(timeout),
            ..self
        }
    }

    /// Waits for the reply as long as the Connection lasts.
    pub fn no_timeout(self) -> Self {
        Self {
            save: self.save.no_timeout(),
            ..self
        }
    }

    /// Sends the save and blocks the calling thread until the file is
    /// saved.
    ///
    /// # Errors
    ///
    /// As awaiting [`crate::SaveFile`].
    pub fn send(self) -> Result<T, Error> {
        self.connection.wait(self.save.into_future())
    }
}

/// The start of a Run (`run.start`). Nothing is sent until
/// [`Start::send`].
#[must_use = "a start is sent only by send()"]
#[derive(Debug)]
pub struct Start(crate::Start, crate::Connection);

impl Start {
    /// Names the Run `run_id`, as [`crate::Start::run_id`] does.
    pub fn run_id(self, run_id: &RunId) -> Self {
        Self(self.0.run_id(run_id), self.1)
    }

    /// Starts the Run and blocks the calling thread until it is started.
    ///
    /// # Errors
    ///
    /// What `run.start` fails with.
    pub fn send(self) -> Result<Run, Error> {
        let Self(start, connection) = self;
        let run = connection.wait(start.into_future())?;
        Ok(Run(run, connection))
    }
}

/// A Run the program drives, whose calls block. Cloning it is cheap and
/// every clone is the same Run.
#[derive(Clone, Debug)]
pub struct Run(crate::Run, crate::Connection);

impl Run {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        self.0.id()
    }

    /// Whether the start began the Run, found it, or went on with it.
    pub fn started_as(&self) -> StartedAs {
        self.0.started_as()
    }

    /// Lets the Run finish, as [`crate::Run::finish`] does.
    pub fn finish(&self) -> Call<RunReport> {
        Call {
            call: self.0.finish(),
            connection: self.1.clone(),
        }
    }

    /// Closes the Run for every clone, as [`crate::Run::close`] does, and
    /// returns once it is closed.
    ///
    /// # Errors
    ///
    /// As [`crate::Run::close`].
    pub fn close(&self) -> Result<(), Error> {
        let run = self.0.clone();
        self.1.wait(async move { run.close().await })
    }
}

/// C-SEND-SYNC: the blocking handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Connection>();
    send_sync_static::<Run>();
    send_static::<Call<RunReport>>();
    send_static::<Start>();
    send_static::<SaveFile<serde_json::Value>>();
};
