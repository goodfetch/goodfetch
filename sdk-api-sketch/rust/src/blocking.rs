//! The same API without `async` (ADR 0081), generated from the same catalog as Python's sync API.
//!
//! Each call hands its work to the connection's runtime and waits on a channel. It never nests a
//! runtime, so it also works inside an async runtime, where it only blocks that thread. A call is a
//! builder ended by [`Call::send`], as in `reqwest::blocking`; a stream is an [`Iterator`]. Handles
//! are cheap clones, safe to use from any thread. Helpers that answer requests run handlers on the
//! `tasks` module's tracked blocking threads (ADR 0060), so blocking handlers are
//! `Fn + Send + Sync + 'static`. Only a few calls are sketched here; the rest mirror the async API.

use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use crate::concurrency::{Record, events};
use crate::lifecycle::ConnectOptions;
use crate::results::Error;
use crate::time::{Limits, Response};

/// One blocking call: set its timeout, then [`Call::send`] it.
#[must_use = "a call does nothing until it is sent"]
pub struct Call<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Call<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Call(..)")
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

    /// Sends the call and blocks until it ends.
    pub fn send(self) -> Result<T, Error> {
        unimplemented!()
    }
}

/// What the engine sends on its own, read in order; dropping it unsubscribes without blocking.
pub struct EventIter<T> {
    _item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for EventIter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EventIter(..)")
    }
}

impl<T> Iterator for EventIter<T> {
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        unimplemented!()
    }
}

/// The engine connection, blocking.
#[derive(Debug, Clone)]
pub struct Goodfetch {
    _private: (),
}

impl Goodfetch {
    /// Starts or finds the engine and completes `hello` before returning.
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// Collie, reached without I/O.
    pub fn collie(&self) -> Collie {
        unimplemented!()
    }

    /// Closes the connection for every clone. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// Collie, blocking.
#[derive(Debug, Clone)]
pub struct Collie {
    _private: (),
}

impl Collie {
    /// Placeholder start verb (#478).
    pub fn start(&self) -> Start {
        unimplemented!()
    }
}

/// Starting a Run, not sent until [`Start::send`].
#[derive(Debug)]
#[must_use = "a Run does not start until this is sent"]
pub struct Start {
    _private: (),
}

impl Start {
    /// How long the whole Run may last.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// The Run's Limits.
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }

    /// Starts the Run and blocks until it has started.
    pub fn send(self) -> Result<Run, Error> {
        unimplemented!()
    }
}

/// A Run, blocking.
#[derive(Debug, Clone)]
pub struct Run {
    _private: (),
}

impl Run {
    /// Opens a page.
    pub fn new_page(&self) -> Call<Page> {
        unimplemented!()
    }

    /// Placeholder for #479: the Run's Records.
    pub fn records(&self) -> EventIter<Record> {
        unimplemented!()
    }

    /// Stops the Run if unfinished. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A page, blocking.
#[derive(Debug, Clone)]
pub struct Page {
    _private: (),
}

impl Page {
    /// Placeholder for #472's navigation.
    pub fn goto(&self, url: &str) -> Call<Option<Response>> {
        let _ = url;
        unimplemented!()
    }

    /// The event's iterator, subscribed when called.
    pub fn events<E: events::PageEvent>(&self, event: E) -> EventIter<E::Payload> {
        let _ = event;
        unimplemented!()
    }

    /// Closes the tab. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}
