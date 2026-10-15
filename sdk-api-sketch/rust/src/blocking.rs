//! The same API without `async` (ADR 0081), generated from the same catalog as Python's sync API.
//!
//! Each call hands its work to the connection's runtime and waits on a channel. It never nests a
//! runtime, so it also works inside an async runtime, where it only blocks that thread. A call is a
//! builder ended by [`Call::send`], as in `reqwest::blocking`; a stream is an [`Iterator`]. Handles
//! are cheap clones, safe to use from any thread. Helpers that answer requests run handlers on the
//! `tasks` module's tracked blocking threads (ADR 0060), so blocking handlers are
//! `Fn + Send + Sync + 'static`. Only a few calls are sketched here; the rest mirror the async API.

use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use crate::concurrency::events;
use crate::lifecycle::ConnectOptions;
use crate::results::Error;
use crate::page::Response;
use crate::time::Limits;

/// One blocking call: set its timeout, then [`Call::send`] it.
#[must_use = "a call does nothing until it is sent"]
pub struct Call<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Call<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Call(..)")
    }
}

impl<T> Call<T> {
    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// Sends the call and blocks until it ends.
    pub fn send(self) -> Result<T, Error> {
        unimplemented!()
    }
}

/// What the engine sends on its own, read in order; dropping it unsubscribes without blocking.
pub struct EventIter<T> {
    _item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for EventIter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EventIter(..)")
    }
}

impl<T> Iterator for EventIter<T> {
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        unimplemented!()
    }
}

/// The engine connection, blocking.
#[derive(Debug, Clone)]
pub struct Goodfetch {
    _private: (),
}

impl Goodfetch {
    /// Starts or finds the engine and completes `hello` before returning.
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// Collie, reached without I/O.
    pub fn collie(&self) -> Collie {
        unimplemented!()
    }

    /// Closes the connection for every clone. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// Collie, blocking.
#[derive(Debug, Clone)]
pub struct Collie {
    _private: (),
}

impl Collie {
    /// Starts a Run, as [`crate::Collie::start`] does.
    pub fn start(&self) -> Start {
        unimplemented!()
    }
}

/// Starting a Run, not sent until [`Start::send`].
#[derive(Debug)]
#[must_use = "a Run does not start until this is sent"]
pub struct Start {
    _private: (),
}

impl Start {
    /// The Run's ID, as [`crate::Start::id`]'s.
    pub fn id(self, id: impl Into<String>) -> Self {
        let _ = id.into();
        unimplemented!()
    }

    /// How long the whole Run may last.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// The Run's Limits.
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }

    /// Starts the Run and blocks until it has started.
    pub fn send(self) -> Result<Run, Error> {
        unimplemented!()
    }
}

/// A Run, blocking.
#[derive(Debug, Clone)]
pub struct Run {
    _private: (),
}

impl Run {
    /// Opens a page.
    pub fn new_page(&self) -> Call<Page> {
        unimplemented!()
    }

    /// Stops the Run if unfinished. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A page, blocking.
#[derive(Debug, Clone)]
pub struct Page {
    _private: (),
}

impl Page {
    /// Placeholder for #472's navigation.
    pub fn goto(&self, url: &str) -> Call<Option<Response>> {
        let _ = url;
        unimplemented!()
    }

    /// The event's iterator, subscribed when called.
    pub fn events<E: events::PageEvent>(&self, event: E) -> EventIter<E::Data> {
        let _ = event;
        unimplemented!()
    }

    /// Closes the tab. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A crawl's declaration for the blocking API (ADR 0093): its handlers run on the `tasks` module's
/// tracked blocking threads, so they are `Fn + Send + Sync + 'static` (ADR 0081).
pub struct Crawl {
    _private: (),
}

impl fmt::Debug for Crawl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Crawl(..)")
    }
}

impl Crawl {
    /// An empty declaration.
    pub fn new() -> Self {
        unimplemented!()
    }

