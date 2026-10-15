//! Time, waits, deadlines, cancellation and Limits (ADR 0080, decided in #467).
//!
//! Every duration is a [`std::time::Duration`]. A call is a builder: set its timeout with
//! [`Call::timeout`] or [`Call::no_timeout`], then `.await` it; left alone, the setting's default
//! applies, finite for every phase. A timeout nests: a call inside another gets the smaller of its
//! own and what remains of the outer one, and time spent in a Human pause does not count.
//! Cancelling is dropping: a dropped call sends `$/cancel` without blocking (ADR 0078's `Drop` rule).
//! The page API around the waits (locators, actions, `goto`) is ADR 0085's, in `page`.

use std::fmt;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

use crate::page::{Request, Response};
use crate::results::Error;

/// A call's timeout, in three states, set only through `.timeout(d)` and `.no_timeout()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Timeout {
    /// The setting's default for the call's phase.
    #[default]
    Default,
    /// No timeout, asked for explicitly.
    Never,
    /// At most this long. A zero duration fails with a program error naming `.no_timeout()`.
    After(Duration),
}

/// The future of a call, boxed so the sketch can name it. It owns what it needs, so it is
/// `Send + 'static` and can be spawned on any executor (ADR 0081).
pub type CallFuture<T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'static>>;

/// One call of the SDK API, not sent until awaited.
#[must_use = "a call does nothing until it is awaited"]
pub struct Call<T> {
    timeout: Timeout,
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Call<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call").field("timeout", &self.timeout).finish()
    }
}

impl<T> Call<T> {
    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }
}

impl<T: Send + 'static> IntoFuture for Call<T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A waiter for one page event: awaited alone, or armed around an action with [`Waiter::during`].
#[must_use = "a waiter does nothing until it is awaited"]
pub struct Waiter<T> {
    timeout: Timeout,
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Waiter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Waiter").field("timeout", &self.timeout).finish()
    }
}

impl<T: Send + 'static> Waiter<T> {
    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// Only an event this predicate accepts ends the wait. It runs in the program, on the events the
    /// engine sends while the waiter is armed, and may use the program's own values (ADR 0081).
    pub fn matching(self, predicate: impl Fn(&T) -> bool + Send + Sync + 'static) -> Self {
        let _ = predicate;
        unimplemented!()
    }

    /// Arms the waiter, then drives `action`, and returns both results. Rust futures are lazy, so a
    /// waiter merely joined with its trigger could arm too late; this cannot (P02).
    pub async fn during<A, U>(self, action: A) -> Result<(T, U), Error>
    where
        A: IntoFuture<Output = Result<U, Error>>,
    {
        let _ = action;
        unimplemented!()
    }
}

impl<T: Send + 'static> IntoFuture for Waiter<T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Matches a URL.
#[non_exhaustive]
pub enum UrlMatcher {
    /// A glob, such as `**/filter.aspx`.
    Glob(String),
    /// A regular expression, run in the engine in the Rust `regex` syntax: no look-around or
    /// backreferences, and linear time. A pattern using either fails at the call with a program
    /// error naming the construct (ADR 0081). No regex crate is required (G7).
    Regex(String),
    /// A predicate, run in the program on the events the engine sends while it is armed (ADR 0081).
    Predicate(Box<dyn Fn(&str) -> bool + Send + Sync>),
}

impl fmt::Debug for UrlMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Glob(glob) => f.debug_tuple("Glob").field(glob).finish(),
            Self::Regex(regex) => f.debug_tuple("Regex").field(regex).finish(),
            Self::Predicate(_) => f.write_str("Predicate(..)"),
        }
    }
}

impl From<&str> for UrlMatcher {
    fn from(glob: &str) -> Self {
        Self::Glob(glob.to_owned())
    }
}

/// Load states.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadState {
    /// `DOMContentLoaded` fired.
    DomContentLoaded,
    /// `load` fired.
    #[default]
    Load,
    /// No request in flight for 500 ms, WebSockets and server-sent events excluded.
    NetworkIdle,
}

/// The state [`crate::page::Locator::wait_for`] waits for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ElementState {
    /// In the DOM.
    Attached,
    /// Not in the DOM.
    Detached,
    /// In the DOM and visible.
    #[default]
    Visible,
    /// Hidden or absent.
    Hidden,
}

/// A count, exact or bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count {
    min: Option<usize>,
    max: Option<usize>,
}

impl Count {
    /// Exactly `n`.
    pub fn exactly(n: usize) -> Self {
        Self { min: Some(n), max: Some(n) }
    }

