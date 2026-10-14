//! Time, waits, deadlines, cancellation and Limits (ADR 0080, decided in #467).
//!
//! Every duration is a [`std::time::Duration`]. A call is a builder: set its timeout with
//! [`Call::timeout`] or [`Call::no_timeout`], then `.await` it; left alone, the setting's default
//! applies, finite for every phase. A timeout nests: a call inside another gets the smaller of its
//! own and what remains of the outer one, and time spent in a Human pause does not count.
//! Cancelling is dropping: a dropped call sends `$/cancel` without blocking (ADR 0078's `Drop` rule).
//! The page API around the waits (locators, actions, `goto`) is a placeholder owned by #472.

use std::fmt;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

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

/// The future of a call, boxed so the sketch can name it.
pub type CallFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

/// One call of the SDK API, not sent until awaited.
#[must_use = "a call does nothing until it is awaited"]
pub struct Call<'a, T> {
    timeout: Timeout,
    _output: PhantomData<&'a T>,
}

impl<T> fmt::Debug for Call<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call").field("timeout", &self.timeout).finish()
    }
}

impl<T> Call<'_, T> {
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

impl<'a, T: Send + 'a> IntoFuture for Call<'a, T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<'a, T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A waiter for one page event: awaited alone, or armed around an action with [`Waiter::during`].
#[must_use = "a waiter does nothing until it is awaited"]
pub struct Waiter<'a, T> {
    timeout: Timeout,
    _output: PhantomData<&'a T>,
}

impl<T> fmt::Debug for Waiter<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Waiter").field("timeout", &self.timeout).finish()
    }
}

impl<'a, T: Send + 'a> Waiter<'a, T> {
    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
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

impl<'a, T: Send + 'a> IntoFuture for Waiter<'a, T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<'a, T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Matches a URL.
#[non_exhaustive]
pub enum UrlMatcher {
    /// A glob, such as `**/filter.aspx`.
    Glob(String),
    /// A regular expression, evaluated by the engine, so no regex crate is required (G7).
    Regex(String),
    /// A host-side predicate; where it runs is #468's.
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

/// The state [`Locator::wait_for`] waits for.
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

/// A response the page received. Its full shape is #472's.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Response {
    /// The final URL.
    pub url: String,
    /// The HTTP status.
    pub status: u16,
}

/// A request the page sent. Its full shape is #472's.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Request {
    /// The URL.
    pub url: String,
    /// The HTTP method.
    pub method: String,
}

/// A placeholder locator: its constructors and actions are #472's; its waits are #467's.
/// Its calls borrow the page, not the locator, so a chained `page.locator(…).click()` can be stored.
#[derive(Debug)]
pub struct Locator<'a> {
    _page: PhantomData<&'a ()>,
}

impl<'a> Locator<'a> {
    /// Waits through the guard (ADR 0019) until the action is safe, then acts.
    pub fn click(&self) -> Call<'a, ()> {
        unimplemented!()
    }

    /// Fills the field.
    pub fn fill(&self, value: &str) -> Call<'a, ()> {
        let _ = value;
        unimplemented!()
    }

    /// Playwright's `wait_for`: one element in the given state.
    pub fn wait_for(&self, state: ElementState) -> Call<'a, ()> {
        let _ = state;
        unimplemented!()
    }

    /// Waits until the number of matches is in range, and returns it: a different behaviour, so a
    /// different name (S3).
    pub fn wait_for_count(&self, count: Count) -> Call<'a, usize> {
        let _ = count;
        unimplemented!()
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
    cost: Option<Money>,
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

    /// A price, not a kind of Usage: priced through ADR 0012's price table.
    pub fn cost(self, cost: Money) -> Self {
        Self { cost: Some(cost), ..self }
    }
}

/// The page's navigation and app-state waits. The page itself is #472's.
impl crate::lifecycle::Page {
    /// Placeholder for #472's navigation. Its timeout is the navigation phase's.
    pub fn goto(&self, url: &str) -> Call<'_, Option<Response>> {
        let _ = url;
        unimplemented!()
    }

    /// Placeholder locator (#472).
    pub fn locator(&self, selector: &str) -> Locator<'_> {
        let _ = selector;
        unimplemented!()
    }

    /// Placeholder locator by ARIA role and accessible name (#472).
    pub fn get_by_role(&self, role: &str, name: &str) -> Locator<'_> {
        let _ = (role, name);
        unimplemented!()
    }

    /// Waits until the page's URL matches.
    pub fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Call<'_, ()> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits for a matching response; arm it around its trigger with [`Waiter::during`].
    pub fn wait_for_response(&self, url: impl Into<UrlMatcher>) -> Waiter<'_, Response> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits for a matching request.
    pub fn wait_for_request(&self, url: impl Into<UrlMatcher>) -> Waiter<'_, Request> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits until a matching request has finished, its body read.
    pub fn wait_for_request_finished(&self, url: impl Into<UrlMatcher>) -> Waiter<'_, Request> {
        let _ = url.into();
        unimplemented!()
    }

    /// Waits for a load state.
    pub fn wait_for_load_state(&self, state: LoadState) -> Call<'_, ()> {
        let _ = state;
        unimplemented!()
    }

    /// Waits until a page function returns a truthy value, returned as JSON text; its
    /// serialization contract is #472's.
    pub fn wait_for_function(&self, page_function: &str) -> Call<'_, String> {
        let _ = page_function;
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
