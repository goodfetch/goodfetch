//! The API for a program that does not use async (ADR 0081), under the same
//! names: a call ends in `send()`, and every handle the catalog gives is
//! the twin of the async one, which `xtask codegen` writes (ADR 0099) into
//! `generated/blocking.rs`, a submodule of this one. Each call is handed to
//! the Connection's runtime, which the Connection owns, and the calling
//! thread waits for its end on a channel. It never enters a runtime, so it
//! works inside one too, where it blocks only the thread that calls it.

use std::collections::BTreeMap;
use std::future::IntoFuture;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::page::events::PageRequest;
use goodfetch_protocol::page::read::{PageRead, PageReadParams};
use goodfetch_protocol::run::{RunId, RunReport, StartedAs, Tags};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::Error;
use crate::field_values::FieldValues;
pub use twins::{Locator, Page, Run, Session};

/// The blocking twins of the handles, which `xtask codegen` writes.
#[path = "generated/blocking.rs"]
mod twins;

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
    pub fn call<T: DeserializeOwned>(&self, method: &str, params: &impl Serialize) -> Call<T> {
        Call {
            call: self.0.call(method, params),
            connection: self.0.clone(),
        }
    }

    /// Leaving the scope or `close()` stops the Run with a Checkpoint;
    /// `finish()` ends it. The start of a Run this Connection drives
    /// (`run.start`), which `send()` sends.
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

/// A call that waits for its reply, `T`, with its params, `P`. Nothing is
/// sent until [`Call::send`].
#[must_use = "a call is sent only by send()"]
pub struct Call<T, P = Value> {
    call: crate::Call<T, P>,
    connection: crate::Connection,
}

impl<T, P> Call<T, P> {
    /// The blocking face of `call`, sent on `connection`'s runtime.
    pub(crate) fn new(call: crate::Call<T, P>, connection: &crate::Connection) -> Self {
        Self {
            call,
            connection: connection.clone(),
        }
    }

    /// Changes the params before they are sent: an option's builder
    /// method.
    pub(crate) fn edit(self, edit: impl FnOnce(&mut P)) -> Self {
        Self {
            call: self.call.edit(edit),
            ..self
        }
    }

    /// Fails the call if its reply has not come within `timeout`, as
    /// [`crate::Call::timeout`] does.
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
}

impl<T: Send + 'static, P: Serialize + Send + 'static> Call<T, P> {
    /// Sends the call and blocks the calling thread until its reply.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`] once its handle or the Connection is closed,
    /// [`Error::Failed`] with the engine's error, [`Error::TimedOut`] once
    /// its timeout runs out, and [`Error::Shape`] when its reply does not
    /// fit `T`.
    pub fn send(self) -> Result<T, Error> {
        self.connection.wait(self.call.into_future())
    }
}

/// A question the engine answers at once, `T`, with its params, `P`.
/// Nothing is sent until [`Query::send`].
#[must_use = "a query is sent only by send()"]
pub struct Query<T, P = Value> {
    query: crate::Query<T, P>,
    connection: crate::Connection,
}

impl<T, P> Query<T, P> {
    /// The blocking face of `query`, sent on `connection`'s runtime.
    pub(crate) fn new(query: crate::Query<T, P>, connection: &crate::Connection) -> Self {
        Self {
            query,
            connection: connection.clone(),
        }
    }

    /// Changes the params before they are sent, as the async query's
    /// option builder methods do.
    #[expect(
        dead_code,
        reason = "no query of the catalog takes an option yet; `xtask codegen` writes its builder methods through this one"
    )]
    pub(crate) fn edit(self, edit: impl FnOnce(&mut P)) -> Self {
        Self {
            query: self.query.edit(edit),
            ..self
        }
    }
}

impl<T: Send + 'static, P: Serialize + Send + 'static> Query<T, P> {
    /// Sends the query and blocks the calling thread until its answer.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`] once its handle or the Connection is closed,
    /// [`Error::Failed`] with the engine's error, and [`Error::Shape`] when
    /// its answer does not fit `T`.
    pub fn send(self) -> Result<T, Error> {
        self.connection.wait(self.query.into_future())
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
    /// Names the Run for good: a start with an existing Run ID resumes the
    /// Run if it was interrupted, or returns it as it ended (ADR 0102), as
    /// [`crate::Start::run_id`] says.
    pub fn run_id(self, run_id: &RunId) -> Self {
        Self(self.0.run_id(run_id), self.1)
    }

    /// Sets the Run's own Limits, as [`crate::Start::limits`] does.
    pub fn limits(self, limits: crate::Limits) -> Self {
        Self(self.0.limits(limits), self.1)
    }

    /// Tags the Run, as [`crate::Start::tags`] does.
    pub fn tags(self, tags: Tags) -> Self {
        Self(self.0.tags(tags), self.1)
    }

    /// Starts the Run and blocks the calling thread until it is started.
    ///
    /// # Errors
    ///
    /// What `run.start` fails with.
    pub fn send(self) -> Result<Run, Error> {
        let Self(start, connection) = self;
        let inner = connection.wait(start.into_future())?;
        Ok(Run { inner })
    }
}

impl Query<PageRead, PageReadParams> {
    /// The read whose values are read into `T`, as
    /// [`crate::Query::typed`] reads them.
    ///
    /// # Errors
    ///
    /// Once sent, [`Error::Shape`] when the values do not fit `T`, and
    /// the errors of the read.
    pub fn typed<T: DeserializeOwned + Send + 'static>(
        self,
    ) -> Query<FieldValues<T>, PageReadParams> {
        Query {
            query: self.query.typed(),
            connection: self.connection,
        }
    }
}

