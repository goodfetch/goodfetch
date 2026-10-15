//! Crawl definition and handler context (ADR 0093, decided in #477).
//!
//! A crawl is declared by a [`Crawl`], made with no connection and no I/O: each Label is declared
//! first, returning its typed [`Label`], and then gets exactly one handler with [`Crawl::handle`].
//! A Run carries the crawl out: [`Foxhound::start_crawl`] starts it, and a start under the same ID
//! resumes it (ADR 0102). The async API spawns nothing (ADR 0081): [`Crawl::serve`] answers the Run's crawl
//! requests inside the future the program awaits, so handlers may borrow local state and need
//! neither `Send` nor `'static`. [`crate::blocking`] runs `Send + Sync + 'static` handlers on the
//! `tasks` module's tracked threads instead.
//!
//! A handler receives one [`CrawlContext`] on every fetch tier: Collie's page over the request's
//! document. Every tier but `browser` serves what it can exactly; a call only a browser can serve,
//! or a wait for something a static document lacks, ends the attempt and runs the request again in
//! the browser, unless `fetch::TIER` forces the tier, when it fails with `browser_required`.

use std::fmt;
use std::future::IntoFuture;
use std::marker::PhantomData;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::concurrency::EventStream;
use crate::Schema;
use crate::data::{Dataset, Label};
use crate::lifecycle::{Page, Run};
use crate::page::{Locator, Response};
use crate::results::Error;
use crate::time::{Call, CallFuture, Limits, UrlMatcher};

/// A URL pattern matched in the engine (ADR 0081). No predicate: every link would cross the
/// connection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UrlPattern {
    /// A glob, such as `**/catalogue/category/**`.
    Glob(String),
    /// A regular expression in the Rust `regex` syntax.
    Regex(String),
}

impl From<&str> for UrlPattern {
    fn from(glob: &str) -> Self {
        Self::Glob(glob.to_owned())
    }
}

/// The fetch tiers of ADR 0020.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchTier {
    /// Plain HTTP.
    Http,
    /// The JSON calls a browser sample made, replayed.
    DiscoveredApi,
    /// HTTP sent from inside the Session's browser, so its identity does not change.
    BrowserFetch,
    /// ADR 0020's optional light engine: scripts without layout.
    LightBrowser,
    /// The browser.
    Browser,
    /// A tier this SDK does not know, as received (G3).
    Unknown(String),
}

/// A crawl's declaration: its Labels and their handlers. It needs no connection and does no I/O.
/// A Run is started from it and resumed with it, since handlers are never persisted.
pub struct Crawl<'a> {
    _handlers: PhantomData<&'a ()>,
}

impl fmt::Debug for Crawl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Crawl(..)")
    }
}

