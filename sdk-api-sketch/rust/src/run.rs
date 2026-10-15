//! The Run in code (ADR 0102, decided in #478).
//!
//! - A Run starts with [`crate::Collie::start`] or [`crate::Foxhound::start_crawl`], which return once
//!   it is started. Its ID, chosen with `.id(..)` or generated, names it for good: a start whose ID
//!   names an existing Run resumes it, returns it ended ([`StartedAs`]), or refuses, and nothing is
//!   inferred from a storage folder.
//! - [`Run::finish`] lets the Run finish and returns its [`RunReport`]; [`Run::stop`] ends it early
//!   with its report; ADR 0078's `close()` is unchanged. A crawl's [`crate::Crawl::serve`] returns its
//!   [`CrawlRunReport`] once the Run ends, since serving is what lets it finish.
//! - Progress is a telemetry [`EventStream`] (ADRs 0024, 0081); a crawl's outcomes are read from its
//!   committed state, so a slow reader never slows the Run.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::concurrency::EventStream;
use crate::crawl::{CrawlRun, Enqueued, FetchTier, Foxhound};
use crate::data::{Dataset, Label, Schema};
use crate::lifecycle::{Collie, Run};
use crate::output::{DatasetRecord, ExportCall, ExportFormat, RejectedRecord, StoredBytes};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use crate::models::RunUsage;
use crate::results::{BlockCause, CrawlRequestOutcome, CrawlRequestRef, ErrorCode, LimitSource, SkipReason, Verdict};
use crate::time::{Call, CallFuture};
use std::future::IntoFuture;

/// How a start opened its Run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartedAs {
    /// A new Run.
    New,
    /// Resumed from its committed state: the run file on this machine, or an imported Checkpoint
    /// (ADR 0002).
    Resumed,
    /// The ID named a Run that had already finished, returned as it is with nothing run. A call that
    /// would give it new work fails with `run_ended`.
    Ended,
    /// A value this SDK does not know, as received (G3).
    Unknown(String),
}

/// Where a Run is (ADR 0029). A Run that ends `failed` makes `finish()` and `stop()` fail with its
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Lifecycle {
    /// Running.
    Running,
    /// Reached its end.
    Finished,
    /// Ended early; see [`RunStopReason`].
    Stopped,
    /// Ended by an error in the engine or the user's code.
    Failed,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// Whether a Run's results cover its scope (ADRs 0005, 0029). A Collie Run its program finishes is
/// `Complete`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Completeness {
    /// Everything in scope.
    Complete,
    /// Ended early.
    Incomplete,
    /// Finished, but the engine cannot vouch that nothing was missed.
    Unverified,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// Why a Run stopped: ADR 0029's causes, and its own timeout (ADR 0080).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunStopReason {
    /// A Limit, by name.
    Limit(String),
    /// The Run's own timeout.
    Timeout,
    /// An abort, and by whom.
    Abort(AbortedBy),
    /// The loss of its engine.
    EngineLost,
    /// Exhausted resources (ADR 0040).
    Resources,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// Who aborted a Run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AbortedBy {
    /// Its program's own `stop()` or `close()`.
    Program,
    /// A person, in the Console.
    Person,
    /// ADR 0036's scheduler.
    Schedule,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// Who raised a Human pause.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PauseRaisedBy {
    /// The program, with `page.pause()` (ADR 0092).
    Program,
    /// The engine, such as ADR 0006's login recovery.
    Engine,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// A Human pause waiting for a person.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WaitingPause {
    /// The Session whose work waits.
    pub session: String,
    /// The program's reason, or the engine's own.
    pub reason: Option<String>,
    /// Who raised it.
    pub raised_by: PauseRaisedBy,
    /// Since when.
    pub since: SystemTime,
}

