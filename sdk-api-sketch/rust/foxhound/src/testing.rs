//! Foxhound's testing kit (ADRs 0069, 0105), behind `test-util`: the shared kit of
//! `goodfetch_collie_client::testing`, and a test engine whose connection is `Foxhound`, with
//! `run_handler` and `run_hook`.

pub use goodfetch_collie_client::testing::*;

use crate::Foxhound;
use crate::BlockCause;
use crate::Crawl;
use crate::CrawlRequestOutcome;
use crate::CrawlRequestOutcomeKind;
use crate::CrawlRequestRef;
use crate::Dataset;
use serde::de::DeserializeOwned;
use crate::Enqueued;
use crate::Error;
use crate::FetchTier;
use crate::Label;
use crate::ModelUsage;
use std::path::Path;
use crate::SkipReason;
use crate::StoredBytes;
use crate::UncertainWhy;

/// The document a handler runs on, given as `route.fulfill` gives a response, or read from a
/// saved file.
#[derive(Debug, Clone)]
pub struct SavedResponse {
    _private: (),
}

impl SavedResponse {
    /// A file on the program's machine, relative to the working directory, such as a page saved
    /// from a browser; its content type comes from its extension, else `text/html`.
    pub fn path(path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// The body itself.
    pub fn body(body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }

    /// Default 200.
    pub fn status(self, status: u16) -> Self {
        let _ = status;
        unimplemented!()
    }

    /// One response header.
    pub fn header(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }
}

/// What [`TestEngine::run_handler`] runs a handler on: one crawl request of the Label.
#[derive(Debug)]
pub struct HandlerInput<P> {
    _payload: std::marker::PhantomData<P>,
}

impl<P: serde::Serialize> HandlerInput<P> {
    /// The request's URL, the scope's seed, and the document's URL unless the response redirects,
    /// with its Payload (`()` for a Label without one).
    pub fn new(url: &str, payload: P) -> Self {
        let _ = (url, payload);
        unimplemented!()
    }

    /// The document. Left out: the response the recording holds for the URL.
    pub fn response(self, response: SavedResponse) -> Self {
        let _ = response;
        unimplemented!()
    }

    /// A Response recording replayed for everything else the handler reaches. Left out: none, so
    /// it reaches nothing.
    pub fn recording(self, path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// `Http` by default, or the tier the Label's `fetch.tier` forces. No browser starts on a
    /// non-browser tier: a call only a browser serves ends the attempt [`AttemptEnd::Escalated`]
    /// under `fetch.tier: auto`, and [`AttemptEnd::Failed`] with `browser_required` under a forced
    /// tier (ADR 0093). `Browser` renders the document, its `Date` starting at the document's `Date`
    /// header.
    pub fn tier(self, tier: FetchTier) -> Self {
        let _ = tier;
        unimplemented!()
    }

    /// Default 1, for a handler that acts on its attempt number.
    pub fn attempt(self, attempt: u32) -> Self {
        let _ = attempt;
        unimplemented!()
    }

    /// Default 0.
    pub fn depth(self, depth: u32) -> Self {
        let _ = depth;
        unimplemented!()
    }

    /// A prior answer the Effect journal holds for `key` (ADR 0106): a declaration of that key is
    /// answered as the journal would answer it, without running its body.
    pub fn effect(self, key: &str, prior: PriorEffect) -> Self {
        let _ = (key, prior);
        unimplemented!()
    }
}

/// A prior answer the Effect journal holds for a key in a test (ADR 0106).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PriorEffect {
    /// Done, with its output.
    Journal(serde_json::Value),
    /// A person answered "it happened" (ADR 0029).
    Reconciled,
    /// Left uncertain, for this reason.
    Uncertain(UncertainWhy),
}

/// How a declaration was answered in an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EffectAnswer {
    /// Its body ran and returned.
    Performed,
    /// The Effect journal answered it.
    Journal,
    /// A person's reconciliation answered it.
    Reconciled,
    /// It is uncertain.
    Uncertain,
    /// Its body ended before anything was sent.
    NotPerformed,
    /// A value a newer engine sends, kept as received (G3).
    Unknown(String),
}

/// One declaration an attempt made, and how it was answered.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DeclaredEffect {
    /// The key.
    pub key: String,
    /// How it was answered.
    pub answer: EffectAnswer,
    /// Whether its body sent anything; a command a Response recording answered counts as sent.
    pub sent: bool,
}

