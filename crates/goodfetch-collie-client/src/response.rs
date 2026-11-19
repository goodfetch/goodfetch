//! The response a page's request got, such as a navigation's document
//! (ADR 0085): what `page.goto`, `reload`, `go_back`, `go_forward` and
//! `request` answer, and beside its request what `wait_for_response` and
//! `wait_for_request_finished` answer. Its head is in hand; its body is read on demand,
//! through `response.body`, from the file the engine keeps it in, which a
//! Client on the same machine reads (ADR 0118).

mod decoding;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use goodfetch_protocol::page::events::{
    PageExchange, PageRequest, RESPONSE_BODY, ResponseBodyParams,
};
use goodfetch_protocol::page::{BodyHandle, PageId, PageResponse};
use serde::de::DeserializeOwned;

use crate::call::{Call, Reply};
use crate::error::Error;
use crate::host::Host;

/// The response a page's request got: a navigation's document, or what
/// `page.request` answered. A 404 or a 500 is a response, a value.
#[derive(Clone, Debug)]
pub struct Response {
    host: Host<PageId>,
    response: PageResponse,
}

impl Response {
    /// The response `response` of the page `host` reaches.
    pub(crate) fn new(host: Host<PageId>, response: PageResponse) -> Self {
        Self { host, response }
    }

    /// The Connection its page's Run is on.
    pub(crate) fn connection(&self) -> &crate::ClientConnection {
        self.host.connection()
    }

    /// Its final URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.response.url
    }

    /// Its HTTP status.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.response.status
    }

    /// Its status text.
    #[must_use]
    pub fn status_text(&self) -> &str {
        &self.response.status_text
    }

    /// Whether its status is a success, from 200 to 299. It says nothing
    /// of the page: a login page answered with a 200 is `ok`.
    #[must_use]
    pub fn ok(&self) -> bool {
        (200..=299).contains(&self.response.status)
    }

    /// Its headers, their names lower-cased, as the browser received them.
    #[must_use]
    pub fn headers(&self) -> &BTreeMap<String, String> {
        &self.response.headers
    }

    /// For a navigation's document, the encoding the browser decoded it
    /// with, as `document.characterSet` names it, such as `Shift_JIS` (ADR
    /// 0119); none for another response, and for a navigation that
    /// answered at `commit`, before its document was decoded.
    #[must_use]
    pub fn encoding(&self) -> Option<&str> {
        self.response.encoding.as_deref()
    }

    /// The engine's ID for its request's hop.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.response.request_id
    }

    /// Whether a site's service worker supplied it, from its own cache or
    /// through its own fetch (ADR 0121).
    #[must_use]
    pub fn from_service_worker(&self) -> bool {
        self.response.from_service_worker
    }

    /// Its body's bytes, as the server sent them once their content coding
    /// is undone.
    ///
    /// # Errors
    ///
    /// [`Error::Failed`] with `response_truncated` for a body that broke
    /// off, never returned as whole, and with `body_unavailable` for a body
    /// the browser no longer holds, as after the page navigated away, or a
    /// redirect's, which has none: the engine keeps the body of a response
    /// obtained through `page.wait_for_request_finished` or `page.request`,
    /// which its fix names. [`Error::BodyUnreadable`] when the file the
    /// engine keeps it in cannot be read, and the errors of any call.
    pub fn body(&self) -> Reply<Vec<u8>> {
        let connection = self.host.connection().clone();
        let call: Call<BodyHandle, ResponseBodyParams> = self.host.call(
            RESPONSE_BODY.name,
            ResponseBodyParams {
                page_id: self.host.id().clone(),
                request_id: self.response.request_id.clone(),
                timeout: None,
            },
        );
        let call = call.wire_timeout();
        Box::pin(async move {
            let handle = call.await?;
            connection.read_body(handle).await
        })
    }

    /// Its body as text, decoded with the response's own encoding (ADR
    /// 0119): a navigation's document with the one the browser decoded it
    /// with, [`Self::encoding`]; any other response with its BOM, then its
    /// `Content-Type` charset, then for HTML a `<meta>` as Chrome's
    /// prescan reads it (its first 1,024 bytes always, then on while only
    /// tags allowed in the head appear) or an XML declaration at its very
    /// start, for an XML type that declaration, and UTF-8 last. A legacy charset such as `Shift_JIS` is honoured; a
    /// BOM is left out, and a byte that does not decode becomes U+FFFD.
    /// Unlike Playwright's `text()`, which decodes every body as UTF-8, it
    /// honours the response's encoding, so a page that declares another
    /// one reads as `page.content()` does, not as mojibake.
    ///
    /// # Errors
    ///
    /// Those of [`Self::body`]: `response_truncated` for a body that broke
    /// off, never decoded as whole, and `body_unavailable` for one the
    /// browser no longer holds.
    pub fn text(&self) -> Reply<String> {
        let body = self.body();
        let document = self.response.encoding.clone();
        let content_type = self.response.headers.get("content-type").cloned();
        Box::pin(async move {
            let body = body.await?;
            Ok(decoding::text(
                &body,
                document.as_deref(),
                content_type.as_deref(),
            ))
        })
    }

    /// Its body parsed as JSON into `T`, a [`serde_json::Value`] for any
    /// JSON: decoded as UTF-8 whatever its charset, as JSON is, a leading
    /// BOM ignored.
    ///
    /// # Errors
    ///
    /// Those of [`Self::body`], `response_truncated` for a body that broke
    /// off included, and [`Error::ResponseNotJson`] for a body that is not
    /// JSON, or not the JSON `T` reads.
    pub fn json<T: DeserializeOwned + Send + 'static>(&self) -> Reply<T> {
        let body = self.body();
        let url = self.response.url.clone();
        Box::pin(async move {
            let body = body.await?;
            decoding::json(&body).map_err(|source| Error::ResponseNotJson { url, source })
        })
    }
}

/// A request a page sent and the response it got, which
/// `page.wait_for_response` and `page.wait_for_request_finished` answer:
/// its response reads its body as any [`Response`] does, and one that
/// `wait_for_request_finished` answered reads it whole once the page has
/// moved on.
#[derive(Clone, Debug)]
pub struct Exchange {
    request: PageRequest,
    response: Response,
}

impl Exchange {
    /// The exchange `exchange` of the page `host` reaches.
    pub(crate) fn new(host: Host<PageId>, exchange: PageExchange) -> Self {
        Self {
            request: exchange.request,
            response: Response::new(host, exchange.response),
        }
    }

    /// The request, as the browser sent it.
    #[must_use]
    pub fn request(&self) -> &PageRequest {
        &self.request
    }

    /// The response it got.
    #[must_use]
    pub fn response(&self) -> &Response {
        &self.response
    }

    /// Its request and its response, apart.
    pub(crate) fn into_parts(self) -> (PageRequest, Response) {
        (self.request, self.response)
    }
}
