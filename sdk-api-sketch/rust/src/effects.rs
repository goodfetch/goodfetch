//! Effects a program declares in code (ADR 0106, decided in #500).
//!
//! A program's own calls are Effects only when it declares them (ADR 0010):
//! `run.effect(key, body)` wraps a closure of its own, and the value the body returns is kept in
//! the Run's Effect journal. In a crawl handler it is `ctx.run().effect(…)`, whose key's namespace
//! is the crawl request, so every retry, escalation, sample comparison, lost lease and
//! `retry_failed` reaches the same entry; elsewhere it is the Run, so a Collie program started
//! again with its Run's ID finds what it already did.
//!
//! Goodfetch judges a body by what it sent for it while the body ran: a body that returned is
//! done and never runs again; one that ended before anything was sent was not performed; one
//! that ended after a send is uncertain (`effect_uncertain`) until a person reconciles it
//! (ADR 0029), unless it is repeatable. A future dropped mid-body is judged when its key is next
//! declared, or when its attempt or Run ends. There is no separate check: a body that checks
//! first, or sends [`Effect::idempotency_key`], is repeatable.

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::SystemTime;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::crawl::CrawlRun;
use crate::lifecycle::Run;
use crate::results::{CrawlRequestRef, Error};

/// The Effect a body performs, handed to the body.
#[derive(Debug)]
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

/// Declaring an Effect, not sent until awaited.
#[must_use = "an Effect is not declared until this is awaited"]
pub struct DeclareEffect<'a, T, F> {
    _run: &'a Run,
    _body: F,
    _output: PhantomData<fn() -> T>,
}

impl<T, F> std::fmt::Debug for DeclareEffect<'_, T, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclareEffect").finish_non_exhaustive()
    }
}

impl<T, F> DeclareEffect<'_, T, F> {
    /// The body is safe to run again: one that ended without returning after it sent something
    /// runs again at the next declaration, instead of being reported uncertain. A done Effect never
    /// runs again. Built-in: not repeatable (ADR 0010).
    pub fn repeatable(self) -> Self {
        unimplemented!()
    }
}

impl<'a, T, F> IntoFuture for DeclareEffect<'a, T, F>
where
    T: Serialize + DeserializeOwned + 'a,
    F: AsyncFnOnce(&Effect) -> Result<T, Error> + 'a,
{
    type Output = Result<EffectResult<T>, Error>;
    /// Runs the body in the awaiting task: it may borrow and needs no `Send` (ADR 0081).
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

impl Run {
    /// Performs `body` as the Effect `key`, unless the Run's Effect journal already answers it.
    ///
    /// The key, at most 255 characters, names one Effect in the Run. A body that fails before
    /// Goodfetch sent anything returns its own error unchanged; one that fails after a send fails
    /// with `effect_uncertain`, its source the original, unless [`DeclareEffect::repeatable`]. The
    /// program's own I/O inside the body is invisible to the journal.
    pub fn effect<'a, T, F>(&'a self, key: &str, body: F) -> DeclareEffect<'a, T, F>
    where
        T: Serialize + DeserializeOwned + 'a,
        F: AsyncFnOnce(&Effect) -> Result<T, Error> + 'a,
    {
        let _ = (key, body);
        unimplemented!()
    }
}

impl CrawlRun {
    /// As [`Run::effect`]. Through a handler's `ctx.run()`, bound to its attempt, the key's
    /// namespace is the crawl request; through the program's own value, the Run.
    pub fn effect<'a, T, F>(&'a self, key: &str, body: F) -> DeclareEffect<'a, T, F>
    where
        T: Serialize + DeserializeOwned + 'a,
        F: AsyncFnOnce(&Effect) -> Result<T, Error> + 'a,
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
    /// The crawl request whose namespace holds it; `None` outside a handler.
    pub request: Option<CrawlRequestRef>,
    /// Why it is uncertain.
    pub why: UncertainWhy,
    /// The first command sent for it, in Rust; `None` when the journal lost it.
    pub first_sent: Option<String>,
    /// Since when.
    pub since: SystemTime,
}

/// Details of `effect_key_invalid`: an empty key, or one longer than 255 characters.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectKeyInvalid {
    /// The key given.
    pub key: String,
}

/// Details of `effect_key_reused`: a key declared again after this attempt already got its answer.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectKeyReused {
    /// The key.
    pub key: String,
    /// The answer it got: `performed`, `journal` or `reconciled`.
    pub source: String,
}

/// Details of `effect_output_invalid`: the body's value is not JSON or is larger than
/// `effect.output.max_size`; judged as a body that did not return.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EffectOutputInvalid {
    /// The key.
    pub key: String,
    /// `not_json` or `too_large`.
    pub problem: String,
    /// Its size in bytes, when it could be serialized.
    pub size: Option<u64>,
}
