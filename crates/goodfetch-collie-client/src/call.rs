//! The builders every call goes through (spec #606): a [`Call`] waits for
//! its reply and takes `.timeout(d)` or `.no_timeout()`; a [`Query`] asks
//! what the engine answers at once and takes no timeout. Awaiting either
//! only exchanges messages with the Connection's runtime, so it runs on any
//! executor (ADR 0081).

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::error::Error;
use crate::rpc::{Outgoing, Sender};

/// A call's reply, as `.await` gives it.
pub type Reply<T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'static>>;

/// What a handle does with a call's reply once it has come, before the
/// caller reads it: a Run notes that it ended.
type OnReply<T> = Box<dyn FnOnce(&T) + Send + Sync + 'static>;

/// A call that waits for its reply, `T`. Nothing is sent until it is
/// awaited.
#[must_use = "a call is sent only when it is awaited"]
pub struct Call<T> {
    request: Request,
    timeout: Option<Duration>,
    on_reply: Option<OnReply<T>>,
    reply: PhantomData<fn() -> T>,
}

impl<T> Call<T> {
    /// The call of `method` with `params` over `sender`. Until the
    /// Connection takes a scope's default (spec #606), it waits as long as
    /// the Connection lasts.
    pub(crate) fn new(sender: Sender, method: &str, params: Result<Value, Error>) -> Self {
        Self {
            request: Request {
                sender,
                method: method.to_owned(),
                params,
            },
            timeout: None,
            on_reply: None,
            reply: PhantomData,
        }
    }

    /// Runs `on_reply` with the call's reply once it has come.
    pub(crate) fn on_reply(mut self, on_reply: impl FnOnce(&T) + Send + Sync + 'static) -> Self {
        self.on_reply = Some(Box::new(on_reply));
        self
    }

    /// Fails the call with [`Error::TimedOut`] if its reply has not come
    /// within `timeout`; a reply that comes later changes nothing.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Waits for the reply as long as the Connection lasts.
    pub fn no_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for Call<T> {
    type Output = Result<T, Error>;
    type IntoFuture = Reply<T>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            request,
            timeout,
            on_reply,
            reply: _,
        } = self;
        Box::pin(async move {
            let reply = request.send(timeout).await;
            if let (Ok(value), Some(on_reply)) = (&reply, on_reply) {
                on_reply(value);
            }
            reply
        })
    }
}

/// A question the engine answers at once, without waiting on a page, a
/// network or anything else, so it takes no timeout. Nothing is sent until
/// it is awaited.
#[must_use = "a query is sent only when it is awaited"]
pub struct Query<T> {
    request: Request,
    reply: PhantomData<fn() -> T>,
}

impl<T> Query<T> {
    /// The query of `method` with `params` over `sender`.
    pub(crate) fn new(sender: Sender, method: &str, params: Result<Value, Error>) -> Self {
        Self {
            request: Request {
                sender,
                method: method.to_owned(),
                params,
            },
            reply: PhantomData,
        }
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for Query<T> {
    type Output = Result<T, Error>;
    type IntoFuture = Reply<T>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.request.send(None))
    }
}

/// What a call or a query sends.
struct Request {
    sender: Sender,
    method: String,
    params: Result<Value, Error>,
}

impl Request {
    /// Sends the request and reads its reply as `T`.
    async fn send<T: DeserializeOwned>(self, timeout: Option<Duration>) -> Result<T, Error> {
        let params = self.params?;
        let (reply, replied) = oneshot::channel();
        self.sender.call(Outgoing {
            method: self.method,
            params,
            timeout,
            reply,
        });
        let result = replied.await.map_err(|_| Error::Closed)??;
        serde_json::from_value(result).map_err(Error::Shape)
    }
}

/// C-SEND-SYNC: a call and a query, and the futures they become, move
/// between threads and outlive the borrow that made them.
const _: () = {
    const fn send_static<T: Send + 'static>() {}
    send_static::<Call<Value>>();
    send_static::<Query<Value>>();
    send_static::<Reply<Value>>();
};