/// The response a page's request got, as [`crate::Response`], its body
/// read by blocking the calling thread.
#[derive(Clone, Debug)]
pub struct Response {
    inner: crate::Response,
}

impl Response {
    /// The blocking face of `inner`.
    pub(crate) fn new(inner: crate::Response) -> Self {
        Self { inner }
    }

    /// Its final URL.
    #[must_use]
    pub fn url(&self) -> &str {
        self.inner.url()
    }

    /// Its HTTP status.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.inner.status()
    }

    /// Its status text.
    #[must_use]
    pub fn status_text(&self) -> &str {
        self.inner.status_text()
    }

    /// Whether its status is a success, as [`crate::Response::ok`] says:
    /// a login page answered with a 200 is `ok`.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.inner.ok()
    }

    /// Its headers, their names lower-cased, as the browser received them.
    #[must_use]
    pub fn headers(&self) -> &BTreeMap<String, String> {
        self.inner.headers()
    }

    /// For a navigation's document, the encoding the browser decoded it
    /// with, as [`crate::Response::encoding`].
    #[must_use]
    pub fn encoding(&self) -> Option<&str> {
        self.inner.encoding()
    }

    /// The engine's ID for its request's hop.
    #[must_use]
    pub fn request_id(&self) -> &str {
        self.inner.request_id()
    }

    /// Whether a site's service worker supplied it (ADR 0121).
    #[must_use]
    pub fn from_service_worker(&self) -> bool {
        self.inner.from_service_worker()
    }

    /// Its body's bytes, as [`crate::Response::body`] reads them.
    ///
    /// # Errors
    ///
    /// As [`crate::Response::body`]: `response_truncated` for a body that
    /// broke off and `body_unavailable` for one the browser no longer
    /// holds, whose fix names `page.wait_for_request_finished`.
    pub fn body(&self) -> Result<Vec<u8>, Error> {
        self.inner.connection().wait(self.inner.body())
    }

    /// Its body as text, decoded with the response's own encoding, as
    /// [`crate::Response::text`] decodes it: a legacy charset is honoured,
    /// a BOM left out, unlike Playwright's `text()`, which decodes every
    /// body as UTF-8.
    ///
    /// # Errors
    ///
    /// As [`crate::Response::text`]: `response_truncated` for a body that
    /// broke off, never decoded as whole.
    pub fn text(&self) -> Result<String, Error> {
        self.inner.connection().wait(self.inner.text())
    }

    /// Its body parsed as JSON into `T`, as [`crate::Response::json`]
    /// parses it: UTF-8 whatever its charset, a leading BOM ignored.
    ///
    /// # Errors
    ///
    /// As [`crate::Response::json`]: `response_truncated` for a body that
    /// broke off, and [`Error::ResponseNotJson`] for one that is not JSON.
    pub fn json<T: DeserializeOwned + Send + 'static>(&self) -> Result<T, Error> {
        self.inner.connection().wait(self.inner.json())
    }
}

/// A request a page sent and the response it got, as
/// [`crate::Exchange`], its response's body read by blocking the calling
/// thread.
#[derive(Clone, Debug)]
pub struct Exchange {
    request: PageRequest,
    response: Response,
}

impl Exchange {
    /// The blocking face of `inner`.
    pub(crate) fn new(inner: crate::Exchange) -> Self {
        let (request, response) = inner.into_parts();
        Self {
            request,
            response: Response::new(response),
        }
    }

    /// The request, as the browser sent it.
    #[must_use]
    pub fn request(&self) -> &PageRequest {
        &self.request
    }

    /// The response it got.
    #[must_use]
    pub fn response(&self) -> &Response {
        &self.response
    }
}

impl Run {
    /// Whether the start began the Run, went on with it, or found it ended.
    pub fn started_as(&self) -> StartedAs {
        self.inner.started_as()
    }

    /// Closes the Run for every clone, as [`crate::Run::close`] does, and
    /// returns once it is closed.
    ///
    /// # Errors
    ///
    /// As [`crate::Run::close`].
    pub fn close(&self) -> Result<(), Error> {
        let run = self.inner.clone();
        self.inner
            .host
            .connection()
            .wait(async move { run.close().await })
    }
}

/// C-SEND-SYNC: the blocking handles move between threads, are shared
/// between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn send_static<T: Send + 'static>() {}
    send_sync_static::<Connection>();
    send_sync_static::<Run>();
    send_sync_static::<Page>();
    send_sync_static::<Response>();
    send_sync_static::<Exchange>();
    send_static::<Call<RunReport>>();
    send_static::<Query<Value>>();
    send_static::<Start>();
    send_static::<SaveFile<serde_json::Value>>();
};
