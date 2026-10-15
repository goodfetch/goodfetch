//! A crawl's Run in code (ADR 0102, decided in #478): what its progress and its report add to every
//! Run's, which are the Collie client crate's, and the calls only a crawl's Run has.

use crate::UncertainEffect;
use crate::BlockCause;
use crate::BodyError;
use crate::Call;
use crate::CallFuture;
use crate::CrawlRequestOutcome;
use crate::CrawlRequestRef;
use crate::CrawlRun;
use crate::Dataset;
use crate::DatasetRecord;
use crate::DeclareEffect;
use serde::de::DeserializeOwned;
use crate::Effect;
use crate::Enqueued;
use crate::ErrorCode;
use crate::EventStream;
use crate::ExportCall;
use crate::ExportFormat;
use crate::FetchTier;
use crate::KeyValueStoreReader;
use crate::Label;
use crate::LimitSource;
use crate::NotAddedReason;
use crate::RejectedRecord;
use crate::RunProgress;
use crate::RunReport;
use crate::RunTrace;
use serde::Serialize;
use crate::SkipReason;
use crate::StartedAs;
use std::future::Future;
use std::time::SystemTime;

/// Why a due crawl request is held back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WaitReason {
    /// The domain's slots are full (`politeness.domain.concurrency`, ADR 0086).
    DomainConcurrency,
    /// The domain's delay since its last start (`politeness.domain.delay`).
    DomainDelay,
    /// A host's Crawl-delay, which raised its domain's delay.
    CrawlDelay,
    /// A backoff after a 429, 502, 503, 504, or a connection refused or timed out.
    Backoff,
    /// The origin's robots.txt, not served yet.
    RobotsTxt,
    /// `crawl.handler.concurrency` handlers already running in the program.
    HandlerConcurrency,
    /// ADR 0040's resource governor, admitting nothing new under memory pressure.
    Governor,
    /// Waiting for a Session of the pool, such as one logging in.
    Session,
    /// Its Session waits for a person.
    HumanPause,
    /// A reason this SDK does not know, as received (G3).
    Unknown(String),
}

/// A domain one reason holds.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WaitingDomain {
    /// The domain.
    pub domain: String,
    /// Its requests held.
    pub requests: u64,
    /// When it is next admitted; `None`: once something else happens, such as a person ending a pause.
    pub until: Option<SystemTime>,
}

/// The due requests one reason holds back.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WaitGroup {
    /// Why.
    pub reason: WaitReason,
    /// How many requests.
    pub requests: u64,
    /// The domains it holds, the busiest first.
    pub domains: Vec<WaitingDomain>,
    /// Domains held for this reason beyond those listed.
    pub more_domains: u64,
    /// The setting behind the wait, with its value and layer, where one sets it.
    pub setting: Option<LimitSource>,
}

/// What a request in flight is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestPhase {
    /// From its start until its response arrives (ADR 0086).
    Fetching,
    /// Its handler runs in the program.
    Handling,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// A crawl request being fetched or handled.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InFlightRequest {
    /// Which request.
    pub request: CrawlRequestRef,
    /// What it is doing.
    pub phase: RequestPhase,
    /// The tier fetching it.
    pub tier: FetchTier,
    /// Its attempt, from 1.
    pub attempt: u32,
    /// Since when.
    pub since: SystemTime,
    /// While its handler's own navigation or fetch waits for a permit, which counts against no
    /// timeout (ADR 0093): why, and the setting behind it.
    pub waiting: Option<(WaitReason, Option<LimitSource>)>,
}

/// A sitemap the engine could not read.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct UnreadSitemap {
    /// Its URL.
    pub url: String,
    /// `fetch_failed`, with the response status when one came; `robots_txt`, which disallows it;
    /// `unparsable`.
    pub reason: String,
    /// The response status, when one came.
    pub status: Option<u16>,
}

/// A sitemap seed's sitemaps, read while the Run works (ADR 0093's R3): its entries counted, never
/// listed one by one.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct SitemapTotals {
    /// Sitemaps read.
    pub sitemaps_read: u64,
    /// Sitemaps it could not read: at most a fixed number, the rest counted in
    /// `more_sitemaps_unread`.
    pub sitemaps_unread: Vec<UnreadSitemap>,
    /// Sitemaps not read beyond those listed.
    pub more_sitemaps_unread: u64,
    /// Entries added.
    pub entries_added: u64,
    /// The entries not added, by reason: `NoRule` for one that matched no rule, and an enqueue's
    /// other reasons (ADR 0093).
    pub entries_not_added: Vec<(NotAddedReason, u64)>,
}

