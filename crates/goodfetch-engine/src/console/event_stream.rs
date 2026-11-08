//! The Console's fallback transport (ADR 0031): where a proxy refuses
//! `WebSocket`s, a signed-in page carries its Connection over Server-Sent
//! Events plus `POST`. `GET /protocol/events`, with the Console's own
//! `Host`, no other origin's `Origin` and a sign-in's cookie, opens the
//! Connection and answers `text/event-stream`: first a `stream` event whose
//! data names the stream, then one event per message the engine sends.
//! Each `POST /protocol/messages/<stream>` carries the page's messages, one
//! per line, with the Console's own `Origin`, the cookie of the sign-in
//! that opened the stream and that sign-in's CSRF token, and is answered
//! `202` once the Connection holds them; a stream that ended, or another
//! sign-in's, is `404`. The page leaving ends its Connection.

use std::collections::HashMap;

use http_body_util::{BodyExt as _, Either, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use tokio::sync::Mutex;

use super::{Answered, Site, production_headers};
use crate::people::{SignInRefused, sign_in_id};
use crate::rpc::ConsoleConnection;
use crate::rpc::event_stream::{self, Posting};
use crate::rpc::framing::MAX_MESSAGE_BYTES;

/// Where the Console's event stream opens.
pub(super) const EVENTS: &str = "/protocol/events";

/// Where the page posts its messages to its stream:
/// `/protocol/messages/<stream>`.
pub(super) const MESSAGES: &str = "/protocol/messages/";

/// The longest `POST` body the fallback reads.
const MAX_POST_BYTES: usize = MAX_MESSAGE_BYTES;

/// The event streams open, by their name: the sign-in that opened each,
/// and where its `POST`s go.
#[derive(Default)]
pub(super) struct Streams(Mutex<HashMap<String, Opened>>);

/// An event stream open.
struct Opened {
    /// The identifier of the sign-in that opened it.
    sign_in: String,
    posting: Posting,
}

impl Streams {
    /// Keeps `posting` as the stream `name` of the sign-in `sign_in`,
    /// forgetting the streams whose Connection ended.
    async fn open(&self, name: String, sign_in: String, posting: Posting) {
        let mut streams = self.0.lock().await;
        streams.retain(|_, opened| !opened.posting.is_closed());
        streams.insert(name, Opened { sign_in, posting });
    }

    /// Where the `POST`s of the stream `name` go, if it is open and the
    /// sign-in `sign_in` opened it.
    async fn posting(&self, name: &str, sign_in: &str) -> Option<Posting> {
        let streams = self.0.lock().await;
        streams
            .get(name)
            .filter(|opened| opened.sign_in == sign_in && !opened.posting.is_closed())
            .map(|opened| opened.posting.clone())
    }
}

impl Site {
    /// `GET /protocol/events`: for a signed-in read of the Console's own,
    /// a new Connection over an event stream, which the Daemon serves.
    pub(super) async fn event_stream(&self, request: &Request<Incoming>) -> Response<Answered> {
        if request.method() != Method::GET {
            return self.refused_method(&["GET"]).map(Either::Left);
        }
        if let Some(refused) = self.refused_read(request).await {
            return refused.map(Either::Left);
        }
        let Some(sign_in) = self
            .cookie_of(request.headers())
            .and_then(sign_in_id)
            .map(str::to_owned)
        else {
            return self.plain(StatusCode::UNAUTHORIZED).map(Either::Left);
        };
        let name = match stream_name() {
            Ok(name) => name,
            Err(error) => {
                tracing::error!(%error, "the OS generator gave no stream name, so the event stream did not open");
                return self
                    .plain(StatusCode::SERVICE_UNAVAILABLE)
                    .map(Either::Left);
            }
        };
        let first = Bytes::from(format!("event: stream\ndata: {name}\n\n"));
        let (stream, posting, body) = event_stream::opened(first);
        self.streams.open(name, sign_in, posting).await;
        let opened = ConsoleConnection::EventStream(stream);
        if self.connections.send(opened).await.is_err() {
            tracing::debug!("the Daemon stopped before it served a Console's event stream");
            return self
                .plain(StatusCode::SERVICE_UNAVAILABLE)
                .map(Either::Left);
        }
        let mut response = Response::new(Either::Right(body));
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        production_headers(response, &self.policy)
    }

    /// `POST /protocol/messages/<stream>`: hands the body's messages to the
    /// stream's Connection, for a `POST` of the Console's own `Origin`
    /// carrying the cookie of the sign-in that opened the stream and its
    /// CSRF token.
    pub(super) async fn post_messages(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        if request.method() != Method::POST {
            return self.refused_method(&["POST"]);
        }
        let headers = request.headers();
        if let Some(refusal) = self.foreign(headers) {
            return self
                .refused(refusal.reason().as_str(), StatusCode::FORBIDDEN)
                .await;
        }
        if let Err(refusal) = self.signed_in(headers).await {
            return self
                .refused(refusal.reason(), StatusCode::UNAUTHORIZED)
                .await;
        }
        let Some(cookie) = self.cookie_of(headers) else {
            return self
                .refused(
                    SignInRefused::TokenMissing.reason(),
                    StatusCode::UNAUTHORIZED,
                )
                .await;
        };
        if let Err(reason) = self.csrf_checked(headers, cookie).await {
            return self.refused(reason, StatusCode::FORBIDDEN).await;
        }
        let posting = match (stream_of(request.uri().path()), sign_in_id(cookie)) {
            (Some(name), Some(sign_in)) => self.streams.posting(name, sign_in).await,
            _ => None,
        };
        let Some(posting) = posting else {
            return self.plain(StatusCode::NOT_FOUND);
        };
        let Ok(body) = Limited::new(request.into_body(), MAX_POST_BYTES)
            .collect()
            .await
        else {
            return self.plain(StatusCode::PAYLOAD_TOO_LARGE);
        };
        if posting.post(&body.to_bytes()).await {
            self.plain(StatusCode::ACCEPTED)
        } else {
            self.plain(StatusCode::NOT_FOUND)
        }
    }
}

/// A new stream's name, from the OS generator (ADR 0059): 16 bytes in hex.
fn stream_name() -> Result<String, getrandom::Error> {
    let mut drawn = [0_u8; 16];
    getrandom::fill(&mut drawn)?;
    Ok(data_encoding::HEXLOWER.encode(&drawn))
}

/// The stream a `POST` path names, `/protocol/messages/<stream>`: one
/// non-empty segment.
fn stream_of(path: &str) -> Option<&str> {
    path.strip_prefix(MESSAGES)
        .filter(|name| !name.is_empty() && !name.contains('/'))
}

/// The fuzz target `console_post_fallback`'s body (ADR 0067): the inputs of
/// the fallback's `POST`, which any page or program may send, split at the
/// first NUL byte into its path and its body. The stream a path names is one
/// segment that ends it, and the body's messages are its lines. It panics
/// only on a bug.
pub fn fuzz_post_fallback(data: &[u8]) {
    let mut parts = data.splitn(2, |byte| *byte == 0);
    let (path, body) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    if let Some(name) = std::str::from_utf8(path).ok().and_then(stream_of) {
        let path = std::str::from_utf8(path).unwrap_or_default();
        assert!(
            !name.is_empty() && !name.contains('/') && path.ends_with(name),
            "a stream's name is no last segment of its path"
        );
    }
    event_stream::fuzz_posted(body);
}

#[cfg(test)]
mod post_fallback {
    goodfetch_test_support::fuzz_replay!(
        crate::console::fuzz_post_fallback,
        "fuzz/console_post_fallback"
    );
}