    /// At least `n`.
    pub fn at_least(n: usize) -> Self {
        Self { min: Some(n), max: None }
    }

    /// Between `min` and `max`, inclusive.
    pub fn between(min: usize, max: usize) -> Self {
        Self { min: Some(min), max: Some(max) }
    }
}

/// A sum of money in a currency, kept as a decimal so no float rounding reaches a Limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Money {
    amount: String,
    currency: String,
}

impl Money {
    /// Parses a decimal amount, such as `"2.00"`, in an ISO 4217 currency, such as `EUR`.
    pub fn new(amount: &str, currency: &str) -> Result<Self, Error> {
        let _ = (amount, currency);
        unimplemented!()
    }
    /// The decimal amount as written, such as `"0.0031"`.
    pub fn amount(&self) -> &str {
        &self.amount
    }

    /// The ISO 4217 code.
    pub fn currency(&self) -> &str {
        &self.currency
    }
}

/// Limits (ADR 0041) a program sets for its own work: one per kind of Usage, plus a price.
///
/// A program may set and lower them, never raise them: only a person raises a Limit. Reaching one
/// follows ADR 0079: a call refused because a Limit is used up fails with `limit_reached`, and AI
/// work a Limit stops returns `Ended::Stopped` with the Limit by name; the Run ends `stopped` and
/// `incomplete`.
#[derive(Debug, Clone, Default)]
#[expect(dead_code, reason = "a signatures-only sketch: the engine reads the Limits")]
pub struct Limits {
    model_tokens: Option<u64>,
    model_calls: Option<u64>,
    browser_time: Option<Duration>,
    proxy_bytes: Option<u64>,
    paid_solves: Option<u64>,
    paid_fetches: Option<u64>,
    price: Option<Money>,
}

impl Limits {
    /// No Limit beyond those already holding on the scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Model tokens, input and output.
    pub fn model_tokens(self, tokens: u64) -> Self {
        Self { model_tokens: Some(tokens), ..self }
    }

    /// Model calls.
    pub fn model_calls(self, calls: u64) -> Self {
        Self { model_calls: Some(calls), ..self }
    }

    /// Browser time of Usage, not elapsed time: that is the Run's timeout.
    pub fn browser_time(self, time: Duration) -> Self {
        Self { browser_time: Some(time), ..self }
    }

    /// Proxy bytes.
    pub fn proxy_bytes(self, bytes: u64) -> Self {
        Self { proxy_bytes: Some(bytes), ..self }
    }

    /// Paid solves (ADR 0035).
    pub fn paid_solves(self, solves: u64) -> Self {
        Self { paid_solves: Some(solves), ..self }
    }

    /// Paid fetches. Only caps: a Paid fetch is made only under a Limit a Client could not set
    /// (ADRs 0041, 0045).
    pub fn paid_fetches(self, fetches: u64) -> Self {
        Self { paid_fetches: Some(fetches), ..self }
    }

    /// A price, not a kind of Usage: priced through the price table (ADR 0012), as a usage's
    /// `price` is (ADR 0101). A call whose model has no price is refused under it with
    /// `model_price_unknown`.
    pub fn price(self, price: Money) -> Self {
        Self { price: Some(price), ..self }
    }
}

/// The page's navigation and app-state waits. The rest of the page is ADR 0085's, in `page`.
impl crate::lifecycle::Page {
    /// Waits until the page's URL matches.
    pub fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Call<()> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits for a matching response; arm it around its trigger with [`Waiter::during`].
    pub fn wait_for_response(&self, url: impl Into<UrlMatcher>) -> Waiter<Response> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits for a matching request.
    pub fn wait_for_request(&self, url: impl Into<UrlMatcher>) -> Waiter<Request> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits until a matching request has finished, its body read.
    pub fn wait_for_request_finished(&self, url: impl Into<UrlMatcher>) -> Waiter<Request> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits for a load state.
    pub fn wait_for_load_state(&self, state: LoadState) -> Call<()> {
        let _ = state;
        unimplemented!()
    }

    /// Removed: a fixed sleep waits for nothing in particular.
    #[deprecated(
        note = "removed: wait for the app's state (wait_for_url, wait_for_response, wait_for_load_state, \
                Locator::wait_for, Locator::wait_for_count), or use the runtime's own sleep; it takes \
                `Infallible`, so no call to it compiles"
    )]
    pub fn wait_for_timeout(&self, removed: std::convert::Infallible) -> ! {
        match removed {}
    }
}
