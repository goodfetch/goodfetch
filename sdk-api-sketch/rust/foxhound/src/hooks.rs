//! Hooks and SDK handlers in code (ADR 0104, decided in #481).
//!
//! A crawl declares its Hooks in code on its [`Crawl`], one method per stage, at the six stages
//! whose answer changes the work. Hooks run in the order declared, after the Hooks that settings
//! name (`hooks.*`), with no integers. Like handlers, they are `AsyncFn` closures answered inside
//! [`Crawl::serve`], so they may borrow and need neither `Send` nor `'static` (ADR 0081). The
//! observe-only stages (the run's lifecycle and a request's end) reach code through the Run's own
//! values: its progress, outcomes and report (ADR 0102). A Collie program has no Hooks in code: its
//! own lines, routes, init scripts and locator handlers answer each stage (ADR 0085).
//!
//! A Hook returns its change as a value and ends the work by returning the error a call on its
//! context gives (`Err(ctx.skip("…"))`), as a handler does (ADR 0093).
//!
//! A program's own implementation of a Plugin kind, an SDK handler, answers the kind's calls,
//! read as an [`EventStream`] by name, as ADR 0101 reads `model_requests` and ADR 0092
//! `secret_requests`, since the async API spawns nothing (ADR 0081).

use std::marker::PhantomData;

use serde_json::{Map, Value};

use crate::concurrency::EventStream;
use crate::crawl::{Crawl, CrawlRequest, CrawlRun, FetchTier};
use crate::data::{Dataset, Label};
use crate::lifecycle::Page;
use crate::page::{Headers, Response};
use crate::results::{BlockCause, Error, ErrorCode};
use crate::sessions::TextInput;
use crate::HandlerError;
use crate::HookStage;

/// A Record of a Dataset declared with no schema: any JSON object (ADR 0082).
pub type UntypedRecord = Map<String, Value>;

/// Which requests a Hook runs for, and its name. From `&label`, its Payload is typed; from
/// [`HookOn::every_label`], it is a JSON value.
#[derive(Debug)]
pub struct HookOn<P> {
    _payload: PhantomData<fn(P) -> P>,
}

impl HookOn<Value> {
    /// Every Label's requests.
    pub fn every_label() -> Self {
        unimplemented!()
    }

    /// Narrows the scope to this Label too: several Labels see their Payload as a JSON value. A
    /// Label of another crawl fails the declaration with `label_unknown`.
    pub fn only<Q>(self, label: &Label<Q>) -> Self {
        let _ = label;
        unimplemented!()
    }
}

