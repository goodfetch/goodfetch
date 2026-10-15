//! The testing kit, `goodfetch_collie_client::testing`, shared by each Product's kit, behind the `test-util` feature (ADRs 0069 and 0105,
//! decided in #482): public and under semver, for a program's own tests. Each Product ships its own
//! (ADRs 0090, 0095); crawl handlers and Hooks are Foxhound's (ADR 0084).
//!
//! It fits any test harness: `#[test]` with `goodfetch_collie::blocking`, `#[tokio::test]` or another
//! executor, since its futures run on any (ADR 0081). Assertions are the harness's own.
//!
//! - [`TestEngine`]: ADR 0069's test engine, the engine linked in-process and set up for one test.
//!   It reaches no site unless a Run records: every Run that names no Response recording replays an
//!   empty one.
//! - [`TestEngine::run_handler`]: one crawl handler on one saved response, reporting how its attempt
//!   ended and what it staged, without a Frontier, Hooks or a commit. [`TestEngine::run_hook`]: one
//!   Hook, the same way.
//! - [`ScriptedModel`]: ADR 0069's Scripted model, whose `model_requests` stream (ADR 0101) the kit
//!   answers on the connection's own runtime, never on the program's executor (ADR 0081), so the
//!   SDK-handler path is the one tested.
//! - [`Error::fake`] and a `fake` constructor on each `#[non_exhaustive]` result type, which a
//!   program cannot build with a struct literal, for a test of the program's own handling.

use std::path::Path;


use crate::ai::{StepReport, StepResult, StepSource};
use crate::models::{ModelRequest, ModelResponse, ModelUsage};
use crate::results::Error;
use crate::settings::Settings;

/// Options of starting a [`TestEngine`].
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct TestOptions {
    _private: (),
}

impl TestOptions {
    /// Settings for its connection and everything beneath it (ADR 0083). Every host its `resolve`
    /// setting (ADR 0059) maps to loopback, such as a test's own server, is allowlisted (ADR 0017)
    /// and reached live even while a Run replays.
    pub fn settings(self, settings: impl Into<Settings>) -> Self {
        let _ = settings.into();
        unimplemented!()
    }

    /// The environment map, as `connect`'s (ADRs 0062, 0083): the process's own unless replaced
    /// here. Only `GOODFETCH_HOME` (to read its browser cache, never to write),
    /// `GOODFETCH_BROWSERS_PATH` and this Product's engine-path variable are read from it; the
    /// engine's own environment stays cleared.
    pub fn env(self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        let _ = env.into_iter();
        unimplemented!()
    }

    /// The project configuration file. Left out: none, as the home and the environment are the
    /// test's own.
    pub fn project(self, path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// A Scripted model, named by `ai.model` as `"<name>/<any>"` (ADR 0101). The kit answers its
    /// stream on the connection's own runtime.
    pub fn model_provider(self, name: &str, model: ScriptedModel) -> Self {
        let _ = (name, model);
        unimplemented!()
    }
}

/// A person's answer about an uncertain Effect (ADR 0029), given in a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reconciliation {
    /// It happened: the next declaration returns `reconciled`.
    Happened,
    /// It did not happen: the next declaration performs the body again.
    DidNotHappen,
}

/// One reply of a Scripted model.
#[derive(Debug)]
#[non_exhaustive]
pub enum ScriptedReply {
    /// A response.
    Response(ModelResponse),
    /// An error the provider would give, such as `Error::fake(ErrorCode::ModelFundsExhausted, ..)`.
    /// A retryable one is retried as a provider's (ADR 0101), the retry taking the next reply.
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
