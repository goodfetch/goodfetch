//! Foxhound's connection (ADRs 0090, 0095): [`Foxhound`] carries every method of
//! `goodfetch_collie::Collie` under the same names, Collie's Run start included, plus its own, so
//! moving a program from Collie to Foxhound changes its dependency and its connection type only.
//! [`ConnectOptions`] is Collie's with the SDK handlers of Foxhound's own Plugin kinds (ADR 0088).

use std::path::PathBuf;
use std::time::Duration;

use goodfetch_collie_client::catalog::RawCall;
use goodfetch_collie_client::models::ModelCall;
use goodfetch_collie_client::plugins::ProxyCall;
use goodfetch_collie_client::sessions::SecretRequest;
use goodfetch_collie_client::time::Call;
use goodfetch_collie_client::{
    Connection, EngineChoice, EngineInfo, Error, EventStream, ExplainedSetting, Identities, Limits, RunView, SettingKey, Start,
};

use crate::crawl::{Crawl, Seed, StartCrawl};
use crate::hooks::{FetchCall, SinkCall};
use crate::run::CrawlRunView;

/// Options of [`Foxhound::connect`]: Collie's, and the program's own Fetch backends and Sinks.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ConnectOptions {
    _private: (),
}

impl ConnectOptions {
    /// Which engine to reach, as Collie's.
    pub fn engine(self, engine: EngineChoice) -> Self {
        let _ = engine;
        unimplemented!()
    }

    /// How long starting or finding the engine and `hello` may take.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout for connecting.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// Limits on this Client connection's work (ADR 0041).
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }

    /// Settings for this connection and everything opened beneath it, Foxhound's keys included.
    pub fn settings(self, settings: impl Into<goodfetch_collie_client::Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }

    /// The environment map, read once here (ADRs 0062, 0083).
    pub fn env(self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        let _ = env.into_iter();
        unimplemented!()
    }

    /// The project configuration file.
    pub fn project(self, path: impl Into<PathBuf>) -> Self {
        let _ = path.into();
        unimplemented!()
    }

    /// No project configuration file.
    pub fn no_project(self) -> Self {
        unimplemented!()
    }

    /// Declares the program's own Model provider `name` (ADR 0101), as Collie's.
    pub fn model_provider(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }

    /// Declares the program's Proxy provider `name` (ADR 0104), as Collie's.
    pub fn proxy_provider(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }

    /// Declares that the program answers `caller:` Secret references (ADR 0092), as Collie's.
    pub fn resolve_secret(self) -> Self {
        unimplemented!()
    }

    /// Declares the program's Fetch backend `name`, whose calls [`Foxhound::fetch_requests`] reads
    /// (ADR 0104).
    pub fn fetch_backend(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }

    /// Declares the program's Sink `name`, serving the `sinks.<name>` entry of its name, which then
    /// takes no `type` (ADRs 0103, 0104).
    pub fn sink(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl From<goodfetch_collie_client::ConnectOptions> for ConnectOptions {
    fn from(options: goodfetch_collie_client::ConnectOptions) -> Self {
        let _ = options;
        unimplemented!()
    }
}

/// A connection to a Foxhound engine: the entry point (ADR 0078). A Collie engine refuses it at
/// `connect` with `not_in_product` (ADR 0095). A cheap clone of one connection (ADR 0081).
#[derive(Debug, Clone)]
pub struct Foxhound {
    _private: (),
}

impl Foxhound {
    /// Starts or finds the engine and completes `hello` before returning.
    pub async fn connect(options: impl Into<ConnectOptions>) -> Result<Self, Error> {
        let _ = options.into();
        unimplemented!()
    }

    /// Opens a connection, lends it to `body`, and closes it on every return and error.
    pub async fn scope<T>(options: impl Into<ConnectOptions>, body: impl AsyncFnOnce(&Foxhound) -> Result<T, Error>) -> Result<T, Error> {
        let _ = (options.into(), body);
        unimplemented!()
    }

    /// The engine this connection reached.
    pub fn engine(&self) -> &EngineInfo {
        unimplemented!()
    }

    /// The install's Saved identities (ADR 0092).
    pub fn identities(&self) -> Identities {
        unimplemented!()
    }

    /// Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
    ///
    /// Collie's Run start (ADR 0102), unchanged: a Run with no crawl.
    /// Fails with `model_price_unknown` when a currency Limit or `ai.allowance.price` applies and
    /// the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).
    pub fn start(&self) -> Start {
        unimplemented!()
    }

    /// Opens a Run by its Run ID to read it, never to drive it. Fails with `run_not_found`.
    pub fn open_run(&self, run_id: &str) -> Call<RunView> {
        let _ = run_id;
        unimplemented!()
    }

    /// Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
    ///
    /// Starts a Run of the crawl from its seeds, which define its scope, and returns once it is
    /// started (ADR 0102, Firecrawl's `start_crawl`); [`Crawl::serve`] answers its requests. A
    /// `.run_id(..)` naming an existing Run behaves as [`Foxhound::start`]'s does, the seeds and the
    /// Plan file being inputs that must match.
    /// Fails with `model_price_unknown` when a currency Limit or `ai.allowance.price` applies and
    /// the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).
    pub fn start_crawl(&self, crawl: &Crawl<'_>, seeds: impl IntoIterator<Item = Seed>) -> StartCrawl {
        let _ = (crawl, seeds.into_iter());
        unimplemented!()
    }

    /// Opens a crawl's Run by its Run ID to read it, as [`Foxhound::open_run`] does, with its
    /// outcomes, Records and Exports.
    pub fn open_crawl_run(&self, run_id: &str) -> Call<CrawlRunView> {
        let _ = run_id;
        unimplemented!()
    }

    /// The raw catalog call on the connection, as [`Connection::call`].
    pub fn call(&self, operation: &str) -> RawCall {
        let _ = operation;
        unimplemented!()
    }

    /// The connection's effective settings, with the layer that set each, Foxhound's keys included.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }

    /// The model calls for the declared provider `name`, as Collie's (ADR 0101).
    pub fn model_requests(&self, name: &str) -> Result<EventStream<ModelCall>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// The calls for the Proxy provider `name`, as Collie's (ADR 0104).
    pub fn proxy_requests(&self, name: &str) -> Result<EventStream<ProxyCall>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// The `caller:` references the engine asks this program to resolve, as Collie's (ADR 0104).
    pub fn secret_requests(&self) -> Result<EventStream<SecretRequest>, Error> {
        unimplemented!()
    }

    /// The fetch calls for the Fetch backend `name`, which [`ConnectOptions::fetch_backend`]
    /// declared; another name, or a second stream while one is open, fails with `setting_invalid`.
    pub fn fetch_requests(&self, name: &str) -> Result<EventStream<FetchCall>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// The deliveries for the Sink `name`, which [`ConnectOptions::sink`] declared.
    pub fn sink_requests(&self, name: &str) -> Result<EventStream<SinkCall>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// Cancels in-flight calls, closes every Run it opened, then stops a linked engine or
    /// disconnects from a Daemon, for every clone. Idempotent. Its timeout is the close phase's.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Drop for Foxhound {
    /// Never blocks and never fails (C-DTOR-FAIL, C-DTOR-BLOCK); acts when the last clone is dropped.
    fn drop(&mut self) {}
}

impl Connection for Foxhound {
    fn engine(&self) -> &EngineInfo {
        Foxhound::engine(self)
    }

    fn identities(&self) -> Identities {
        Foxhound::identities(self)
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

    fn open_run(&self, run_id: &str) -> Call<RunView> {
        Foxhound::open_run(self, run_id)
    }
}