impl Default for Crawl<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Crawl<'a> {
    /// An empty declaration.
    pub fn new() -> Self {
        unimplemented!()
    }

    /// Declares a Label and returns its typed handle. A bare name declares one whose Payload is `P`
    /// (`()` for none), with the schema `schemars` derives; a [`LabelDeclaration`] adds its options
    /// (ADR 0099: one name, with `impl Into` for the second argument shape). A second Label of the
    /// same name fails at once with `label_duplicate`, with no I/O.
    pub fn label<P>(&mut self, declaration: impl Into<LabelDeclaration<'a, P>>) -> Result<Label<P>, Error> {
        let _ = declaration.into();
        unimplemented!()
    }

    /// Declares a Dataset and returns its typed handle (ADR 0103): there is no default Dataset, so
    /// every push names one. A bare name declares a Dataset of `T` with the schema `schemars`
    /// derives (`serde_json::Map` takes any JSON object); a [`DatasetDeclaration`] adds its schema
    /// and key (ADR 0099: one name, with `impl Into` for the second argument shape). A second
    /// Dataset of the same name fails at once with `dataset_duplicate`.
    pub fn dataset<T>(&mut self, declaration: impl Into<DatasetDeclaration<T>>) -> Result<Dataset<T>, Error> {
        let _ = declaration.into();
        unimplemented!()
    }

    /// Attaches the Label's one handler. A second handler fails at once with `label_duplicate`; a
    /// Label left without one makes the Run's start fail with `label_unhandled`.
    pub fn handle<P, F>(&mut self, label: &Label<P>, handler: F) -> Result<(), Error>
    where
        F: AsyncFn(CrawlContext<P>) -> Result<(), Error> + 'a,
    {
        let _ = (label, handler);
        unimplemented!()
    }

    /// The scripted login step of the Run's Session pool (ADRs 0006, 0092), called with the
    /// Session's page on the login page; a Label's own, from [`LabelDeclaration::login`], wins.
    pub fn login<F>(&mut self, login: F)
    where
        F: AsyncFn(Page) -> Result<(), Error> + 'a,
    {
        let _ = login;
        unimplemented!()
    }

    /// Answers the Run's crawl requests with these handlers and Hooks (ADR 0104),
    /// `crawl::handler::CONCURRENCY` requests at once, inside this future, until the Run ends, then
    /// returns its report (ADR 0102): serving is what lets a crawl finish. Dropping it is the
    /// program's own stop: it sends `$/cancel`, and the requests it leased return to pending
    /// without counting an attempt. A handler's panic is caught and ends its attempt with
    /// `handler_failed`, as an exception does in TypeScript and Python.
    pub async fn serve(&self, run: &CrawlRun) -> Result<crate::run::CrawlRunReport, Error> {
        let _ = run;
        unimplemented!()
    }
}

/// A Dataset's declaration with its options, the builder methods named after the options of
/// TypeScript's `crawl.dataset(name, { schema, key })` (ADR 0099). A `&str` converts into one with
/// none.
pub struct DatasetDeclaration<T> {
    _record: PhantomData<fn(T) -> T>,
}

impl<T> fmt::Debug for DatasetDeclaration<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatasetDeclaration(..)")
    }
}

impl<T: JsonSchema + Serialize + DeserializeOwned> DatasetDeclaration<T> {
    /// A Dataset named `name` of `T`, with the schema `schemars` derives.
    pub fn new(name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl<T> DatasetDeclaration<T> {
    /// An explicit schema in place of the derived one; [`Schema::from_json`] is the way down.
    pub fn schema(self, schema: Schema<T>) -> Self {
        let _ = schema;
        unimplemented!()
    }

    /// The top-level fields that identify a Record, which Sinks' keyed write modes and Baseline
    /// comparisons use. A field the schema does not require fails with `dataset_key_invalid`.
    pub fn key(self, key: &[&str]) -> Self {
        let _ = key;
        unimplemented!()
    }
}

impl<T: JsonSchema + Serialize + DeserializeOwned> From<&str> for DatasetDeclaration<T> {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

/// A Label's declaration with its options, the builder methods named after the options of
/// TypeScript's `crawl.label(name, { payload, settings, login })` (ADR 0099). A `&str` converts into
/// one with none.
pub struct LabelDeclaration<'a, P> {
    _payload: PhantomData<fn(P) -> P>,
    _handlers: PhantomData<&'a ()>,
}

impl<P> fmt::Debug for LabelDeclaration<'_, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LabelDeclaration(..)")
    }
}