impl<P> HookOn<P> {
    /// Names the Hook in `hook_failed`, the Run trace and a test's `run_hook`, unique on its crawl
    /// or `hook_duplicate` at once; by default the stage and its position (`before_request#2`),
    /// since a closure has no name.
    pub fn named(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl<P> From<&Label<P>> for HookOn<P> {
    fn from(label: &Label<P>) -> Self {
        let _ = label;
        unimplemented!()
    }
}

/// Which Records a Hook before their commit runs for. From `&dataset`, it sees and returns the
/// Dataset's Record type.
#[derive(Debug)]
pub struct RecordHookOn<T> {
    _record: PhantomData<fn(T) -> T>,
}

impl RecordHookOn<UntypedRecord> {
    /// Every Dataset's Records.
    pub fn every_dataset() -> Self {
        unimplemented!()
    }
}

impl<T> RecordHookOn<T> {
    /// Only the Records of this Label's requests.
    pub fn label<P>(self, label: &Label<P>) -> Self {
        let _ = label;
        unimplemented!()
    }

    /// Names the Hook in `hook_failed` and the Run trace.
    pub fn named(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

impl<T> From<&Dataset<T>> for RecordHookOn<T> {
    fn from(dataset: &Dataset<T>) -> Self {
        let _ = dataset;
        unimplemented!()
    }
}

/// Before each fetch of a request: every attempt, escalation and sample-comparison run comes here
/// again, so this is also the stage before a retry. A context belongs to one call.
#[derive(Debug)]
pub struct BeforeRequestContext<P> {
    _payload: PhantomData<P>,
}

impl<P> BeforeRequestContext<P> {
    /// The crawl request.
    pub fn request(&self) -> &CrawlRequest<P> {
        unimplemented!()
    }

    /// The crawl's Run, for its Key-value store (#479).
    pub fn run(&self) -> &CrawlRun {
        unimplemented!()
    }

    /// The tier about to fetch.
    pub fn tier(&self) -> &FetchTier {
        unimplemented!()
    }

    /// The method the request was enqueued with, `GET` by default (ADR 0093). A Hook reads it and
    /// never changes it.
    pub fn method(&self) -> &str {
        unimplemented!()
    }

    /// The body the request was enqueued with, if any. A Hook reads it (to sign the request, say)
    /// and never changes it.
    pub fn body(&self) -> Option<&[u8]> {
        unimplemented!()
    }

    /// The headers as built so far, the enqueue's own and the Hooks before this one included. Cookies, Secret values
    /// and the Session's credentials are added after the last Hook, so none of them is here.
    pub fn headers(&self) -> &Headers {
        unimplemented!()
    }

    /// The error of the attempt before this one; `None` on the first.
    pub fn previous_error(&self) -> Option<&Error> {
        unimplemented!()
    }

    /// Ends the request `skipped`, with the reason `hook` and this message: return it,
    /// `return Err(ctx.skip("…"))`.
    pub fn skip(&self, message: &str) -> Error {
        let _ = message;
        unimplemented!()
    }
}

/// What a before-request Hook changes: the URL and headers, never the method or the body the request
/// was enqueued with (ADR 0093).
#[derive(Debug, Clone, Default)]
#[must_use = "a change does nothing until the Hook returns it"]
pub struct RequestChange {
    _private: (),
}

impl RequestChange {
    /// No change.
    pub fn none() -> Self {
        Self::default()
    }

    /// A new URL, checked against the crawl's scope and the run's domain policy (not an enqueue's
    /// other checks; a Hook in code may reach any host that admits),
    /// else the request ends `skipped`. robots.txt and Site preferences read it as rewritten.
    pub fn url(self, url: &str) -> Self {
        let _ = url;
        unimplemented!()
    }

    /// Sets a header. `Host`, `Cookie`, `Authorization`, `Proxy-*` and method overrides fail the
    /// Hook with `header_forbidden`. A value may be a Secret placeholder, checked against the
    /// request's URL (ADR 0092).
    pub fn header(self, name: &str, value: impl Into<TextInput>) -> Self {
        let _ = (name, value.into());
        unimplemented!()
    }

    /// Removes a header.
    pub fn remove_header(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }
}

/// After the final response, before ADR 0093's status rule: where a soft block served with a 200
/// is marked.
#[derive(Debug)]
pub struct AfterResponseContext<P> {
    _payload: PhantomData<P>,
}

impl<P> AfterResponseContext<P> {
    /// The crawl request.
    pub fn request(&self) -> &CrawlRequest<P> {
        unimplemented!()
    }

    /// The crawl's Run.
    pub fn run(&self) -> &CrawlRun {
        unimplemented!()
    }

    /// The tier that fetched.
    pub fn tier(&self) -> &FetchTier {
        unimplemented!()
    }

    /// The final response, after redirects.
    pub fn response(&self) -> &Response {
        unimplemented!()
    }

    /// Ends the attempt `blocked` with this cause: return it. ADR 0021 escalates by new Session,
    /// and the request ends `blocked` once escalation runs out.
    pub fn block(&self, cause: BlockCause, message: &str) -> Error {
        let _ = (cause, message);
        unimplemented!()
    }

    /// Ends this attempt with `retry_requested`, retried under `crawl::retry::MAX`: return it.
    pub fn retry(&self, message: &str) -> Error {
        let _ = message;
        unimplemented!()
    }
}

/// What an after-response Hook changes.
#[derive(Debug, Clone, Default)]
#[must_use = "a change does nothing until the Hook returns it"]
pub struct ResponseChange {
    _private: (),
}

impl ResponseChange {
    /// No change.
    pub fn none() -> Self {
        Self::default()
    }

    /// A rewritten body, which the handler's page is served from.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }
}

/// Before and after a navigation (only on the `browser` and `light_browser` tiers), and when the page
/// is ready, just before the handler, on every tier. Page calls follow the handler's tier rules (ADR
/// 0093).
#[derive(Debug)]
pub struct PageStageContext<P> {
    _payload: PhantomData<P>,
}

impl<P> PageStageContext<P> {
    /// The stage: before navigation, after navigation or page ready.
    pub fn stage(&self) -> &HookStage {
        unimplemented!()
    }

    /// The crawl request.
    pub fn request(&self) -> &CrawlRequest<P> {
        unimplemented!()
    }

    /// The crawl's Run.
    pub fn run(&self) -> &CrawlRun {
        unimplemented!()
    }

    /// The tier of this attempt.
    pub fn tier(&self) -> &FetchTier {
        unimplemented!()
    }

    /// Collie's page (ADR 0085).
    pub fn page(&self) -> &Page {
        unimplemented!()
    }

    /// Reads `stream`, one of this page's (its routes, its events, dialogs included, or a locator
    /// handler's turns), for the rest of the attempt, inside [`Crawl::serve`], calling `handler`
    /// with each item (ADR 0111): what a Hook subscribes before navigation lasts past its return.
    /// Routes are answered concurrently, other items one at a time in order. A failing `handler`
    /// follows its kind's rule: a route's request is aborted and the route removed, then
    /// `page_handler_failed` (ADR 0085); a listener is removed, then `listener_failed` (ADR 0081);
    /// a dialog gets its default; a locator handler fails the action it interrupted and stays. On
    /// a context read from [`CrawlRun::hook_requests`], which no `serve` answers, it fails at once
    /// with `setting_invalid`: the program reads the stream itself.
    pub fn for_each<T, F>(&self, stream: EventStream<T>, handler: F) -> Result<(), Error>
    where
        T: 'static,
        F: AsyncFn(T) -> Result<(), Error> + 'static,
    {
        let _ = (stream, handler);
        unimplemented!()
    }
}

/// Before a Record's commit, with the attempt's other outputs (ADR 0003). The engine checks the
/// Record it commits against the Dataset's schema.
#[derive(Debug)]
pub struct RecordCommitContext<T> {
    _record: PhantomData<T>,
}

impl<T> RecordCommitContext<T> {
    /// The crawl request whose attempt pushed the Record.
    pub fn request(&self) -> &CrawlRequest<Value> {
        unimplemented!()
    }

    /// The crawl's Run.
    pub fn run(&self) -> &CrawlRun {
        unimplemented!()
    }

    /// The Dataset.
    pub fn dataset(&self) -> &Dataset<T> {
        unimplemented!()
    }

    /// The Record, taken by value to return changed.
    pub fn into_record(self) -> T {
        unimplemented!()
    }

    /// The Record.
    pub fn record(&self) -> &T {
        unimplemented!()
    }

    /// Leaves the Record out, counted in the run's report with this reason: return it.
    pub fn drop_record(&self, reason: &str) -> Error {
        let _ = reason;
        unimplemented!()
    }

    /// Puts the Record in the Dataset's rejection table with this reason, read with
    /// `run.rejected_records` (ADR 0103): return it.
    pub fn reject(&self, reason: &str) -> Error {
        let _ = reason;
        unimplemented!()
    }
}

// Hooks in code, declared on the crawl beside its handlers.
impl<'a> Crawl<'a> {
    /// Before each fetch of a request, retries, escalations and sample-comparison runs included:
    /// it may rewrite the URL, set headers or skip.
    pub fn before_request<P, F>(&mut self, on: impl Into<HookOn<P>>, hook: F) -> Result<(), Error>
    where
        F: AsyncFn(BeforeRequestContext<P>) -> Result<RequestChange, Error> + 'a,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }

    /// After the final response, before the status rule: it may rewrite the body, mark a block or
    /// ask for a retry.
    pub fn after_response<P, F>(&mut self, on: impl Into<HookOn<P>>, hook: F) -> Result<(), Error>
    where
        F: AsyncFn(AfterResponseContext<P>) -> Result<ResponseChange, Error> + 'a,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }

    /// Before the browser navigates: set cookies, block resources, add init scripts.
    pub fn before_navigation<P, F>(&mut self, on: impl Into<HookOn<P>>, hook: F) -> Result<(), Error>
    where
        F: AsyncFn(PageStageContext<P>) -> Result<(), Error> + 'a,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }

    /// After the browser navigated, before the engine's own page work (consent, challenges, the
    /// expected page).
    pub fn after_navigation<P, F>(&mut self, on: impl Into<HookOn<P>>, hook: F) -> Result<(), Error>
    where
        F: AsyncFn(PageStageContext<P>) -> Result<(), Error> + 'a,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }

    /// Just before the handler, on every tier, with the handler's page: the router middleware of
    /// Crawlee's `router.use`.
    pub fn page_ready<P, F>(&mut self, on: impl Into<HookOn<P>>, hook: F) -> Result<(), Error>
    where
        F: AsyncFn(PageStageContext<P>) -> Result<(), Error> + 'a,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }

    /// Before each Record's commit: it returns the Record to commit, changed or not, or drops or
    /// rejects it.
    pub fn before_record_commit<T, F>(&mut self, on: impl Into<RecordHookOn<T>>, hook: F) -> Result<(), Error>
    where
        F: AsyncFn(RecordCommitContext<T>) -> Result<T, Error> + 'a,
    {
        let _ = (on.into(), hook);
        unimplemented!()
    }
}

/// One call to a Hook in code, for a program that answers [`CrawlRun::hook_requests`] itself
/// instead of [`Crawl::serve`] (G8). Its Payload and Record are JSON values.
#[derive(Debug)]
#[non_exhaustive]
pub enum HookCall {
    /// Before request.
    BeforeRequest(BeforeRequestContext<Value>),
    /// After response.
    AfterResponse(AfterResponseContext<Value>),
    /// Before navigation, after navigation or page ready, as [`PageStageContext::stage`] says.
    Page(PageStageContext<Value>),
    /// Before a Record's commit.
    RecordCommit(RecordCommitContext<UntypedRecord>),
}

/// A Hook call's answer, matching its stage.
#[derive(Debug)]
#[non_exhaustive]
pub enum HookAnswer {
    /// Before request's change.
    Request(RequestChange),
    /// After response's change.
    Response(ResponseChange),
    /// A page stage's.
    Page,
    /// The Record to commit.
    Record(UntypedRecord),
}

impl HookCall {
    /// The Hook's name.
    pub fn hook(&self) -> &str {
        unimplemented!()
    }

    /// Ends the call with the Hook's answer, or with an error that ends the work.
    pub async fn finish(self, answer: Result<HookAnswer, Error>) -> Result<(), Error> {
        let _ = answer;
        unimplemented!()
    }
}

impl CrawlRun {
    /// The calls to the crawl's Hooks in code, in order, beside `crawl_requests` (ADR 0093): the
    /// way down under [`Crawl::serve`]. Dropped unanswered, a call fails its work as a Hook past
    /// `timeout::HOOK` does.
    pub fn hook_requests(&self) -> EventStream<HookCall> {
        unimplemented!()
    }
}

/// Details of `hook_failed`: the Hook, then the error as `handler_failed` carries a handler's.
/// For the program's own error, not retried unless `crawl::retry::HANDLER_FAILED` is on; for a
/// Goodfetch code returned inside, retried when that code is.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HookFailed {
    /// The stage.
    pub stage: HookStage,
    /// The Hook's name.
    pub hook: String,
    /// The request's Label; `None` at a lifecycle stage, which has no request.
    pub label: Option<String>,
    /// The error's type name.
    pub error_type: String,
    /// Its message.
    pub error_message: String,
    /// The backtrace frames of the program's code, when captured.
    pub stack: Vec<String>,
    /// The Goodfetch code returned inside the Hook, whose retryable flag the error carries; `None`
    /// for the program's own error.
    pub wrapped_code: Option<ErrorCode>,
}

/// Details of `header_forbidden`: a before-request Hook set a header the engine owns.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HeaderForbidden {
    /// The header.
    pub header: String,
}

// SDK handlers: a program's own implementation of a Plugin kind (ADR 0033), its one form with every
// engine, a linked one included. `ConnectOptions` declares each handler's name, one builder method
// per kind, and the connection's `*_requests(name)` reads its calls. Settings choose it by that name as they
// choose an installed Plugin. The only one of a kind that one setting chooses (Fetch backend, Proxy
// provider) becomes that setting's value at the connection scope, with the run-layer origin
// `Registered`, the lowest: every other run-layer value replaces it, configuration does not, and a
// lock wins with no refusal. Two of such a kind fail `connect` with `setting_invalid` when nothing
// else sets the key, as does a name a built-in or an installed Plugin of its kind already has. A
// setting naming a handler neither installed nor declared fails with `setting_invalid` when its scope opens. A call waits for its
// stream until its own deadline; one dropped unanswered, or held by a stream dropped mid-Run, fails
// with its kind's code for an unreachable implementation (ADR 0101). Collie's SDK has no Fetch
// backend or Sink (ADR 0088).
// Each kind's calls are generated from its plugin-protocol method group (area 7's spec): the
// shapes below are provisional.

/// A target the engine already checked under the page class and the run's domain policy (ADR 0033).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BackendRequest {
    /// The target.
    pub url: String,
    /// The crawl request's own method (ADR 0093).
    pub method: String,
    /// The headers to send.
    pub headers: Headers,
    /// The crawl request's own body, if any.
    pub body: Option<Vec<u8>>,
    /// The Session's proxy server, with no credentials, if it has one: a proxied crawl does not
    /// leave from the user's own address.
    pub proxy: Option<String>,
}

/// A Fetch backend's answer; a redirect's target comes back as a new request, checked again.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BackendResponse {
    /// The status.
    pub status: u16,
    /// The headers.
    pub headers: Headers,
    /// The body.
    pub body: Vec<u8>,
}