/// A Run's progress: a telemetry value, coalesced under load (ADR 0024).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunProgress {
    /// Where the Run is.
    pub lifecycle: Lifecycle,
    /// False when nothing works the Run, as after a crash, until a start with its ID resumes it.
    pub has_writer: bool,
    /// The Human pauses waiting.
    pub pauses: Vec<WaitingPause>,
    /// The program's own line, set with [`Run::set_progress_message`] (ADR 0037's RT-15).
    pub message: Option<String>,
    /// When the engine took this value.
    pub at: SystemTime,
}

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
}

/// A Checkpoint, as ADR 0002 reports it, written where the engine keeps the Run's: its own folder
/// (ADR 0031), or wherever Goodfetch Cloud stores them. Copied to this machine as ADR 0103's Export is.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Checkpoint {
    /// Where the engine wrote it, on the engine's side.
    pub location: String,
    /// In bytes.
    pub size: u64,
    /// Its digest.
    pub digest: String,
    /// The run epoch (ADR 0003).
    pub epoch: u64,
    /// The journal position.
    pub journal_position: u64,
    /// When.
    pub taken_at: SystemTime,
}

/// Why a Run did nothing (G1).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NothingDone {
    /// The start found the Run ended.
    AlreadyEnded {
        /// When it ended.
        ended_at: SystemTime,
    },
    /// No seed reached a handler: each was refused when added or skipped when fetched.
    NoSeedHandled {
        /// The seeds' reasons, counted.
        by_reason: Vec<(SkipReason, u64)>,
    },
    /// A resume or a retry found nothing left.
    NothingPending,
    /// A reason this SDK does not know, as received.
    Unknown(String),
}

/// A Run report: what a Run states about itself when it ends.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[must_use]
pub struct RunReport {
    /// The Run's ID.
    pub id: String,
    /// `Finished` or `Stopped`: a `Failed` Run fails the call instead.
    pub lifecycle: Lifecycle,
    /// Set when `lifecycle` is `Stopped`.
    pub stop_reason: Option<RunStopReason>,
    /// Its Verdict.
    pub verdict: Verdict,
    /// Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects.
    pub verdict_reasons: Vec<String>,
    /// Each Effect left uncertain, waiting for a person to reconcile it in the Console (ADRs 0029,
    /// 0106).
    pub uncertain_effects: Vec<crate::effects::UncertainEffect>,
    /// Its Completeness.
    pub completeness: Completeness,
    /// Why it is not `complete`: stopped early, a bound reached, offset pagination, pages left at
    /// `payment_required` (ADR 0005).
    pub completeness_reasons: Vec<String>,
    /// The final figures (ADR 0101), the uncapped mark included (ADR 0041).
    pub usage: RunUsage,
    /// What became of the Run's Plan (ADR 0091): every Run has one, its file `None` when it named none.
    pub plan: crate::ai::PlanReport,
    /// The Operator policy version it ran under (ADR 0031).
    pub policy_version: Option<String>,
    /// Each effective value, with the layer that set it and the one that locked it (ADR 0028).
    pub settings: Vec<crate::settings::ExplainedSetting>,
    /// The Plugins that ran.
    pub plugins: Vec<PluginRun>,
    /// Its warnings.
    pub warnings: Vec<RunWarning>,
    /// Every setting that weakened a safeguard for this Run (ADR 0028): a costly control switched
    /// off (ADR 0023), the "no trace" mark included (ADR 0031), or a value pressing a site harder
    /// (ADR 0086).
    pub weakened: Vec<LimitSource>,
    /// The last Checkpoint written.
    pub checkpoint: Option<Checkpoint>,
    /// Why the Run did nothing, when it did nothing.
    pub nothing_done: Option<NothingDone>,
    /// The Run's page in the Console, where its Run trace replays, when the engine knows a Console
    /// address (ADR 0031).
    pub console_url: Option<String>,
    /// What the Run did with its Response recording (ADR 0105): `None` when it named none. A
    /// replayed Run says so here.
    pub recording: Option<crate::recording::RecordingReport>,
    /// When it started.
    pub started_at: SystemTime,
    /// When it ended.
    pub ended_at: SystemTime,
}