/// How an attempt ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum AttemptEnd {
    /// The handler returned: what it staged would commit.
    Returned,
    /// `ctx.skip(message)` (reason `handler`), or a handler's navigation that Site preferences
    /// refused and it let escape.
    Skipped {
        /// Why.
        reason: SkipReason,
        /// The handler's text.
        message: Option<String>,
    },
    /// `ctx.retry(message)`.
    RetryRequested {
        /// Its message.
        message: String,
    },
    /// A call the tier cannot serve; in a crawl, the request would run again in the browser.
    Escalated {
        /// That call, rendered as the program wrote it (ADR 0079).
        call: String,
    },
    /// The document's status is a Block cause (ADRs 0021, 0093): the handler did not run.
    Blocked(BlockCause),
    /// The handler failed: its own error as `handler_failed`, or the code a Goodfetch error keeps;
    /// or, before it ran, the document's status failed it (`response_not_ok`,
    /// `response_unavailable`).
    Failed(Error),
    /// A kind this SDK does not know, as received.
    Unknown(String),
}

/// An enqueue or a seed the attempt staged, with the answer it got.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StagedRequest {
    /// Its URL.
    pub url: String,
    /// Its Label's name.
    pub label: String,
    /// Its Payload, as JSON.
    pub payload: serde_json::Value,
    /// The answer it got.
    pub answer: Enqueued,
}

/// What a Key-value store write the attempt staged put under its key.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum StagedValue {
    /// A JSON value.
    Json(serde_json::Value),
    /// Bytes with their content type.
    Bytes(StoredBytes),
    /// A delete.
    Deleted,
}

/// A Key-value store write the attempt staged.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StagedWrite {
    /// The key.
    pub key: String,
    /// What it put there.
    pub value: StagedValue,
}

/// One attempt of a handler: how it ended and everything it staged, kept even when it ended early,
/// where a crawl would discard it (ADR 0003). Nothing is committed.
#[derive(Debug)]
#[non_exhaustive]
pub struct AttemptResult {
    /// How it ended.
    pub ended: AttemptEnd,
    /// The tier it ran on.
    pub tier: FetchTier,
    /// Its enqueues.
    pub enqueued: Vec<StagedRequest>,
    /// Its `ctx.run().add_seed()` calls.
    pub seeds: Vec<StagedRequest>,
    /// Its Key-value store writes.
    pub key_value_writes: Vec<StagedWrite>,
    /// The Effects it declared, in order (ADR 0106).
    pub effects: Vec<DeclaredEffect>,
    /// Model calls made by AI verbs in the handler (ADR 0101).
    pub usage: ModelUsage,
}

impl AttemptResult {
    /// The Records pushed to a Dataset, as `run.records()` would read them back.
    pub fn records<T: DeserializeOwned>(&self, dataset: &Dataset<T>) -> Result<Vec<T>, Error> {
        let _ = dataset;
        unimplemented!()
    }
}

/// What a Hook is run on (ADR 0104): the request, and what its stage sees: before a request, its
/// method, body, headers, tier and previous error; after one, the response; before a Record's
/// commit, the Record.
#[derive(Debug)]
pub struct HookInput {
    _private: (),
}

impl HookInput {
    /// A request of the Label, at this URL, with its Payload.
    pub fn new<P: serde::Serialize>(label: &Label<P>, url: &str, payload: P) -> Self {
        let _ = (label, url, payload);
        unimplemented!()
    }

    /// The method it was enqueued with, `GET` by default.
    pub fn method(self, method: &str) -> Self {
        let _ = method;
        unimplemented!()
    }

    /// The body it was enqueued with, none by default.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }

    /// The headers as built so far, the enqueue's own included.
    pub fn headers(self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        let _ = headers.into_iter();
        unimplemented!()
    }

    /// The tier about to fetch: `Http` by default, or the one the Label forces.
    pub fn tier(self, tier: FetchTier) -> Self {
        let _ = tier;
        unimplemented!()
    }

    /// The error of the attempt before, for a Hook's retry branch. Left out: none.
    pub fn previous_error(self, error: Error) -> Self {
        let _ = error;
        unimplemented!()
    }

    /// The attempt number, 1 by default.
    pub fn attempt(self, attempt: u32) -> Self {
        let _ = attempt;
        unimplemented!()
    }

    /// The request's depth, 0 by default.
    pub fn depth(self, depth: u32) -> Self {
        let _ = depth;
        unimplemented!()
    }

    /// The response or document the stage sees.
    pub fn response(self, response: SavedResponse) -> Self {
        let _ = response;
        unimplemented!()
    }

