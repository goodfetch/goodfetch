//! Effects a program declares in code (ADR 0106, decided in #500).
//!
//! A program's own calls are Effects only when it declares them (ADR 0010):
//! `run.effect(key, body)` wraps a closure of its own, and the value the body returns is kept in
//! the Run's Effect journal. In a crawl handler it is `ctx.run().effect(…)`, whose key's namespace
//! is the crawl request, so every retry, escalation, sample comparison, lost lease and
//! `retry_failed` reaches the same entry; elsewhere it is the Run, so a Collie program started
//! again with its Run's ID finds what it already did.
//!
//! The engine judges a body by what it sent for it while the body ran: a body that returned is
//! done and never runs again; one that ended before anything was sent was not performed; one
//! that ended after a send is uncertain (`effect_uncertain`) until a person reconciles it
//! (ADR 0029), unless it is repeatable. A future dropped mid-body, or a body cut by the end of
//! its attempt, is judged when its key is next declared, or when its attempt or Run ends. A body
//! counts as sent from its start while the engine cannot see every command (a raw CDP attach, a
//! lent page, a browser shared with other code). Prepare the page before the declaration: the body
//! holds the action that commits the Effect and the reads of its result. There is no separate
//! check: a body that checks first, or sends [`Effect::idempotency_key`], is repeatable.
//!
//! The body is generic in its error `E`, which takes the SDK's [`Error`] (`From<Error>`) and
//! boxes into a `std::error::Error` source: [`Error`] itself or `anyhow::Error` fit. Its own error
//! returns unchanged when nothing was sent, and is the `source()` of `effect_uncertain` after a send.

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::SystemTime;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::lifecycle::Run;
use crate::results::Error;

/// The Effect a body performs, handed to the body by value, a cheap clone, so the body's future
/// owns it and can be `Send` (ADR 0112).
#[derive(Debug, Clone)]
pub struct Effect {
    _private: (),
}

impl Effect {
    /// The key it was declared with.
    pub fn key(&self) -> &str {
        unimplemented!()
    }

    /// An opaque key, the same on every attempt and resume of this Effect, on any machine, and
    /// different in every Run and every install. Send it as a site's `Idempotency-Key` header, or
    /// to the program's own service, to make the body safe to repeat. It reveals neither the Run's
    /// ID nor the key.
    pub fn idempotency_key(&self) -> &str {
        unimplemented!()
    }
}

/// What declaring an Effect returns.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EffectResult<T> {
    /// The body ran now.
    Performed {
        /// What it returned.
        output: T,
        /// When.
        performed_at: SystemTime,
    },
    /// The Effect journal already held it from an earlier attempt or start: the body did not run.
    Journal {
        /// What the body returned then.
        output: T,
        /// When it was performed.
        performed_at: SystemTime,
    },
    /// A person answered "it happened" (ADR 0029): the body did not run and no output is known.
    Reconciled {
        /// Who answered.
        by: String,
        /// When.
        at: SystemTime,
    },
    /// A source a newer engine sends, kept as received (ADR 0079).
    Unknown(String),
}

impl<T> EffectResult<T> {
    /// The body's output, now or from the journal; `None` when it is not known.
    pub fn output(&self) -> Option<&T> {
        unimplemented!()
    }

    /// The body's output, by value; `None` when it is not known.
    pub fn into_output(self) -> Option<T> {
        unimplemented!()
    }
}

/// The error a body may return: anything that takes the SDK's [`Error`] and boxes into a
/// source, such as [`Error`] itself or `anyhow::Error`.
pub trait BodyError: From<Error> + Into<Box<dyn std::error::Error + Send + Sync>> {}

impl<E: From<Error> + Into<Box<dyn std::error::Error + Send + Sync>>> BodyError for E {}

/// Declaring an Effect, not sent until awaited.
#[must_use = "an Effect is not declared until this is awaited"]
pub struct DeclareEffect<'a, T, E, F> {
    _run: &'a Run,
    _body: F,
    _output: PhantomData<fn() -> Result<T, E>>,
}

impl<T, E, F> std::fmt::Debug for DeclareEffect<'_, T, E, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclareEffect").finish_non_exhaustive()
    }
}

impl<T, E, F> DeclareEffect<'_, T, E, F> {
    /// The body is safe to run again: one that ended without returning after it sent something
    /// runs again at the next declaration, instead of being reported uncertain. A done Effect never
    /// runs again. Built-in: not repeatable (ADR 0010).
    pub fn repeatable(self) -> Self {
        unimplemented!()
    }
}