/// ADR 0040's resource governor.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GovernorState {
    /// `normal`, `pressure` or `critical`.
    pub level: String,
    /// Browser pages open.
    pub open_pages: u64,
    /// Pages the memory allowance admits.
    pub allowed_pages: u64,
    /// What set the allowance: `cgroup`, `job_object`, `memory_allowance` or `physical_memory`.
    pub allowance_from: String,
}

/// Why the crawl runs as many requests at once as it does (P47).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CrawlConcurrency {
    /// Handlers running in the program.
    pub handlers_running: u64,
    /// `crawl.handler.concurrency`, with its value and layer.
    pub handlers_limit: LimitSource,
    /// The governor.
    pub governor: GovernorState,
}

/// A crawl's requests by state: live ones, then ended ones by outcome.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct CrawlRequestCounts {
    /// In the Frontier, not admitted.
    pub pending: u64,
    /// Being fetched.
    pub fetching: u64,
    /// Being handled in the program.
    pub handling: u64,
    /// In a Human pause.
    pub paused: u64,
    /// Ended `succeeded`.
    pub succeeded: u64,
    /// Ended `failed`.
    pub failed: u64,
    /// Ended `skipped`.
    pub skipped: u64,
    /// Ended `blocked`.
    pub blocked: u64,
}

/// A crawl's progress: what is in flight, what waits and why.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CrawlRunProgress {
    /// What every Run reports.
    pub run: RunProgress,
    /// Its requests by state.
    pub requests: CrawlRequestCounts,
    /// The ended ones by skip reason, Error code and Block cause.
    pub ended: EndedRequestBreakdown,
    /// Why due requests wait.
    pub waiting: Vec<WaitGroup>,
    /// The requests being fetched or handled.
    pub in_flight: Vec<InFlightRequest>,
    /// Why concurrency is what it is.
    pub concurrency: CrawlConcurrency,
    /// True while `crawl::KEEP_ALIVE` holds the Run `running` with an empty Frontier and nothing
    /// in flight, until `add` gives it work; idle time counts toward the Run's own timeout.
    pub idle: bool,
    /// Per sitemap seed's URL.
    pub sitemaps: Vec<(String, SitemapTotals)>,
}

/// A crawl's requests when its Run ended; its Records per Dataset are the report's `records`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CrawlRequestTotals {
    /// Ended `succeeded`.
    pub succeeded: u64,
    /// Ended `failed`.
    pub failed: u64,
    /// Ended `skipped`.
    pub skipped: u64,
    /// Ended `blocked`.
    pub blocked: u64,
    /// Left in the Frontier by a Run that stopped.
    pub pending: u64,
    /// The skipped ones by reason.
    pub by_skip_reason: Vec<(SkipReason, u64)>,
    /// The failed ones by Error code.
    pub by_error_code: Vec<(ErrorCode, u64)>,
    /// The blocked ones by Block cause.
    pub by_block_cause: Vec<(BlockCause, u64)>,
}

/// A Dataset's Records over the Run (ADR 0103).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DatasetTotals {
    /// Committed.
    pub committed: u64,
    /// Dropped by a "Record before commit" Hook.
    pub dropped: u64,
    /// In the rejection table.
    pub rejected: u64,
    /// The dropped ones, by the Hook's reason.
    pub dropped_by_reason: Vec<(String, u64)>,
}

/// A Sink's delivery (ADR 0026), as it stood when the report was read: a Sink may go on delivering
/// after the Run ends.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SinkDeliveryStatus {
    /// `complete`, `complete_with_rejections`, `behind` or `failed` (ADR 0026), or `inactive` for a
    /// Sink inherited from the connection or configuration that names none of the crawl's
    /// Datasets, which delivers nothing (ADR 0103).
    pub status: String,
    /// For an `inactive` Sink, the Dataset names it gave that the crawl does not declare.
    pub undeclared_datasets: Vec<String>,
    /// ADR 0026's Guarantee class.
    pub guarantee_class: String,
    /// Its dead letters.
    pub dead_letters: u64,
    /// Deliveries that fell outside the destination's deduplication window, so were at-least-once.
    pub at_least_once: u64,
}

/// A crawl's Run report.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[must_use]
pub struct CrawlRunReport {
    /// What every Run reports.
    pub run: RunReport,
    /// Its requests.
    pub requests: CrawlRequestTotals,
    /// Per Dataset name.
    pub records: Vec<(String, DatasetTotals)>,
    /// Per Sink name.
    pub sinks: Vec<(String, SinkDeliveryStatus)>,
    /// The Frontier's learned URL rules.
    pub learned_rules: Vec<LearnedRule>,
    /// Per sitemap seed's URL, as the progress has them.
    pub sitemaps: Vec<(String, SitemapTotals)>,
    /// `run.uncertain_effects`, each with the crawl request whose namespace holds it (ADR 0106).
    pub uncertain_effects: Vec<CrawlUncertainEffect>,
}

