//! A response as the browser reports it over CDP, read one way wherever
//! the engine reads one (a page's navigations, its events, the
//! interception's pipeline, the browser-fetch tier and a paused body's
//! read): its headers by lower-cased name, its status, the length it
//! declared, the rules that tell its body cut, and its body's bytes.
//!
//! A load the browser reported failed after its response arrived broke off,
//! unless the browser cancelled it or its content coding failed to decode
//! (ADR 0118's #699 note); one whose body the engine read and sent on to
//! its own page broke off even when the browser cancelled it, as it does
//! when it turns that copy into a download. A load it reported finished is
//! cut all the same when it came from the network with a body and counted
//! no bytes: the pinned Chromium loses the failure of a body that broke off
//! once its response was read at the response stage and sent on (ADR
//! 0118's #1732 note).

use std::collections::BTreeMap;

use data_encoding::{BASE64, DecodeError};
use goodfetch_cdp::generated::{fetch, network};
use goodfetch_cdp::{Failure, NetError};
use goodfetch_protocol::page::PageResponse;

use super::PageDriver;

/// The headers of the Network domain's request or response, by lower-cased
/// name, each value a string: the browser gives a repeated header's values
/// on lines of their own already.
pub(crate) fn headers(headers: &network::Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            (name.to_ascii_lowercase(), value)
        })
        .collect()
}

/// The headers of a paused response, by lower-cased name, a repeated one's
/// values on lines of their own, as the Network domain gives them.
pub(crate) fn paused_headers(entries: &[fetch::HeaderEntry]) -> BTreeMap<String, String> {
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for header in entries {
        headers
            .entry(header.name.to_ascii_lowercase())
            .and_modify(|value| {
                value.push('\n');
                value.push_str(&header.value);
            })
            .or_insert_with(|| header.value.clone());
    }
    headers
}

/// The headers of a paused response in their order, each with its
/// lower-cased name, a repeated one's entries kept apart.
pub(crate) fn paused_header_list(entries: &[fetch::HeaderEntry]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|header| (header.name.to_ascii_lowercase(), header.value.clone()))
        .collect()
}

/// A response's status as the browser counts it, 0 when it is none an HTTP
/// status can be.
pub(crate) fn status(code: i64) -> u16 {
    u16::try_from(code).unwrap_or_default()
}

/// Whether `status` is a redirect's, a 3xx: never 0, the status of no
/// response.
pub(crate) fn redirects(status: u16) -> bool {
    (300..400).contains(&status)
}

/// The status of the response paused at `paused`; 0 when it paused before
/// a response arrived.
pub(crate) fn paused_status(paused: &fetch::RequestPausedEvent) -> u16 {
    paused.response_status_code.map_or(0, status)
}

/// The response the Network domain reports for the request's hop
/// `request_id` as the protocol's; a document's encoding is read from it
/// once decoded.
pub(crate) fn response_of(request_id: &str, response: &network::Response) -> PageResponse {
    PageResponse {
        url: response.url.clone(),
        status: status(response.status),
        status_text: response.status_text.clone(),
        headers: headers(&response.headers),
        encoding: None,
        request_id: request_id.to_owned(),
        from_service_worker: response.from_service_worker == Some(true),
    }
}

/// The `Content-Length` that `headers`, by lower-cased name, declare.
pub(crate) fn declared_length(headers: &BTreeMap<String, String>) -> Option<u64> {
    headers
        .get("content-length")
        .and_then(|length| length.trim().parse().ok())
}

/// Whether `headers`, by lower-cased name, list a content coding.
pub(crate) fn coded(headers: &BTreeMap<String, String>) -> bool {
    headers
        .get("content-encoding")
        .is_some_and(|coding| !coding.trim().eq_ignore_ascii_case("identity"))
}

/// Whether a load that failed with `error` after its response arrived cut
/// that response's body: any network error but the browser's own cancel
/// and a content coding that failed to decode (ADR 0118's #699 note).
pub(crate) fn cut_by(error: &NetError) -> bool {
    *error != NetError::Aborted && !error.fails_to_decode()
}