impl<'a, P: JsonSchema + Serialize + DeserializeOwned> LabelDeclaration<'a, P> {
    /// A Label named `name` whose Payload is `P`, with the schema `schemars` derives.
    pub fn new(name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl<'a, P> LabelDeclaration<'a, P> {
    /// An explicit Payload schema in place of the derived one; [`Schema::from_json`] is the way
    /// down.
    pub fn payload(self, schema: Schema<P>) -> Self {
        let _ = schema;
        unimplemented!()
    }

    /// The Label's own settings: a scope between the Run and each request (ADR 0093). What a
    /// Session holds (`session.source`, `session.http_credentials`, `secrets.*`, or a login from
    /// [`LabelDeclaration::login`]) gives its requests a Session pool of their own, while
    /// `session.page_check` judges its pages in whichever pool serves them; the Session's identity,
    /// Politeness, Site preferences and scope are refused with `setting_not_applicable`.
    pub fn settings(self, settings: impl Into<goodfetch_collie_client::Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }

    /// The scripted login step of the Sessions this Label's requests use, which wins over the
    /// crawl's [`Crawl::login`] and gives the Label a Session pool of its own (ADR 0093).
    pub fn login<F>(self, login: F) -> Self
    where
        F: AsyncFn(Page) -> Result<(), Error> + 'a,
    {
        let _ = login;
        unimplemented!()
    }
}

impl<'a, P: JsonSchema + Serialize + DeserializeOwned> From<&str> for LabelDeclaration<'a, P> {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

/// A seed: a crawl request the program gives at the start, checked as an enqueue is. On a resume,
/// the seeds must match the Run's (`run_conflict`).
#[derive(Debug, Clone)]
pub struct Seed {
    _private: (),
}

impl Seed {
    /// A seed of `label`, with its Payload (`()` for none).
    pub fn new<P: Serialize>(url: &str, label: &Label<P>, payload: P) -> Self {
        let _ = (url, label, payload);
        unimplemented!()
    }

    /// Seeds from a sitemap, or a site's robots.txt whose `Sitemap:` lines are read (ADR 0093;
    /// ADR 0037's CO-10). The first rule whose pattern matches picks the Label, one without a
    /// Payload; an entry matching none is not added and is reported as `no_rule`, and nested
    /// sitemap indexes are followed. Its entries are checked against the scope at depth 0.
    pub fn sitemap(url: &str, rules: impl IntoIterator<Item = (UrlPattern, Label<()>)>) -> Self {
        let _ = (url, rules.into_iter());
        unimplemented!()
    }

    /// The request's method, `GET` by default (ADR 0093).
    pub fn method(self, method: &str) -> Self {
        let _ = method;
        unimplemented!()
    }

    /// One request header.
    pub fn header(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }

    /// The request's body.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }
}

/// A link `enqueue_links_with` matched, as its Payload function sees it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LinkInfo {
    /// The resolved `href`.
    pub href: String,
    /// The link's text.
    pub text: String,
    /// Its attributes.
    pub attributes: Vec<(String, String)>,
}

/// One crawl request, as its handler sees it. The engine's fields are kept apart from the Payload
/// (ADR 0082).
#[derive(Debug)]
pub struct CrawlRequest<P> {
    _payload: PhantomData<P>,
}

impl<P> CrawlRequest<P> {
    /// Its ID.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// The URL the Frontier admitted, after its URL rules (ADR 0005). The final URL is
    /// `page().url()`.
    pub fn url(&self) -> &str {
        unimplemented!()
    }

    /// The request's own Label, the handle to enqueue more of the same kind (pagination).
    pub fn label(&self) -> &Label<P> {
        unimplemented!()
    }

    /// The Payload, parsed by the Label's schema.
    pub fn payload(&self) -> &P {
        unimplemented!()
    }

    /// 0 for a seed, one more for each enqueue from a handler.
    pub fn depth(&self) -> u32 {
        unimplemented!()
    }

    /// 1 for the first attempt. Escalations and sample-comparison runs are not retries.
    pub fn attempt(&self) -> u32 {
        unimplemented!()
    }
}

/// What a handler receives, the same on every fetch tier. A context belongs to one attempt: once
/// it has ended (the handler's return, `skip`, `retry`, an escalation, the handler's timeout, a
/// lost lease, the Run's close), every call
/// on it fails with `attempt_ended`. An attempt whose handler returned commits what it staged;
/// one that ended early discards it.
#[derive(Debug)]
pub struct CrawlContext<P> {
    _payload: PhantomData<P>,
}

impl<P> CrawlContext<P> {
    /// The crawl request.
    pub fn request(&self) -> &CrawlRequest<P> {
        unimplemented!()
    }

    /// Collie's page (ADR 0085), with its AI verbs, fenced by the work item's lease and journaled.
    pub fn page(&self) -> &Page {
        unimplemented!()
    }

    /// The main document's final response, after redirects; its body is always kept.
    pub fn response(&self) -> &Response {
        unimplemented!()
    }

    /// The first response kept for this attempt whose URL matches (ADR 0110): kept because its
    /// Label's `fetch::kept_responses::URLS` names it, on the request's page and its frames, from
    /// the start of its navigation, so one the page loaded before the handler ran is found. Waits
    /// for one under `timeout.wait`, as a waiter armed when the navigation started, so it needs no
    /// `during`, and resolves once its body has arrived, so the body never fails unless past
    /// `fetch::kept_responses::MAX_SIZE` (`body_unavailable`). Past the timeout,
    /// `deadline_exceeded` naming the matcher; on a Label that keeps nothing, `responses_not_kept`
    /// at once. On `http` and `browser_fetch` it escalates (`browser_required` under a forced
    /// tier); `discovered_api` answers from its replayed calls.
    pub fn kept_response(&self, url: impl Into<UrlMatcher>) -> Call<Response> {
        let _ = url.into();
        unimplemented!()
    }

    /// The responses kept for this attempt so far, in arrival order, without waiting; `url`
    /// filters them. Same rules as [`CrawlContext::kept_response`].
    pub fn kept_responses(&self, url: Option<UrlMatcher>) -> Call<Vec<Response>> {
        let _ = url;
        unimplemented!()
    }

    /// The tier that fetched this attempt.
    pub fn tier(&self) -> &FetchTier {
        unimplemented!()
    }

    /// Reads `stream`, one of this page's, for the rest of the attempt, inside [`Crawl::serve`],
    /// calling `handler` with each item, as [`crate::PageStageContext::for_each`] does (ADR 0111).
    /// On a context read from [`CrawlRun::crawl_requests`] it fails with `setting_invalid`.
    pub fn for_each<T, F>(&self, stream: EventStream<T>, handler: F) -> Result<(), Error>
    where
        T: 'static,
        F: AsyncFn(T) -> Result<(), Error> + 'static,
    {
        let _ = (stream, handler);
        unimplemented!()
    }

    /// The crawl's Run, bound to this attempt (ADR 0093): `add_seed` and its Key-value store's
    /// writes through it are staged with the attempt, and its reads see them first. The program's
    /// own `run` value writes at once. `push` is in [`crate::output`].
    pub fn run(&self) -> &CrawlRun {
        unimplemented!()
    }

    /// Adds one URL to the Frontier, checked against the crawl's scope; a Payload failing its
    /// schema fails with `payload_invalid`. Staged with the attempt (ADR 0003).
    pub fn enqueue<Q: Serialize>(&self, url: &str, label: &Label<Q>, payload: Q) -> Enqueue {
        let _ = (url, label, payload);
        unimplemented!()
    }

    /// Adds every link the locator matches, each `href` resolved against the document's base URL.
    pub fn enqueue_links<Q: Serialize>(&self, links: &Locator, label: &Label<Q>, payload: Q) -> EnqueueLinks {
        let _ = (links, label, payload);
        unimplemented!()
    }

    /// As [`CrawlContext::enqueue_links`], with a Payload per link: `payload` returns it, or `None`
    /// to skip that link, reported as `skipped_by_payload` (ADR 0093).
    pub fn enqueue_links_with<Q: Serialize, F>(&self, links: &Locator, label: &Label<Q>, payload: F) -> EnqueueLinks
    where
        F: Fn(&LinkInfo) -> Option<Q>,
    {
        let _ = (links, label, payload);
        unimplemented!()
    }

    /// Ends the request `skipped`, with the reason `handler` and this message: return it,
    /// `return Err(ctx.skip("…"))`. Its code is `attempt_ended`, which the helper consumes.
    pub fn skip(&self, message: &str) -> Error {
        let _ = message;
        unimplemented!()
    }

    /// Ends this attempt with `retry_requested`, retried under `crawl::retry::MAX`: return it,
    /// `return Err(ctx.retry("…"))`.
    pub fn retry(&self, message: &str) -> Error {
        let _ = message;
        unimplemented!()
    }
}

/// Why a URL was not added.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotAddedReason {
    /// The Frontier already holds it, pending or visited.
    Duplicate,
    /// Outside the crawl's scope.
    OutOfScope,
    /// A host the run's domain policy refuses (ADRs 0014, 0017).
    DomainPolicy,
    /// Beyond `crawl::MAX_DEPTH`.
    MaxDepth,
    /// Beyond `crawl::MAX_REQUESTS`.
    MaxRequests,
    /// Not `http` or `https`.
    UnsupportedScheme,
    /// A matched element with no link.
    NoLink,
    /// A sitemap entry matching no rule of its sitemap seed.
    NoRule,
    /// A link the Payload function of `enqueue_links_with` skipped.
    SkippedByPayload,
    /// A reason this SDK does not know, as received.
    Unknown(String),
}

/// What an enqueue did: an answer, never an error (ADR 0079).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Enqueued {
    /// Added, or staged with the attempt from a handler.
    Added {
        /// The new request's ID.
        id: String,
        /// Its URL, after the Frontier's rules.
        url: String,
    },
    /// Not added.
    NotAdded {
        /// The URL, or `None` for a matched element with no link.
        url: Option<String>,
        /// Why.
        reason: NotAddedReason,
    },
}