/// A crawl's requests when its Run ended. Records per Dataset are #479's to add.
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
    /// `complete`, `complete_with_rejections`, `behind` or `failed`.
    pub status: String,
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
}

impl Run {
    /// How the start opened this Run.
    pub fn started_as(&self) -> &StartedAs {
        unimplemented!()
    }

    /// Lets the Run finish, ends it `finished`, closes its Sessions and pages, and returns its
    /// report: at once for a Collie Run, whose program says it is done; when the engine ends it for
    /// a detached Run. If something stops the Run first (a Limit, its timeout, a person, the
    /// engine), the report says `Stopped`. Its wait has no timeout of its own: the Run's own bounds
    /// it, and `.timeout(..)` or dropping the future ends only the wait.
    pub fn finish(&self) -> Call<RunReport> {
        unimplemented!()
    }

    /// Stops the Run at its next safe point with a Checkpoint, `stopped` and `incomplete` as an
    /// abort by its program, and returns its report. Resumable by its ID.
    pub fn stop(&self) -> Call<RunReport> {
        unimplemented!()
    }

    /// The Run's progress: its first value at once, then the latest after each change, until the
    /// Run ends. Subscribed when called; take one value and drop it to read one.
    pub fn progress(&self) -> EventStream<RunProgress> {
        unimplemented!()
    }

    /// The program's own status line, shown in `progress()` and the Console.
    pub fn set_progress_message(&self, message: &str) -> Call<()> {
        let _ = message;
        unimplemented!()
    }

    /// Writes a Checkpoint now (ADR 0002), where the engine keeps the Run's.
    pub fn checkpoint(&self) -> Call<Checkpoint> {
        unimplemented!()
    }
}

impl Checkpoint {
    /// Copies it to a file on this machine, wherever the engine runs.
    pub fn save_as(&self, path: impl AsRef<Path>) -> Call<()> {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// Its bytes, streamed to this machine.
    pub fn stream(&self) -> EventStream<Vec<u8>> {
        unimplemented!()
    }
}

/// A Plugin that ran in the Run (ADRs 0016, 0033).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginRun {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The network level it ran at (ADR 0033).
    pub network_level: String,
    /// Where Operator policy allowed reduced confinement, the parts the host could not provide.
    pub missing_confinement: Vec<String>,
}

/// A warning the Run carries: a `stealth` choice (ADR 0035), stale Clean-up rules (ADR 0047),
/// throttled CPU (ADR 0049), a degraded Politeness store (ADR 0053), a Plugin's reported figures
/// (ADR 0033). A newer engine's code is kept.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunWarning {
    /// Its code.
    pub code: String,
    /// Its message.
    pub message: String,
}