/// Whether a load that failed with `error` after the engine read its
/// response's body at the response stage and sent it on to its own page
/// cut that body: any network error but a content coding that failed to
/// decode. The browser's cancel (`net::ERR_ABORTED`) cuts it too: nothing
/// of the engine's cancels that page's load once its response went on, but
/// the browser does when it turns the page's copy into a download (a
/// binary body sniffed under `text/plain`), before the network reported
/// how the body ended, so the bytes read may be short.
pub(crate) fn cut_after_sent_on(error: &NetError) -> bool {
    !error.fails_to_decode()
}

/// The browser's count of a load's encoded bytes, its headers' included,
/// when it counted any.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "CDP counts bytes in a double; a count is whole and far below 2^53"
)]
pub(crate) fn encoded_bytes(encoded: f64) -> Option<u64> {
    (encoded.is_finite() && encoded >= 1.0).then_some(encoded as u64)
}

/// Whether `response`, as the browser handed it to the page, came from
/// the network: an `http:` or `https:` response that neither a cache nor
/// a service worker answered, so its load counts the bytes it took.
pub(crate) fn from_the_network(response: &network::Response) -> bool {
    let scheme = response.url.split_once(':').map(|(scheme, _)| scheme);
    scheme.is_some_and(|scheme| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    }) && response.from_disk_cache != Some(true)
        && response.from_prefetch_cache != Some(true)
        && response.from_service_worker != Some(true)
}

/// Whether a response of `status` to a request of `method` has no body: a
/// `HEAD`'s, and a 1xx, 204 or 304.
pub(crate) fn bodiless(method: Option<&str>, status: u16) -> bool {
    method.is_some_and(|method| method.eq_ignore_ascii_case("HEAD"))
        || matches!(status, 100..=199 | 204 | 304)
}

/// Whether the load of a response of `status` to a request of `method`,
/// which the browser reported finished with `encoded` bytes counted, broke
/// off all the same: it came from the network (`from_the_network`), has a
/// body, and counted none, whatever its framing or content coding (ADR
/// 0118's #1732 note). One a cache answered counts none, and is whole.
///
/// A response read at its response stage is sent on by the interception
/// once its body's stream ended, and with how the network finished it
/// once that arrived. The pinned Chromium's network sends a load it read
/// whole as finished before it ends the body's stream, but ends the stream
/// of one whose body broke off before it sends the failure: when the
/// interception goes on before that failure arrives, it finishes the load
/// itself, counting no bytes, and the failure is lost. A load the network
/// finished counts its headers at least.
pub(crate) fn cut_though_finished(
    method: Option<&str>,
    status: u16,
    from_the_network: bool,
    encoded: f64,
) -> bool {
    from_the_network && !bodiless(method, status) && encoded_bytes(encoded).is_none()
}

/// The bytes of a body, or of a chunk of one, the browser gave as `body`,
/// in Base64 when `base64_encoded`.
///
/// # Errors
///
/// The decoding's, when the Base64 does not decode.
pub(crate) fn bytes_of(body: String, base64_encoded: bool) -> Result<Vec<u8>, DecodeError> {
    if base64_encoded {
        BASE64.decode(body.as_bytes())
    } else {
        Ok(body.into_bytes())
    }
}

/// The browser's answer with a paused response's body was past what one
/// message carries: the body decoded far past what any limit reads whole.
#[derive(Debug)]
pub(crate) struct PastAMessage;

/// The body of the response `driver`'s page paused at `id`, after any
/// content coding, read whole in one message (`Fetch.getResponseBody`):
/// empty, and logged why, when the browser gives none or gives one whose
/// Base64 does not decode.
///
/// # Errors
///
/// [`PastAMessage`] when the browser's answer was past what one message
/// carries.
///
/// # Cancel safety
///
/// Dropping the future loses the read; the response stays paused.
pub(crate) async fn whole_body(driver: &PageDriver, id: &str) -> Result<Vec<u8>, PastAMessage> {
    let read = match driver
        .command(&fetch::GetResponseBodyParams {
            request_id: id.to_owned(),
        })
        .await
    {
        Ok(read) => read,
        Err(failure) => {
            tracing::debug!(%failure, "the browser gave no body of a paused response");
            return match failure {
                Failure::TooLarge { .. } => Err(PastAMessage),
                _ => Ok(Vec::new()),
            };
        }
    };
    Ok(bytes_of(read.body, read.base64_encoded).unwrap_or_else(|error| {
        tracing::warn!(%error, "the browser gave a paused response's body in Base64 that does not decode");
        Vec::new()
    }))
}

#[cfg(test)]
mod tests;
