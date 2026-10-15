//! Concurrency, events and user code (ADR 0081, decided in #468).
//!
//! Every handle ([`Goodfetch`], [`Collie`], [`Run`], [`Page`], [`Locator`]) is a cheap clone of
//! one value, an `Arc` inside, and is `Send + Sync + 'static`; methods take `&self`, and no public
//! type has a lifetime parameter. A call owns what it needs, so its future is `Send + 'static` and
//! can be spawned. `close()` on any clone closes the value for every clone, and dropping the last
//! clone unclosed fires ADR 0078's `Drop` rule. The SDK keeps no queue: each call goes to the
//! engine when it is made, and the engine starts a page's calls in arrival order.
//!
//! The SDK hosts its engine like `main` (ADR 0060 as amended): a connection builds its linked
//! engine's Tokio runtime, or a client runtime for a Daemon or Goodfetch Cloud, through the helper
//! `main` uses (unit tests over the in-memory transport pair hand it the paused one), so these
//! futures and streams run on any executor, a single-threaded one included. The async API spawns
//! nothing on the program's executor: whatever the engine sends on its own, events and requests
//! that need an answer alike (a route here; a crawl handler, a dialog or a Hook in code
//! elsewhere), arrives as an [`EventStream`], subscribed when the method that returns it is
//! called. The program drives it where it likes, so handlers run inside the future the program
//! awaits, may borrow local state, and need neither `Send` nor `'static`. Dropping a stream
//! unsubscribes without blocking. [`crate::blocking`] is the same API without `async`.
//!
//! The page's events, routes, dialogs and the data they carry are ADR 0085's, in `page`;
//! [`Run::records`] is #479's.

use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::lifecycle::{Collie, Goodfetch, Page, Run, Session};
use crate::page::{CdpSession, Download, FrameLocator, Locator, Response, Route, WebSocket};
use crate::results::Error;
use crate::time::{CallFuture, Waiter};

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

/// Page events under Playwright's names (S3), each a value naming the data it carries (ADR 0085).
pub mod events {
    use crate::lifecycle::Page;
    use crate::page::{ConsoleMessage, Dialog as DialogRequest, Download as DownloadValue, FileChooser as FileChooserRequest};
    use crate::page::{Request as RequestValue, Response as ResponseValue, WebError as WebErrorValue, WebSocket as WebSocketValue};

    /// A page event and the data it carries.
    pub trait PageEvent: Send + Sync + 'static {
        /// What each event carries.
        type Data: Send + 'static;
    }

    /// A Session event and the data it carries.
    pub trait SessionEvent: Send + Sync + 'static {
        /// What each event carries.
        type Data: Send + 'static;
    }

    macro_rules! page_event {
        ($(#[$doc:meta])* $name:ident => $data:ty) => {
            $(#[$doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $name;

            impl PageEvent for $name {
                type Data = $data;
            }
        };
    }

    page_event!(
        /// `console`: a console message.
        Console => ConsoleMessage
    );
    page_event!(
        /// `pageerror`: an exception the page's own scripts did not catch.
        WebError => WebErrorValue
    );
    page_event!(
        /// `request`: a request the page sent.
        Request => RequestValue
    );
    page_event!(
        /// `requestfinished`: a request finished, its body read.
        RequestFinished => RequestValue
    );
    page_event!(
        /// `requestfailed`: a request failed; its `failure()` says why.
        RequestFailed => RequestValue
    );
    page_event!(
        /// `response`: a response the page received.
        Response => ResponseValue
    );
    page_event!(
        /// `popup`: a page this page opened, in the same Session.
        Popup => Page
    );
    page_event!(
        /// `download`: a download, kept by the engine with its Run.
        Download => DownloadValue
    );
    page_event!(
        /// `filechooser`: a file chooser waiting for its files.
        FileChooser => FileChooserRequest
    );
    page_event!(
        /// `websocket`: a WebSocket the page opened.
        WebSocket => WebSocketValue
    );
    page_event!(
        /// `load`.
        Load => ()
    );
    page_event!(
        /// `domcontentloaded`.
        DomContentLoaded => ()
    );
    page_event!(
        /// `close`: the page closed.
        Close => ()
    );
    page_event!(
        /// `crash`: the renderer crashed; calls fail with `page_crashed`.
        Crash => ()
    );
    page_event!(
        /// `dialog`: a dialog waiting for the program's answer. Dropping one unanswered passes it on;
        /// past the last subscriber, or with none, it gets Playwright's default (dismissed, or accepted
        /// for `beforeunload`).
        Dialog => DialogRequest
    );

    /// `page`: a new page in the Session, one it opened, a popup, or a tab a link opened.
    #[derive(Debug, Clone, Copy)]
    pub struct NewPage;

    impl SessionEvent for NewPage {
        type Data = Page;
    }
}

/// A Record a crawl committed. A placeholder: a Record has its Dataset's type (ADR 0082).
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
    pub fn events<E: events::PageEvent>(&self, event: E) -> EventStream<E::Data> {
        let _ = event;
        unimplemented!()
    }

    /// Waits for one event, armed when awaited alone or around an action with
    /// [`Waiter::during`]; [`Waiter::matching`] adds a predicate.
    pub fn wait_for_event<E: events::PageEvent>(&self, event: E) -> Waiter<E::Data> {
        let _ = event;
        unimplemented!()
    }

}

/// The Session's events.
impl Session {
    /// The event's stream, subscribed when called.
    pub fn events<E: events::SessionEvent>(&self, event: E) -> EventStream<E::Data> {
        let _ = event;
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
    handle::<crate::crawl::CrawlRun>();
    handle::<crate::crawl::Foxhound>();
    handle::<Session>();
    handle::<Page>();
    handle::<Locator>();
    handle::<FrameLocator>();
    handle::<CdpSession>();
    handle::<Download>();
    handle::<WebSocket>();
    handle::<crate::data::Dataset<serde_json::Map<String, serde_json::Value>>>();
    spawnable::<CallFuture<()>>();
    spawnable::<EventStream<Response>>();
    spawnable::<EventStream<Route>>();
};