/// An uncertain Effect of a crawl's Run: one declared in a handler or a Hook sits in its crawl
/// request's namespace (ADR 0106), a field only Foxhound fills, so it is Foxhound's type (ADR 0099).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CrawlUncertainEffect {
    /// The Effect, as every Run reports it.
    pub effect: UncertainEffect,
    /// The crawl request whose namespace holds it; `None` for one the program declared outside a
    /// handler.
    pub request: Option<CrawlRequestRef>,
}

/// A URL rule the Frontier learned, with its evidence (ADR 0005).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LearnedRule {
    /// `no_effect_parameter`, `equivalent_form` (`www.` and the bare host, `http` and `https`, a
    /// trailing slash or an index file, shown one resource by a redirect, page signatures or
    /// `rel=canonical`, ADR 0120), `trap_path` or `cursor_pagination`.
    pub kind: String,
    /// The registrable domain it applies to.
    pub domain: String,
    /// The rule.
    pub rule: String,
    /// What it was learned from.
    pub evidence: String,
    /// How many URLs it merged.
    pub urls_merged: u64,
}

/// The ended requests counted by skip reason, Error code and Block cause.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EndedRequestBreakdown {
    /// By skip reason.
    pub by_skip_reason: Vec<(SkipReason, u64)>,
    /// By Error code.
    pub by_error_code: Vec<(ErrorCode, u64)>,
    /// By Block cause.
    pub by_block_cause: Vec<(BlockCause, u64)>,
}

/// The kinds `outcomes()` can be filtered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutcomeKind {
    /// `succeeded`.
    Succeeded,
    /// `failed`.
    Failed,
    /// `skipped`.
    Skipped,
    /// `blocked`.
    Blocked,
}

/// Which outcomes `outcomes()` reads, filtered in the engine (ADR 0081). The default reads them all.
#[derive(Debug, Clone, Default)]
pub struct OutcomesFilter {
    _private: (),
}

impl OutcomesFilter {
    /// Only these kinds.
    pub fn kinds(self, kinds: &[OutcomeKind]) -> Self {
        let _ = kinds;
        unimplemented!()
    }

    /// Also this Label's requests, where none was named before only these.
    pub fn label<P>(self, label: &Label<P>) -> Self {
        let _ = label;
        unimplemented!()
    }

    /// From the next commit, not the Run's first outcome: failures as they happen, in place of a
    /// failed-request callback (ADR 0093's R4).
    pub fn after_now(self) -> Self {
        unimplemented!()
    }
}

/// Which current failures `failed_requests()` reads. The default reads the requests whose latest
/// outcome is `failed`.
#[derive(Debug, Clone, Default)]
pub struct FailedRequestsFilter {
    _private: (),
}

impl FailedRequestsFilter {
    /// Also the requests whose latest outcome is `blocked`, as [`RetryFailed::blocked`].
    pub fn blocked(self) -> Self {
        unimplemented!()
    }
}

/// Re-admitting a crawl's failed requests, not sent until awaited.
#[derive(Debug)]
#[must_use = "nothing is re-admitted until this is awaited"]
pub struct RetryFailed {
    _private: (),
}

impl RetryFailed {
    /// Re-admit the requests that ended `blocked` too: a block may have lifted (ADR 0021).
    pub fn blocked(self) -> Self {
        unimplemented!()
    }
}

impl IntoFuture for RetryFailed {
    /// How many requests it re-admitted.
    type Output = Result<u64, crate::results::Error>;
    type IntoFuture = CallFuture<u64>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

impl CrawlRun {
    /// How the start opened this Run.
    pub fn started_as(&self) -> &StartedAs {
        unimplemented!()
    }

    /// Each seed's answer, given once at the start: added with its ID, or not added with its reason.
    /// On a resume, the answers given when the Run began.
    pub fn seeds(&self) -> &[Enqueued] {
        unimplemented!()
    }

    /// Lets the crawl finish: once its Frontier is empty and nothing is in flight, or, under
    /// `crawl::KEEP_ALIVE`, once drained after this call. [`crate::Crawl::serve`] returns the same
    /// report when the Run ends; this future does not serve requests, so it ends a keep-alive crawl
    /// from another task while `serve` runs.
    pub fn finish(&self) -> Call<CrawlRunReport> {
        unimplemented!()
    }

    /// Stops the crawl at its next safe point with a Checkpoint, as [`Run::stop`] does.
    pub fn stop(&self) -> Call<CrawlRunReport> {
        unimplemented!()
    }

    /// The crawl's progress, as [`Run::progress`]'s.
    pub fn progress(&self) -> EventStream<CrawlRunProgress> {
        unimplemented!()
    }