/// A URL rule the Frontier learned, with its evidence (ADR 0005).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LearnedRule {
    /// `no_effect_parameter`, `trap_path` or `cursor_pagination`.
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

    /// The crawl requests' outcomes, read from the Run's committed state in commit order from its
    /// first, then following new commits until the Run ends. A reader of committed state, it never
    /// slows the Run. A request [`CrawlRun::retry_failed`] re-admits ends again, and its new outcome
    /// supersedes the old: key outcomes by `request.id`.
    pub fn outcomes(&self, filter: OutcomesFilter) -> EventStream<CrawlRequestOutcome> {
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

impl Collie {
    /// Opens a Run by its ID to read it, never to drive it: a detached Run, another program's, or
    /// one that ended. Fails with `run_not_found`.
    pub fn open_run(&self, id: &str) -> Call<RunView> {
        let _ = id;
        unimplemented!()
    }
}

impl Foxhound {
    /// Opens a crawl's Run by its ID to read it, as [`Collie::open_run`] does, with its outcomes,
    /// Records and Exports.
    pub fn open_crawl_run(&self, id: &str) -> Call<CrawlRunView> {
        let _ = id;
        unimplemented!()
    }
}

/// A Run read by a program that does not drive it (ADR 0102). It never gives the Run work and
/// takes no writer lock (ADR 0031); closing it frees only the view. A cheap clone.
#[derive(Debug, Clone)]
pub struct RunView {
    _private: (),
}

impl RunView {
    /// The Run's ID.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// The Run's progress, as [`Run::progress`]'s.
    pub fn progress(&self) -> EventStream<RunProgress> {
        unimplemented!()
    }

    /// The Run's report: at once for an ended Run, at its end for a live one, which it never ends.
    /// On a Run with no live writer that has not ended, fails with `run_interrupted`.
    pub fn report(&self) -> Call<RunReport> {
        unimplemented!()
    }

    /// Its Key-value store's reads, on every Run (ADR 0103).
    pub fn key_value_store(&self) -> KeyValueStoreReader {
        unimplemented!()
    }

    /// Frees the view; the Run goes on.
    pub fn close(&self) -> Call<()> {
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

    /// The outcomes, as [`CrawlRun::outcomes`]'s.
    pub fn outcomes(&self, filter: OutcomesFilter) -> EventStream<CrawlRequestOutcome> {
        let _ = filter;
        unimplemented!()
    }

    /// The Dataset's committed Records, as `CrawlRun::records`' (ADR 0103).
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

/// A Key-value store's reads, for a Run view.
#[derive(Debug, Clone)]
pub struct KeyValueStoreReader {
    _private: (),
}

impl KeyValueStoreReader {
    /// The value under the key, as `KeyValueStore::get`'s.
    pub fn get<T: DeserializeOwned + JsonSchema>(&self, key: &str) -> Call<Option<T>> {
        let _ = key;
        unimplemented!()
    }

    /// The value under the key, checked against a hand-written JSON Schema.
    pub fn get_with(&self, key: &str, schema: Schema<serde_json::Value>) -> Call<Option<serde_json::Value>> {
        let _ = (key, schema);
        unimplemented!()
    }

    /// The bytes under the key, or `None`.
    pub fn get_bytes(&self, key: &str) -> Call<Option<StoredBytes>> {
        let _ = key;
        unimplemented!()
    }

    /// The keys starting with `prefix`; `""` for all.
    pub fn keys(&self, prefix: &str) -> EventStream<String> {
        let _ = prefix;
        unimplemented!()
    }
}

/// An input of a resumed Run that differs from what the Run began with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunInputDifference {
    /// `product`, `kind`, `seeds`, `plan` or `caller`.
    pub input: String,
    /// What the Run began with.
    pub recorded: String,
    /// What this start gave.
    pub given: String,
}

/// What a Run's own Error codes carry (ADR 0102).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RunFailure {
    /// `run_id_invalid`: an ID the engine refuses.
    IdInvalid {
        /// The ID.
        id: String,
    },
    /// `run_conflict`: the ID names a Run started with other inputs, or by another caller.
    Conflict {
        /// The ID.
        id: String,
        /// Each difference.
        differences: Vec<RunInputDifference>,
    },
    /// `run_ended`: a call that would give an ended Run new work.
    Ended {
        /// The ID.
        id: String,
        /// How it ended.
        lifecycle: Lifecycle,
        /// When.
        ended_at: SystemTime,
    },
    /// `run_in_use`: the Run's writer's lock is held, by another program or this one (ADR 0031).
    /// Retryable: that program may end.
    InUse {
        /// The ID.
        id: String,
        /// Who holds it, rendered.
        owner: String,
    },
    /// `limit_raise_refused`: only a person raises a Limit (ADR 0041).
    LimitRaiseRefused {
        /// The Limit, by name.
        limit: String,
    },
    /// `run_interrupted`: a view's `report()` on a Run with no live writer that has not ended.
    Interrupted {
        /// The ID.
        id: String,
    },
    /// `run_not_found`: the ID names no Run in the Run index.
    NotFound {
        /// The ID.
        id: String,
    },
    /// `run_state_unavailable`: the Run's state is missing or unreadable.
    StateUnavailable {
        /// The ID.
        id: String,
        /// `missing` or `damaged`.
        state: String,
        /// Where it was expected.
        path: Option<PathBuf>,
    },
}