/// What `enqueue_links` did with each element its locator matched.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EnqueuedLinks {
    /// The links added, as [`Enqueued::Added`].
    pub added: Vec<Enqueued>,
    /// The links not added, as [`Enqueued::NotAdded`].
    pub not_added: Vec<Enqueued>,
}

/// One enqueue, not sent until awaited.
#[derive(Debug)]
#[must_use = "an enqueue does nothing until it is awaited"]
pub struct Enqueue {
    _private: (),
}

impl Enqueue {
    /// The request's method, `GET` by default (ADR 0093). A crawl request is a read whatever its
    /// method, sent again on a retry, an escalation or a sample comparison; an Effect goes through
    /// `page.request` inside `ctx.run().effect(…)` (ADR 0106).
    pub fn method(self, method: &str) -> Self {
        let _ = method;
        unimplemented!()
    }

    /// One request header.
    pub fn header(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }

    /// The request's body. The Frontier's identity is the method, the URL and a hash of the body.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }

    /// Higher first, within its domain (ADRs 0004, 0005). Default 0.
    pub fn priority(self, priority: i32) -> Self {
        let _ = priority;
        unimplemented!()
    }

    /// Ahead of the domain's pending requests at its priority (ADR 0005).
    pub fn forefront(self) -> Self {
        unimplemented!()
    }

