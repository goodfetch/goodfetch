//! The testing kit, `goodfetch::testing` behind the `test-util` feature (ADRs 0069 and 0105,
//! decided in #482): public and under semver, for a program's own tests. Each Product ships its own
//! (ADRs 0090, 0095); crawl handlers and Hooks are Foxhound's (ADR 0084).
//!
//! It fits any test harness: `#[test]` with `goodfetch::blocking`, `#[tokio::test]` or another
//! executor, since its futures run on any (ADR 0081). Assertions are the harness's own.
//!
//! - [`TestEngine`]: ADR 0069's test engine, the engine linked in-process and set up for one test.
//!   Nothing it does leaves the machine unless a Run records: every Run that names no Response
//!   recording replays an empty one.
//! - [`TestEngine::run_handler`]: one crawl handler on one saved response, reporting how its attempt
//!   ended and what it staged, without a Frontier, Hooks or a commit. [`TestEngine::run_hook`]: one
//!   Hook, the same way.
//! - [`ScriptedModel`]: ADR 0069's Scripted model, answered in-process by the linked engine.
//! - [`Error::fake`] and a `fake` constructor on each `#[non_exhaustive]` result type, which a
//!   program cannot build with a struct literal, for a test of the program's own handling.

use std::path::Path;

use serde::de::DeserializeOwned;

use crate::ai::{StepReport, StepResult, StepSource};
use crate::crawl::{Crawl, Enqueued, FetchTier};
use crate::data::{Dataset, Label};
use crate::lifecycle::Goodfetch;
use crate::models::{ModelRequest, ModelResponse, ModelUsage};
use crate::output::StoredBytes;
use crate::results::{CrawlRequestOutcome, CrawlRequestOutcomeKind, CrawlRequestRef, Error};
use crate::settings::Settings;

/// Options of starting a [`TestEngine`].
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct TestOptions {
    _private: (),
}

impl TestOptions {
    /// Settings for its connection and everything beneath it (ADR 0083).
    pub fn settings(self, settings: Settings) -> Self {
        let _ = settings;
        unimplemented!()
    }

    /// One entry of ADR 0059's `resolve` map, such as `("shop.test", "127.0.0.1:8080")` for the
    /// test's own server. Its host is allowlisted for this engine (ADR 0017), so it is reached live
    /// even while a Run replays.
    pub fn resolve(self, host: &str, address: &str) -> Self {
        let _ = (host, address);
        unimplemented!()
    }

    /// The project configuration file. Left out: none, as the home and the environment are the
    /// test's own.
    pub fn project(self, path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// A Scripted model, named by `ai.model` as `"<name>/<any>"` (ADR 0101).
    pub fn model_provider(self, name: &str, model: ScriptedModel) -> Self {
        let _ = (name, model);
        unimplemented!()
    }
}

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

    /// `Http` by default, or the tier the Label's `fetch.tier` forces: no browser starts, and a call
    /// only a browser serves ends the attempt [`AttemptEnd::Escalated`]. `Browser` renders the
    /// document.
    pub fn tier(self, tier: FetchTier) -> Self {
        let _ = tier;
        unimplemented!()
    }

    /// Default 1, for a handler that acts on its attempt number.
    pub fn attempt(self, attempt: u32) -> Self {
        let _ = attempt;
        unimplemented!()
    }
}

/// How an attempt ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum AttemptEnd {
    /// The handler returned: what it staged would commit.
    Returned,
    /// `ctx.skip(message)`.
    Skipped {
        /// Its message.
        message: String,
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
    /// The handler failed: its own error as `handler_failed`, or the code a Goodfetch error keeps.
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

/// What a Hook is run on: the request, and the response, document or Record its stage sees. The
/// stages' contexts are #481's.
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

    /// The response or document the stage sees.
    pub fn response(self, response: SavedResponse) -> Self {
        let _ = response;
        unimplemented!()
    }

    /// The Record a "Record before commit" Hook sees, in its Dataset.
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

/// A real engine for one test, linked in-process: a temporary Goodfetch home removed when it
/// closes, a cleared environment, no project file unless named, `offline` (ADR 0043), short
/// durations, and no Model provider unless the test registers or configures one, so a call that
/// needs a model with none fails with `model_not_configured`. Every Run started from
/// [`TestEngine::connection`] that names no Response recording replays an empty one: a request off
/// the machine fails with `response_not_recorded`, naming it.
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

    /// A connection to this engine, [`Goodfetch::connect`]'s type: everything a program does works
    /// on it.
    pub fn connection(&self) -> &Goodfetch {
        unimplemented!()
    }

    /// Runs the Label's handler, the one the crawl attached, on one request, with the Label's
    /// settings, in a Run of its own. Nothing else of the crawl runs: no Frontier, no Hooks, no
    /// commit. Fails when it cannot run the handler (`response_not_recorded` for the document,
    /// `payload_invalid`, `label_unknown`, `label_unhandled`); once the handler ran, how its attempt
    /// ended is a value.
    pub async fn run_handler<P: serde::Serialize>(&self, crawl: &Crawl<'_>, label: &Label<P>, input: HandlerInput<P>) -> Result<AttemptResult, Error> {
        let _ = (crawl, label, input);
        unimplemented!()
    }

    /// Runs one of the crawl's Hooks, by the name Hooks carry (#481), alone, its context built from
    /// `input` as `run_handler` builds a handler's. A name the crawl does not hold fails with
    /// `hook_unknown`, naming the closest.
    pub async fn run_hook(&self, crawl: &Crawl<'_>, name: &str, input: HookInput) -> Result<HookResult, Error> {
        let _ = (crawl, name, input);
        unimplemented!()
    }

    /// Closes the connection and the engine and removes the home. Fails with
    /// `scripted_model_unused` if a Scripted model registered here has replies left. Dropping it
    /// unclosed removes the home too, without that check.
    pub async fn close(self) -> Result<(), Error> {
        unimplemented!()
    }
}

/// One reply of a Scripted model.
#[derive(Debug)]
#[non_exhaustive]
pub enum ScriptedReply {
    /// A response.
    Response(ModelResponse),
    /// An error the provider would give, such as `Error::fake(ErrorCode::ModelUnavailable, ..)`.
    Error(Error),
}

/// ADR 0069's Scripted model: answers call n with reply n, records every request, and reports
/// Usage from its replies. A call beyond its replies fails with `scripted_model_exhausted`. A cheap
/// clone: the test keeps one to read its requests.
#[derive(Debug, Clone)]
pub struct ScriptedModel {
    _private: (),
}

impl ScriptedModel {
    /// Its replies, in order.
    pub fn new(replies: impl IntoIterator<Item = ScriptedReply>) -> Self {
        let _ = replies.into_iter();
        unimplemented!()
    }

    /// Every request it answered, in order.
    pub fn requests(&self) -> Vec<ModelRequest> {
        unimplemented!()
    }

    /// Replies not used yet.
    pub fn remaining(&self) -> usize {
        unimplemented!()
    }
}

// `fake` constructors: a program cannot build a `#[non_exhaustive]` value with a struct literal.

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

impl StepReport {
    /// A step report for a program's own tests.
    pub fn fake(source: StepSource, instruction: &str) -> Self {
        let _ = (source, instruction);
        unimplemented!()
    }
}

impl<T> StepResult<T> {
    /// An AI verb's result for a program's own tests.
    pub fn fake(output: T, step: StepReport, usage: ModelUsage) -> Self {
        let _ = (output, step, usage);
        unimplemented!()
    }
}