    /// The Run trace recorded so far, as [`Run::trace`]'s.
    pub fn trace(&self) -> Call<Option<RunTrace>> {
        unimplemented!()
    }

    /// Every ending in commit order, a request ending again after `retry_failed`: a log of endings,
    /// not current state; `failed_requests()` reads the current failures.
    ///
    /// The crawl requests' outcomes, read from the Run's committed state in commit order from its
    /// first, then following new commits until the Run ends. A reader of committed state, it never
    /// slows the Run. A request [`CrawlRun::retry_failed`] re-admits ends again, and its new outcome
    /// supersedes the old: key outcomes by `request.id`.
    pub fn outcomes(&self, filter: OutcomesFilter) -> EventStream<CrawlRequestOutcome> {
        let _ = filter;
        unimplemented!()
    }

    /// The crawl requests whose latest outcome is `failed` (with `.blocked()`, also those whose
    /// latest is `blocked`), each as that outcome, read from committed state at the call and ending
    /// after what is committed: exactly what `retry_failed` with the same option would re-admit
    /// (ADR 0117). Never slows the Run.
    pub fn failed_requests(&self, filter: FailedRequestsFilter) -> EventStream<CrawlRequestOutcome> {
        let _ = filter;
        unimplemented!()
    }

    /// Re-admits, in this same Run, every request that ended `failed` (and `blocked` with
    /// `.blocked()`), each keeping its ID, Label, Payload and depth, its attempts counting on with a
    /// fresh `crawl::retry::MAX`. A skip is a decision and is never re-admitted. On an ended Run it
    /// returns the Run to `running`, its Verdict and Completeness given anew when it ends (ADR 0029).
    pub fn retry_failed(&self) -> RetryFailed {
        unimplemented!()
    }
}

/// A crawl's Run read by a program that does not drive it. Its Records are read with the program's
/// own Dataset handles. A cheap clone.
#[derive(Debug, Clone)]
pub struct CrawlRunView {
    _private: (),
}

impl CrawlRunView {
    /// The Run's ID.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// The crawl's progress.
    pub fn progress(&self) -> EventStream<CrawlRunProgress> {
        unimplemented!()
    }

    /// The crawl's report, as [`RunView::report`]'s.
    pub fn report(&self) -> Call<CrawlRunReport> {
        unimplemented!()
    }

    /// The Run trace recorded so far, as [`RunView::trace`]'s.
    pub fn trace(&self) -> Call<Option<RunTrace>> {
        unimplemented!()
    }

    /// The outcomes, as [`CrawlRun::outcomes`]'s: a log of endings, not current state.
    pub fn outcomes(&self, filter: OutcomesFilter) -> EventStream<CrawlRequestOutcome> {
        let _ = filter;
        unimplemented!()
    }

    /// The crawl requests whose latest outcome is `failed`, as [`CrawlRun::failed_requests`]'s:
    /// exactly what `retry_failed` with the same option would re-admit (ADR 0117).
    pub fn failed_requests(&self, filter: FailedRequestsFilter) -> EventStream<CrawlRequestOutcome> {
        let _ = filter;
        unimplemented!()
    }

    /// The Dataset's committed Records, as `CrawlRun::records`' (ADR 0103); on a Run with no live
    /// writer, it ends after the last committed Record, as `outcomes` does.
    pub fn records<T: DeserializeOwned>(&self, dataset: &Dataset<T>) -> EventStream<DatasetRecord<T>> {
        let _ = dataset;
        unimplemented!()
    }

    /// The Dataset's rejection table.
    pub fn rejected_records<T>(&self, dataset: &Dataset<T>) -> EventStream<RejectedRecord> {
        let _ = dataset;
        unimplemented!()
    }

    /// An Export of the Records committed so far (ADR 0026).
    pub fn export<T>(&self, dataset: &Dataset<T>, format: ExportFormat) -> ExportCall {
        let _ = (dataset, format);
        unimplemented!()
    }

    /// Its Key-value store's reads.
    pub fn key_value_store(&self) -> KeyValueStoreReader {
        unimplemented!()
    }

    /// Frees the view; the Run goes on.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl CrawlRun {
    /// As [`Run::effect`]. Through a handler's or a Hook's `run()`, bound to its crawl request, the
    /// key's namespace is that request; through the program's own value, the Run.
    pub fn effect<'a, T, E, F, Fut>(&'a self, key: &str, body: F) -> DeclareEffect<'a, T, E, F>
    where
        T: Serialize + DeserializeOwned + Send + 'a,
        E: BodyError + Send + 'a,
        F: FnOnce(Effect) -> Fut + Send + 'a,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
    {
        let _ = (key, body);
        unimplemented!()
    }
}

