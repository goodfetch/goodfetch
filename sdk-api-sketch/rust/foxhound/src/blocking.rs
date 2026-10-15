//! The blocking twins of Foxhound's own values (ADR 0081): its connection, a crawl's declaration and
//! its handler context, with `goodfetch_collie_client::blocking`'s for every value both Products share.

pub use goodfetch_collie_client::blocking::*;

use goodfetch_collie_client::{EngineInfo, Error, ExplainedSetting, Identities, SettingKey};

use crate::connection::ConnectOptions;
use std::marker::PhantomData;
use std::fmt;

/// [`crate::Foxhound`], blocking.
#[derive(Debug, Clone)]
pub struct Foxhound {
    _private: (),
}

impl Foxhound {
    /// Starts or finds the engine and completes `hello` before returning.
    pub fn connect(options: impl Into<ConnectOptions>) -> Result<Self, Error> {
        let _ = options.into();
        unimplemented!()
    }

    /// The engine this connection reached.
    pub fn engine(&self) -> &EngineInfo {
        unimplemented!()
    }

    /// Collie's Run start, as [`crate::Foxhound::start`] does.
    pub fn start(&self) -> Start {
        unimplemented!()
    }

    /// The raw catalog call on the connection; [`RawCall::send`] sends it.
    pub fn call(&self, operation: &str) -> RawCall {
        let _ = operation;
        unimplemented!()
    }

    /// The connection's effective settings, with the layer that set each.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }

    /// Closes the connection for every clone. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Connection for Foxhound {
    fn engine(&self) -> &EngineInfo {
        Foxhound::engine(self)
    }

    fn identities(&self) -> Identities {
        unimplemented!()
    }

    fn call(&self, operation: &str) -> RawCall {
        Foxhound::call(self, operation)
    }

    fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        Foxhound::explain_settings(self, keys)
    }

    fn close(&self) -> Call<()> {
        Foxhound::close(self)
    }

    fn start(&self) -> Start {
        Foxhound::start(self)
    }
}

/// A Label's declaration for the blocking API, as [`crate::LabelDeclaration`], whose login step is
/// `Fn + Send + Sync + 'static` (ADR 0081).
pub struct LabelDeclaration<P> {
    _payload: PhantomData<fn(P) -> P>,
}

impl<P> fmt::Debug for LabelDeclaration<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LabelDeclaration(..)")
    }
}

impl<P: schemars::JsonSchema + serde::Serialize + serde::de::DeserializeOwned> LabelDeclaration<P> {
    /// A Label named `name` whose Payload is `P`, with the schema `schemars` derives.
    pub fn new(name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl<P> LabelDeclaration<P> {
    /// An explicit Payload schema, as [`crate::LabelDeclaration::payload`].
    pub fn payload(self, schema: crate::Schema<P>) -> Self {
        let _ = schema;
        unimplemented!()
    }

    /// The Label's own settings, as [`crate::LabelDeclaration::settings`].
    pub fn settings(self, settings: impl Into<goodfetch_collie_client::Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }

    /// The Label's login step, as [`crate::LabelDeclaration::login`].
    pub fn login<F>(self, login: F) -> Self
    where
        F: Fn(Page) -> Result<(), Error> + Send + Sync + 'static,
    {
        let _ = login;
        unimplemented!()
    }
}

impl<P: schemars::JsonSchema + serde::Serialize + serde::de::DeserializeOwned> From<&str> for LabelDeclaration<P> {
    fn from(name: &str) -> Self {
        Self::new(name)
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

    /// Declares a Label, as [`crate::Crawl::label`]: a bare name, or a [`LabelDeclaration`] with
    /// its options.
    pub fn label<P>(&mut self, declaration: impl Into<LabelDeclaration<P>>) -> Result<crate::Label<P>, Error> {
        let _ = declaration.into();
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

    /// Attaches the Label's one handler, as [`crate::Crawl::handle`].
    pub fn handle<P, F>(&mut self, label: &crate::Label<P>, handler: F) -> Result<(), Error>
    where
        F: Fn(CrawlContext<P>) -> Result<(), Error> + Send + Sync + 'static,
    {
        let _ = (label, handler);
        unimplemented!()
    }

    /// A before-request Hook, as [`crate::Crawl::before_request`], run on tracked threads. The
    /// other five stages mirror the async crawl's in the same form.
    pub fn before_request<P, F>(&mut self, on: impl Into<crate::HookOn<P>>, hook: F) -> Result<(), Error>
    where
        F: Fn(crate::BeforeRequestContext<P>) -> Result<crate::RequestChange, Error> + Send + Sync + 'static,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }

    /// Answers the Run's crawl requests on tracked threads, `crawl::handler::CONCURRENCY` at once,
    /// blocks until the Run ends, and returns its report.
    /// A handler's panic is caught and ends its attempt with `handler_failed` (ADR 0093).
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

/// What a blocking handler receives: [`crate::CrawlContext`] without `async`. As this module says,
/// only a few calls are sketched; `response`, `tier`, `run`, `enqueue_links` and `enqueue`'s
/// options mirror the async context.
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

