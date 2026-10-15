//! The Run in code (ADR 0102, decided in #478).
//!
//! - A Run starts with [`crate::Collie::start`] or [`crate::Foxhound::start_crawl`], which return once
//!   it is started. Its Run ID, chosen with `.run_id(..)` or generated, names it for good: a start
//!   whose Run ID names an existing Run resumes it, returns it ended ([`StartedAs`]), or refuses,
//!   and nothing is inferred from a storage folder.
//! - [`Run::finish`] lets the Run finish and returns its [`RunReport`]; [`Run::stop`] ends it early
//!   with its report; ADR 0078's `close()` is unchanged. A crawl's [`crate::Crawl::serve`] returns its
//!   [`CrawlRunReport`] once the Run ends, since serving is what lets it finish.
//! - Progress is a telemetry [`EventStream`] (ADRs 0024, 0081); a crawl's outcomes are read from its
//!   committed state, so a slow reader never slows the Run.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::concurrency::EventStream;
use crate::data::Schema;
use crate::lifecycle::Run;
use crate::output::StoredBytes;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use crate::models::RunUsage;
use crate::results::{LimitSource, Verdict};
use crate::time::Call;

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
    /// A Limit, with its scope and the scope's ID, and when its period resets.
    Limit {
        /// The Limit, by name.
        name: String,
        /// Where it attaches.
        scope: crate::models::LimitScope,
        /// The scope's ID.
        scope_id: String,
        /// `None` for a Limit with no period.
        resets_at: Option<SystemTime>,
    },
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

/// A Human pause waiting for a person (ADR 0092). It begins and ends as a change of `progress()`;
/// the person learns of it from ADR 0027's Notification and the Console's "Needs you". Its time
/// counts toward no timeout, the Run's included, while its Usage counts toward Limits.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WaitingPause {
    /// The Session whose work waits.
    pub session: String,
    /// The page it holds, by the page's ID, when it holds one.
    pub page: Option<String>,
    /// The program's reason, or the engine's own.
    pub reason: Option<String>,
    /// The `until` condition as text, the Locator's or the URL matcher's description; `None`: a
    /// person's "Done" ends it.
    pub until: Option<String>,
    /// Who raised it.
    pub raised_by: PauseRaisedBy,
    /// For an engine's pause, the step that raised it: `login_recovery` (ADR 0006) or `challenge`
    /// (ADR 0035's Challenge ladder).
    pub step: Option<String>,
    /// Since when.
    pub since: SystemTime,
    /// When it runs out: the first of `timeout.human_pause` and, in a scheduled run, ADR 0036's
    /// pause timeout; `None` when it waits without limit.
    pub deadline: Option<SystemTime>,
}

/// A Run's progress: a telemetry value, coalesced under load (ADR 0024).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunProgress {
    /// Where the Run is.
    pub lifecycle: Lifecycle,
    /// False when nothing works the Run, as after a crash, until a start with its ID resumes it.
    pub has_writer: bool,
    /// The run epoch (ADR 0003), which a resume or a migration to another machine advances.
    pub epoch: u64,
    /// When the last Checkpoint was written.
    pub checkpoint_at: Option<SystemTime>,
    /// The Human pauses waiting.
    pub pauses: Vec<WaitingPause>,
    /// The program's own line, set with [`Run::set_progress_message`] (ADR 0037's RT-15).
    pub message: Option<String>,
    /// When the engine took this value.
    pub at: SystemTime,
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
    /// A crawl's Run only: no seed reached a handler, each refused when added or skipped when
    /// fetched, counted by their wire names, since enqueue and skip reasons are Foxhound's types.
    NoSeedHandled {
        /// The seeds refused when added, by an enqueue's reason.
        refused: Vec<(String, u64)>,
        /// The seeds skipped when fetched, by a skip reason.
        skipped: Vec<(String, u64)>,
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
    /// The Run's Verdict (ADR 0029), reading every check it relies on, each AI task's Success
    /// check included.
    pub verdict: Verdict,
    /// Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects.
    pub verdict_reasons: Vec<VerdictReason>,
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
    /// Each Agent task is one entry (ADR 0100).
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

impl Run {
    /// How the start opened this Run.
    pub fn started_as(&self) -> &StartedAs {
        unimplemented!()
    }

    /// Lets the Run finish, ends it `finished`, closes its Sessions and pages, and returns its
    /// report: at once for a Collie Run, whose program says it is done; when the engine ends it for
    /// a detached Run. If something stops the Run first (a Limit, its timeout, a person, the
    /// engine), the report says `Stopped`. Its wait has no timeout of its own: the Run's own bounds
    /// it, and `.timeout(..)` or dropping the future ends only the wait. A failed write of the Plan
    /// file or the Response recording fails with `path_not_writable` or `file_write_failed` once the
    /// Run has ended; `open_run(run_id).report()` reads its report, which keeps a warning naming the code
    /// (ADR 0116).
    pub fn finish(&self) -> Call<RunReport> {
        unimplemented!()
    }

    /// Stops the Run at its next safe point with a Checkpoint, `stopped` and `incomplete` as an
    /// abort by its program, and returns its report. Resumable by its ID. Its wait is bounded by
    /// `timeout.close`, as `close()`'s is, or by `.timeout(..)`; past it, it fails with
    /// `deadline_exceeded` naming the setting, and the Run still stops at its next safe point.
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

    /// Writes a Checkpoint now (ADR 0002), where the engine keeps the Run's. Resolves only once its
    /// file is whole; a write the engine cannot make fails with `file_write_failed` on the engine's
    /// side (ADR 0116).
    pub fn checkpoint(&self) -> Call<Checkpoint> {
        unimplemented!()
    }

    /// The Run trace recorded so far; `None` when the Run keeps none, which the report's
    /// `weakened` shows.
    pub fn trace(&self) -> Call<Option<RunTrace>> {
        unimplemented!()
    }
}

/// Why a Run is not `verified`, naming the AI task concerned, an Agent's included, by its
/// `call_id` (ADR 0100).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VerdictReason {
    /// Such as `success_check_failed`, `no_success_check`, `check_switched_off` or
    /// `effect_uncertain`; a newer engine's is kept (G3).
    pub code: String,
    /// Its message.
    pub message: String,
    /// The AI task concerned.
    pub call_id: Option<String>,
}

/// The Run trace (ADR 0031), copied to this machine as an Export is (ADR 0103): what was recorded
/// so far, in the format ADR 0031 exports it in. A person replays it in the Console at
/// `console_url`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunTrace {
    /// In bytes.
    pub size: u64,
}

impl RunTrace {
    /// Copies it to a file on this machine, wherever the engine runs, whole or not at all: a failed
    /// write fails with `path_not_writable` or `file_write_failed`, leaving the path as it was
    /// (ADR 0116).
    pub fn save_as(&self, path: impl AsRef<Path>) -> Call<()> {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// Its bytes, streamed to this machine.
    pub fn stream(&self) -> EventStream<Vec<u8>> {
        unimplemented!()
    }
}

impl Checkpoint {
    /// Copies it to a file on this machine, wherever the engine runs, whole or not at all: a failed
    /// write fails with `path_not_writable` or `file_write_failed`, leaving the path as it was
    /// (ADR 0116).
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

    /// The Run trace recorded so far; `None` when the Run keeps none.
    pub fn trace(&self) -> Call<Option<RunTrace>> {
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
    /// `product`, `kind`, `seeds`, `plan`, `recording` (ADR 0105) or `caller`.
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
