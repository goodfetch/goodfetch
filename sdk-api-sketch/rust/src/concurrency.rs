//! Concurrency, events and user code (ADR 0081, decided in #468).
//!
//! Every handle ([`Goodfetch`], [`Collie`], [`Run`], [`Page`], [`Locator`]) is a cheap clone of one
//! value, an `Arc` inside, and is `Send + Sync + 'static`; methods take `&self`, and no public type
//! has a lifetime parameter. A call owns what it needs, so its future is `Send + 'static` and can be
//! spawned. `close()` on any clone closes the value for every clone, and dropping the last clone
//! unclosed fires ADR 0078's `Drop` rule. The SDK keeps no queue: each call goes to the engine when
//! it is made, and the engine starts a page's calls in arrival order.
//!
//! The SDK hosts its engine like `main` (ADR 0060 as amended): a connection builds its linked
//! engine's Tokio runtime, or a client runtime for a Daemon or Goodfetch Cloud, through the helper
//! `main` uses (its tests pass the test helper's), so these futures and streams run on any
//! executor, a single-threaded one included. The async API spawns nothing on the program's
//! executor: whatever the engine sends on its own, events and requests that need an answer alike (a route
//! here; a crawl handler, a dialog or a Hook in code elsewhere), arrives as an [`EventStream`],
//! subscribed when the method that returns it is called. The program drives it where it likes, so
//! handlers run inside the future the program awaits, may borrow local state, and need neither
//! `Send` nor `'static`. Dropping a stream unsubscribes without blocking. [`crate::blocking`] is the
//! same API without `async`.
//!
//! The page's events, routes and their payloads are placeholders owned by #472; [`Run::records`] is
//! #479's.

use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::lifecycle::{Collie, Goodfetch, Page, Run};
use crate::results::Error;
use crate::time::{Call, CallFuture, Locator, Request, Response, UrlMatcher, Waiter};

/// What the engine sends on its own, read in order. One reader each: a second reader calls the
/// method again and gets its own subscription. While it is open, an unread lossless sequence slows
/// its producer (ADR 0024); a telemetry sequence keeps only the latest value.
#[must_use = "a stream does nothing until it is read; dropping it unsubscribes"]
pub struct EventStream<T> {
    _item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for EventStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EventStream(..)")
    }
}

impl<T> EventStream<T> {
    /// The next item, or `None` once the sequence has ended. Goodfetch's own method, so the
    /// `futures` crate is never required (G7).
    pub async fn next(&mut self) -> Option<Result<T, Error>> {
        unimplemented!()
    }
}

impl<T> futures_core::Stream for EventStream<T> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let _ = cx;
        unimplemented!()
    }
}

impl<T> Drop for EventStream<T> {
    /// Unsubscribes without blocking.
    fn drop(&mut self) {}
}

/// Page events under Playwright's names (S3), each a value naming its payload. The full list is
/// #472's.
pub mod events {
    /// A page event and the payload it carries.
    pub trait PageEvent: Send + Sync + 'static {
        /// What each event carries.
        type Payload: Send + 'static;
    }

    /// `console`: a console message.
    #[derive(Debug, Clone, Copy)]
    pub struct Console;

    /// `request`: a request the page sent.
    #[derive(Debug, Clone, Copy)]
    pub struct Request;

    /// `response`: a response the page received.
    #[derive(Debug, Clone, Copy)]
    pub struct Response;

    /// `requestfinished`: a request finished, its body read.
    #[derive(Debug, Clone, Copy)]
    pub struct RequestFinished;

    impl PageEvent for Console {
        type Payload = super::ConsoleMessage;
    }

    impl PageEvent for Request {
        type Payload = crate::time::Request;
    }

    impl PageEvent for Response {
        type Payload = crate::time::Response;
    }

    impl PageEvent for RequestFinished {
        type Payload = crate::time::Request;
    }
}

/// A console message. Its full shape is #472's.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConsoleMessage {
    /// `log`, `error`, …
    pub kind: String,
    /// Its text.
    pub text: String,
}

/// An intercepted request waiting for the program's answer. Its semantics are #472's.
#[derive(Debug)]
pub struct Route {
    _private: (),
}

impl Route {
    /// The intercepted request.
    pub fn request(&self) -> &Request {
        unimplemented!()
    }

    /// Lets the request go on unchanged.
    pub fn continue_(self) -> Call<()> {
        unimplemented!()
    }

    /// Fails the request.
    pub fn abort(self) -> Call<()> {
        unimplemented!()
    }
}

/// A Record a crawl committed. A placeholder: its type is #469's.
#[derive(Debug, Clone)]
pub struct Record {
    _private: (),
}

impl Record {
    /// A text field by name.
    pub fn text(&self, field: &str) -> Option<&str> {
        let _ = field;
        unimplemented!()
    }
}

/// The page's events.
impl Page {
    /// The event's stream, subscribed when called: Rust's form of `on` (ADR 0081).
    pub fn events<E: events::PageEvent>(&self, event: E) -> EventStream<E::Payload> {
        let _ = event;
        unimplemented!()
    }

    /// Waits for one event, armed when awaited alone or around an action with
    /// [`Waiter::during`]; [`Waiter::matching`] adds a predicate.
    pub fn wait_for_event<E: events::PageEvent>(&self, event: E) -> Waiter<E::Payload> {
        let _ = event;
        unimplemented!()
    }

    /// Matching requests, each waiting for the program's answer, subscribed when called. One left
    /// unanswered ends by its timeout.
    pub fn route(&self, url: impl Into<UrlMatcher>) -> EventStream<Route> {
        let _ = url.into();
        unimplemented!()
    }
}

/// The Run's sequences.
impl Run {
    /// Placeholder for #479: the Run's Records. Where the sequence starts is #479's.
    pub fn records(&self) -> EventStream<Record> {
        unimplemented!()
    }
}

// C-SEND-SYNC: every handle is a cheap clone, `Send + Sync + 'static`, and every call's future and
// stream can be spawned. Checked whenever the crate compiles.
const _: () = {
    const fn handle<T: Clone + Send + Sync + 'static>() {}
    const fn spawnable<T: Send + 'static>() {}
    handle::<Goodfetch>();
    handle::<Collie>();
    handle::<Run>();
    handle::<Page>();
    handle::<Locator>();
    spawnable::<CallFuture<()>>();
    spawnable::<EventStream<Response>>();
    spawnable::<EventStream<Route>>();
};