    /// Declares a Label, as [`crate::Crawl::label`].
    pub fn label<P: schemars::JsonSchema + serde::Serialize + serde::de::DeserializeOwned>(
        &mut self,
        name: &str,
    ) -> Result<crate::Label<P>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// Declares a Label with an explicit schema and its settings, as [`crate::Crawl::label_with`].
    pub fn label_with<P>(&mut self, name: &str, payload: crate::Schema<P>, settings: crate::Settings) -> Result<crate::Label<P>, Error> {
        let _ = (name, payload, settings);
        unimplemented!()
    }

    /// The crawl's login step, as [`crate::Crawl::login`].
    pub fn login<F>(&mut self, login: F)
    where
        F: Fn(Page) -> Result<(), Error> + Send + Sync + 'static,
    {
        let _ = login;
        unimplemented!()
    }

    /// A Label's login step, as [`crate::Crawl::login_for`].
    pub fn login_for<P, F>(&mut self, label: &crate::Label<P>, login: F) -> Result<(), Error>
    where
        F: Fn(Page) -> Result<(), Error> + Send + Sync + 'static,
    {
        let _ = (label, login);
        unimplemented!()
    }

    /// Attaches the Label's one handler, as [`crate::Crawl::handle`].
    pub fn handle<P, F>(&mut self, label: &crate::Label<P>, handler: F) -> Result<(), Error>
    where
        F: Fn(CrawlContext<P>) -> Result<(), Error> + Send + Sync + 'static,
    {
        let _ = (label, handler);
        unimplemented!()
    }

    /// Answers the Run's crawl requests on tracked threads, `crawl::handler::CONCURRENCY` at once,
    /// blocks until the Run ends, and returns its report.
    pub fn serve(&self, run: &crate::CrawlRun) -> Result<crate::CrawlRunReport, Error> {
        let _ = run;
        unimplemented!()
    }
}

impl Default for Crawl {
    fn default() -> Self {
        Self::new()
    }
}

/// What a blocking handler receives: [`crate::CrawlContext`] without `async`.
#[derive(Debug)]
pub struct CrawlContext<P> {
    _payload: PhantomData<P>,
}

impl<P> CrawlContext<P> {
    /// The crawl request.
    pub fn request(&self) -> &crate::CrawlRequest<P> {
        unimplemented!()
    }

    /// Collie's page.
    pub fn page(&self) -> &Page {
        unimplemented!()
    }

    /// Adds one URL to the Frontier, as the async context does.
    pub fn enqueue<Q: serde::Serialize>(&self, url: &str, label: &crate::Label<Q>, payload: Q) -> Call<crate::Enqueued> {
        let _ = (url, label, payload);
        unimplemented!()
    }

    /// Stages a Record with this attempt, as the async `push` does (ADR 0103).
    pub fn push<T: serde::Serialize>(&self, dataset: &crate::Dataset<T>, record: &T) -> Call<()> {
        let _ = (dataset, record);
        unimplemented!()
    }

    /// Ends the request `skipped`: `return Err(ctx.skip("…"))`.
    pub fn skip(&self, message: &str) -> Error {
        let _ = message;
        unimplemented!()
    }

    /// Ends this attempt with `retry_requested`: `return Err(ctx.retry("…"))`.
    pub fn retry(&self, message: &str) -> Error {
        let _ = message;
        unimplemented!()
    }
}

/// Any blocking connection, as [`crate::Connection`] (ADR 0099): what `Collie` and `Foxhound`
/// share in `blocking`.
pub trait Connection {
    /// The raw catalog call, ended by [`Call::send`].
    fn call(&self, operation: &str, params: crate::JsonObject) -> Call<serde_json::Value>;

    /// Closes the connection for every clone. Idempotent.
    fn close(&self) -> Call<()>;
}

impl Connection for Goodfetch {
    fn call(&self, operation: &str, params: crate::JsonObject) -> Call<serde_json::Value> {
        Goodfetch::call(self, operation, params)
    }

    fn close(&self) -> Call<()> {
        Goodfetch::close(self)
    }
}

impl Goodfetch {
    /// The raw catalog call on the connection, ended by [`Call::send`].
    pub fn call(&self, operation: &str, params: crate::JsonObject) -> Call<serde_json::Value> {
        let _ = (operation, params);
        unimplemented!()
    }
}