    /// A Response recording replayed for anything else the Hook reaches.
    pub fn recording(self, path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// The Record a Hook before a Record's commit sees, in its Dataset.
    pub fn record<T: serde::Serialize>(self, dataset: &Dataset<T>, record: T) -> Self {
        let _ = (dataset, record);
        unimplemented!()
    }
}

/// What a Hook did.
#[derive(Debug)]
#[non_exhaustive]
pub enum HookResult {
    /// Its answer, as JSON; `Null` when it changed nothing.
    Answered(serde_json::Value),
    /// The call that ended the work (`skip`, `block`, `retry`, `drop`, `reject`), with its message.
    Ended {
        /// The call's name.
        call: String,
        /// Its message or cause.
        message: String,
    },
    /// It failed: `hook_failed`, or the code a Goodfetch error keeps.
    Failed(Error),
    /// A kind this SDK does not know, as received.
    Unknown(String),
}

impl CrawlRequestRef {
    /// A request reference for a program's own tests.
    pub fn fake(url: &str, label: &str) -> Self {
        let _ = (url, label);
        unimplemented!()
    }
}

impl CrawlRequestOutcome {
    /// An outcome for a program's own tests, ended now after `attempts` attempts.
    pub fn fake(request: CrawlRequestRef, kind: CrawlRequestOutcomeKind, attempts: u32) -> Self {
        let _ = (request, kind, attempts);
        unimplemented!()
    }
}


/// A real engine for one test, linked in-process: a temporary Goodfetch home removed when it
/// closes, a cleared environment, no project file unless named, `offline` (ADR 0043), short
/// durations, and no Model provider unless the test registers or configures one, so a call that
/// needs a model with none fails with `model_not_configured`. Every Run started from
/// [`TestEngine::connection`] that names no Response recording replays an empty one: a request to a
/// site fails with `response_not_recorded`, naming it. Browsers and the Product's executable come
/// from the browser cache of the caller's Goodfetch home, read and never written, or
/// `GOODFETCH_BROWSERS_PATH` and the Product's engine-path variable (ADRs 0024, 0059, 0090, 0105).
#[derive(Debug)]
pub struct TestEngine {
    _private: (),
}

impl TestEngine {
    /// Starts it.
    pub async fn start(options: TestOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// A connection to this engine, [`Foxhound::connect`](crate::Foxhound::connect)'s type: everything a program does works
    /// on it.
    pub fn connection(&self) -> &Foxhound {
        unimplemented!()
    }

    /// Runs the Label's handler, the one the crawl attached, on one request, with the Label's
    /// settings, in a Run of its own: one attempt, no retry. Nothing else of the crawl runs: no
    /// Frontier, no Hooks, no commit. The document is not fetched, so neither robots.txt nor
    /// Politeness applies to it, but ADR 0093's status rules do, before the handler runs. Fails when
    /// it cannot run the handler (`response_not_recorded` for the document,
    /// `payload_invalid`, `label_unknown`, `label_unhandled`, `setting_invalid` for a tier other than
    /// a forced one); once the handler ran, how its attempt ended is a value.
    pub async fn run_handler<P: serde::Serialize>(&self, crawl: &Crawl<'_>, label: &Label<P>, input: HandlerInput<P>) -> Result<AttemptResult, Error> {
        let _ = (crawl, label, input);
        unimplemented!()
    }

    /// Runs one of the crawl's Hooks, by the name Hooks carry (ADR 0104), alone, its context built from
    /// `input` as `run_handler` builds a handler's. A name the crawl does not hold fails with
    /// `hook_unknown`, naming the closest.
    pub async fn run_hook(&self, crawl: &Crawl<'_>, name: &str, input: HookInput) -> Result<HookResult, Error> {
        let _ = (crawl, name, input);
        unimplemented!()
    }

    /// Stands for a person's answer about an uncertain Effect of a Run on this engine (ADR 0029).
    /// Only the testing kit has it (ADR 0106).
    pub async fn reconcile(&self, run_id: &str, key: &str, answer: Reconciliation) -> Result<(), Error> {
        let _ = (run_id, key, answer);
        unimplemented!()
    }

    /// Closes the connection and the engine and removes the home. Fails with
    /// `scripted_model_unused` if a Scripted model registered here has replies left. Dropping it
    /// unclosed removes the home too, without that check.
    pub async fn close(self) -> Result<(), Error> {
        unimplemented!()
    }
}