impl<'a, T, E, F, Fut> IntoFuture for DeclareEffect<'a, T, E, F>
where
    T: Serialize + DeserializeOwned + Send + 'a,
    E: BodyError + Send + 'a,
    F: FnOnce(Effect) -> Fut + Send + 'a,
    Fut: Future<Output = Result<T, E>> + Send + 'a,
{
    /// The body's own error unchanged, or an SDK error converted into `E`.
    type Output = Result<EffectResult<T>, E>;
    /// Runs the body in the awaiting task: it may borrow (ADR 0081), and it is `Send`, so a
    /// handler or a multi-threaded server's future that declares an Effect stays `Send` (ADR 0112).
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

impl Run {
    /// Performs `body` as the Effect `key`, unless the Run's Effect journal already answers it.
    ///
    /// The key, at most 255 code points, names one Effect in the Run; declaring it again while its
    /// body runs, or after this attempt got its answer, fails with `effect_key_reused`, except that
    /// on a borrowed browser (ADR 0089) a key declared again after its answer returns that answer,
    /// since the other crawler's retries reach the Run as new calls. While the body runs in an
    /// attempt, staging an output with it (`enqueue`, `add_seed`, `ctx.push`, a Key-value
    /// store write) fails with
    /// `effect_staging_refused`: stage it after the declaration, from the output. A body that
    /// fails before the engine sent anything returns its own error unchanged; one that fails after a
    /// send fails with `effect_uncertain`, its source the original, unless
    /// [`DeclareEffect::repeatable`]. The program's own I/O inside the body is invisible to the
    /// journal. No timeout of its own: the body's calls carry theirs (ADR 0080).
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

/// Why an Effect is uncertain.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UncertainWhy {
    /// Its body failed after something was sent.
    FailedAfterSend,
    /// An earlier attempt ended inside it after a send.
    Interrupted,
    /// The Run resumed from a Checkpoint after which it may have gone on, and the journal lacks
    /// the key (ADR 0002).
    JournalIncomplete,
    /// A reason a newer engine sends, kept as received (G3).
    Unknown(String),
}

/// An Effect the journal holds as uncertain, until a person reconciles it (ADR 0029); the details
/// of `effect_uncertain`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct UncertainEffect {
    /// The Effect journal's entry.
    pub journal_id: String,
    /// A declared Effect's key; `None` for an AI action.
    pub key: Option<String>,
    /// An AI action's instruction (ADR 0091); `None` for a declared Effect.
    pub instruction: Option<String>,
    /// Why it is uncertain.
    pub why: UncertainWhy,
    /// The first command sent for it, in Rust; `None` when the journal lost it.
    pub first_sent: Option<String>,
    /// Since when.
    pub since: SystemTime,
}

/// Details of `effect_key_invalid`: an empty key, or one longer than 255 code points.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectKeyInvalid {
    /// The key given.
    pub key: String,
}

/// The answer a key already got in this attempt, or that its body still runs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyAnswer {
    /// Its body ran in this attempt.
    Performed,
    /// The Effect journal answered it in this attempt.
    Journal,
    /// A person's reconciliation answered it in this attempt.
    Reconciled,
    /// Its body is still running in this attempt.
    Running,
    /// A value a newer engine sends, kept as received (G3).
    Unknown(String),
}

/// Details of `effect_key_reused`: a key declared again while its body runs or after this attempt
/// got its answer.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectKeyReused {
    /// The key.
    pub key: String,
    /// What the key already has in this attempt.
    pub answer: KeyAnswer,
}

/// Why a body's value cannot be kept.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputProblem {
    /// It does not serialize to JSON.
    NotJson,
    /// It is larger than `effect.output.max_size`.
    TooLarge,
    /// A value a newer engine sends, kept as received (G3).
    Unknown(String),
}

/// What a call tried to stage while a body ran.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StagedOutput {
    /// An enqueue.
    Enqueue,
    /// A seed added through `ctx.run()`.
    Seed,
    /// A pushed Record.
    Record,
    /// A Key-value store write through `ctx.run()` (ADR 0103).
    KeyValue,
    /// A value a newer engine sends, kept as received (G3).
    Unknown(String),
}

/// Details of `effect_staging_refused`: a call staged an output with an attempt while a declared
/// Effect's body ran in it. A re-run that gets `journal` does not run the body again, so the output
/// would be lost when the attempt ends early.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectStagingRefused {
    /// The running body's key.
    pub key: String,
    /// What the call tried to stage.
    pub staged: StagedOutput,
}

/// Details of `effect_output_invalid`: the body's value is not JSON or is larger than
/// `effect.output.max_size`. Returned as such when nothing was sent, and as the source of
/// `effect_uncertain` after a send.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectOutputInvalid {
    /// The key.
    pub key: String,
    /// What is wrong with it.
    pub problem: OutputProblem,
    /// Its size in bytes, when it could be serialized.
    pub size: Option<u64>,
}