impl BackendResponse {
    /// A response.
    pub fn new(status: u16, headers: Headers, body: Vec<u8>) -> Self {
        Self { status, headers, body }
    }
}

/// One call to the program's Fetch backend. Like a third-party backend, it never carries a request
/// that uses a Secret or a logged-in identity, such as a logged-in Session's, one whose Hook set a
/// Secret placeholder, one whose Session has `session.http_credentials` or one whose proxy has a
/// password: the engine sends those itself, so no logged-in cookie or Secret value passes through
/// the program. Other requests carry an anonymous Session's cookies in `headers`, and a response's
/// `Set-Cookie` updates the Session.
#[derive(Debug)]
pub struct FetchCall {
    _private: (),
}

impl FetchCall {
    /// What the engine asks.
    pub fn request(&self) -> &BackendRequest {
        unimplemented!()
    }

    /// Sends the answer to the engine.
    pub fn answer(self, response: BackendResponse) {
        let _ = response;
        unimplemented!()
    }

    /// Fails the call: not retried unless `crawl::retry::HANDLER_FAILED` is on, unless a Goodfetch
    /// code says otherwise.
    pub fn fail(self, error: HandlerError) {
        let _ = error;
        unimplemented!()
    }
}

/// One delivery to the program's Sink (ADR 0026's SDK-handler form), serving the `sinks.<name>`
/// entry of its name: the same delivery ID on each retry (ADR 0008), so a write keyed by it lands
/// once.
#[derive(Debug)]
pub struct SinkCall {
    _private: (),
}

impl SinkCall {
    /// The delivery ID.
    pub fn delivery_id(&self) -> &str {
        unimplemented!()
    }

    /// The run epoch.
    pub fn run_epoch(&self) -> u64 {
        unimplemented!()
    }

    /// The Dataset's name.
    pub fn dataset(&self) -> &str {
        unimplemented!()
    }

    /// Committed Records, in commit order.
    pub fn records(&self) -> &[UntypedRecord] {
        unimplemented!()
    }

    /// Acknowledges the delivery.
    pub fn delivered(self) {
        unimplemented!()
    }

    /// Fails the delivery, retried on ADR 0008's schedule.
    pub fn fail(self, error: HandlerError) {
        let _ = error;
        unimplemented!()
    }
}

/// Details of `hook_ineffective`: a Hook that can never run.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HookIneffective {
    /// The Hook's name.
    pub hook: String,
    /// Its stage.
    pub stage: HookStage,
}

/// Details of `hook_duplicate`: a second Hook of one name on one crawl.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HookDuplicate {
    /// The name.
    pub hook: String,
}