    /// The request's identity in the Frontier instead of its method, URL and body hash: a declared
    /// canonicalization, which always wins (ADR 0005).
    pub fn key(self, key: &str) -> Self {
        let _ = key;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl IntoFuture for Enqueue {
    type Output = Result<Enqueued, Error>;
    type IntoFuture = CallFuture<Enqueued>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// One `enqueue_links`, not sent until awaited.
#[derive(Debug)]
#[must_use = "an enqueue does nothing until it is awaited"]
pub struct EnqueueLinks {
    _private: (),
}

impl EnqueueLinks {
    /// Only links matching one of these, within the crawl's scope, which they narrow and never widen.
    pub fn include(self, patterns: impl IntoIterator<Item = UrlPattern>) -> Self {
        let _ = patterns.into_iter();
        unimplemented!()
    }

    /// No link matching any of these.
    pub fn exclude(self, patterns: impl IntoIterator<Item = UrlPattern>) -> Self {
        let _ = patterns.into_iter();
        unimplemented!()
    }

    /// At most this many links added: the first in document order, once `include` and `exclude`
    /// have filtered them.
    pub fn limit(self, limit: u32) -> Self {
        let _ = limit;
        unimplemented!()
    }

    /// Higher first, within its domain.
    pub fn priority(self, priority: i32) -> Self {
        let _ = priority;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl IntoFuture for EnqueueLinks {
    type Output = Result<EnqueuedLinks, Error>;
    type IntoFuture = CallFuture<EnqueuedLinks>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// One crawl request leased to the program, for a program that answers them itself: the way down
/// under [`Crawl::serve`] (G8). Dropped unanswered, it returns to pending without counting an
/// attempt; only a lost process or connection counts, as `handler_lost`.
#[derive(Debug)]
pub struct CrawlCall {
    _private: (),
}

impl CrawlCall {
    /// The request's context, its Payload as JSON.
    pub fn context(&self) -> &CrawlContext<serde_json::Value> {
        unimplemented!()
    }

    /// Ends the attempt with the handler's result.
    pub async fn finish(self, result: Result<(), Error>) -> Result<(), Error> {
        let _ = result;
        unimplemented!()
    }
}

/// A crawl's Run (ADR 0102); its run-level calls are in `run.rs`, and its Records, rejection table,
/// Exports and Key-value store are ADR 0103's ([`crate::output`]). A cheap clone (ADR 0081).
#[derive(Debug, Clone)]
pub struct CrawlRun {
    _private: (),
}

impl CrawlRun {
    /// The Run's ID.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// The Run's calls common to every Run.
    pub fn as_run(&self) -> &Run {
        unimplemented!()
    }

    /// Adds a seed to the running crawl (ADR 0037's CO-03), committed at once, or staged with the
    /// attempt when called through a handler's `ctx.run()`: a [`Seed::new`], or a [`Seed::sitemap`]
    /// whose entries are checked as enqueues are, those matching no rule reported as `no_rule` in
    /// the Run's report (ADR 0093). A seed widens the scope by its host, where a handler's `enqueue`
    /// is checked against it; one the domain policy refuses is not added.
    pub fn add_seed(&self, seed: impl Into<Seed>) -> Enqueue {
        let _ = seed.into();
        unimplemented!()
    }

    /// The crawl requests leased to this program, in order: the way down under [`Crawl::serve`].
    pub fn crawl_requests(&self) -> EventStream<CrawlCall> {
        unimplemented!()
    }
}

/// Starting a crawl's Run, not sent until awaited. Fails with `label_unhandled` when a Label has
/// no handler.
#[derive(Debug)]
#[must_use = "a Run does not start until this is awaited"]
pub struct StartCrawl {
    _private: (),
}

impl StartCrawl {
    /// The Run's ID, as [`crate::Start::id`]'s.
    pub fn id(self, id: impl Into<String>) -> Self {
        let _ = id.into();
        unimplemented!()
    }

    /// The Run's Plan file (ADR 0091), an input of the Run like its seeds.
    pub fn plan(self, path: impl AsRef<std::path::Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// The Run's Response recording (ADR 0105), an input like its seeds: replayed, or recorded into.
    pub fn recording(self, recording: crate::recording::Recording) -> Self {
        let _ = recording;
        unimplemented!()
    }

    /// Settings for this Run and everything opened beneath it (ADR 0083).
    pub fn settings(self, settings: impl Into<goodfetch_collie_client::Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }

    /// How long the whole Run may last (ADR 0080).
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// The Run's Limits.
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }
}

impl IntoFuture for StartCrawl {
    type Output = Result<CrawlRun, Error>;
    type IntoFuture = CallFuture<CrawlRun>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}
