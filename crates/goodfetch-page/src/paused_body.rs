//! The body of a response paused at its response stage (`Fetch`), as the
//! engine reads it through the browser under the Run's limits (SC-05): the
//! one rule a body is judged by before it is read, the read from the
//! browser's stream that judges it as it arrives, and how a load whose
//! body was read so, then sent on, is told cut when the browser lost its
//! failure.

use std::collections::BTreeMap;

use data_encoding::BASE64;
use goodfetch_cdp::generated::{fetch, io, network};
use goodfetch_engine::CallError;
use goodfetch_engine::response_limits::Limits;
use goodfetch_protocol::page::{RESPONSE_TRUNCATED, ResponseTruncatedDetails};

use crate::driver::PageDriver;

/// The bytes the browser reads in one `IO.read` of a body's stream: their
/// Base64 stays far below what a CDP message carries.
const STREAM_CHUNK: i64 = 1024 * 1024;

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

/// Whether the load of a response the network answered (`from_the_network`)
/// with a body (`bodiless` false), which the browser reported finished
/// with `encoded` bytes counted, broke off all the same: it counted none
/// (ADR 0118's #1732 note).
///
/// A response read at its response stage is sent on by the interception
/// once its body's stream ended, and with how the network finished it
/// once that arrived. The pinned Chromium's network sends a load it read
/// whole as finished before it ends the body's stream, but ends the stream
/// of one whose body broke off before it sends the failure: when the
/// interception goes on before that failure arrives, it finishes the load
/// itself, counting no bytes, and the failure is lost, whatever the body's
/// framing or content coding. A load the network finished counts its
/// headers at least.
pub(crate) fn lost_its_failure(from_the_network: bool, bodiless: bool, encoded: f64) -> bool {
    from_the_network && !bodiless && encoded_bytes(encoded).is_none()
}

/// Whether `headers`, by lower-cased name, list a content coding.
pub(crate) fn coded(headers: &BTreeMap<String, String>) -> bool {
    headers
        .get("content-encoding")
        .is_some_and(|coding| !coding.trim().eq_ignore_ascii_case("identity"))
}

/// Refuses the body of `url`, whose response has `headers` by lower-cased
/// name, when its `Content-Length` declares it past `network.max_body_size`
/// with no content coding, so the browser's copy is never read: the one
/// rule a body read through the browser is judged by before it is read.
///
/// # Errors
///
/// `response_limit_exceeded` naming `network.max_body_size`.
pub(crate) fn declared_within_limits(
    url: &str,
    headers: &BTreeMap<String, String>,
    limits: &Limits,
) -> Result<(), CallError> {
    let declared = headers
        .get("content-length")
        .and_then(|length| length.trim().parse::<u64>().ok());
    match declared {
        Some(length) if !coded(headers) => limits
            .judge_decoded(url, length, None)
            .map_err(|exceeded| exceeded.error()),
        _ => Ok(()),
    }
}

/// Reads the body of the response paused at `id`, the body of `url` that
/// declared `declared` bytes on the wire, from the browser's stream of it
/// (`Fetch.takeResponseBodyAsStream`), after any content coding, judging it
/// by `limits` as it arrives: the stream closes at the first limit passed,
/// so neither the browser nor the engine holds more than the limit and one
/// chunk. A body the browser gives no stream of reads as empty, as one it
/// gives no body of does; a stream whose read fails, or whose chunk does
/// not decode, broke off, and is never read as the whole body.
///
/// The paused request cannot be sent on as it came once its body is taken,
/// and the pinned Chromium then reports no network or decoding failure of
/// it: its caller answers it.
///
/// # Errors
///
/// `response_limit_exceeded` naming the limit the body passed;
/// `response_truncated` for a stream that broke off.
pub(crate) async fn streamed(
    driver: &PageDriver,
    id: &fetch::RequestId,
    url: &str,
    declared: Option<u64>,
    limits: &Limits,
) -> Result<Vec<u8>, CallError> {
    let stream = match driver
        .command(&fetch::TakeResponseBodyAsStreamParams {
            request_id: id.clone(),
        })
        .await
    {
        Ok(taken) => taken.stream,
        Err(failure) => {
            tracing::debug!(%failure, "the browser gave no stream of a paused response's body");
            return Ok(Vec::new());
        }
    };
    let mut bytes = Vec::new();
    let read = loop {
        let chunk = match driver
            .command(&io::ReadParams {
                handle: stream.clone(),
                offset: None,
                size: Some(STREAM_CHUNK),
            })
            .await
        {
            Ok(chunk) => chunk,
            Err(failure) => {
                tracing::debug!(%failure, "a paused response's body stream broke off");
                break Err(broke_off(url, &bytes, declared));
            }
        };
        if chunk.base64_encoded == Some(true) {
            match BASE64.decode(chunk.data.as_bytes()) {
                Ok(decoded) => bytes.extend_from_slice(&decoded),
                Err(error) => {
                    tracing::warn!(%error, "the browser gave a body's chunk in Base64 that does not decode");
                    break Err(broke_off(url, &bytes, declared));
                }
            }
        } else {
            bytes.extend_from_slice(chunk.data.as_bytes());
        }
        let decoded = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if let Err(exceeded) = limits.judge_decoded(url, decoded, declared) {
            break Err(exceeded.error());
        }
        if chunk.eof {
            break Ok(());
        }
    };
    if let Err(failure) = driver.command(&io::CloseParams { handle: stream }).await {
        tracing::debug!(%failure, "a body's stream was closed already");
    }
    read.map(|()| bytes)
}

/// `response_truncated` for the body of `url`, whose stream broke off after
/// `bytes`, decoded, of the `declared` bytes on the wire.
fn broke_off(url: &str, bytes: &[u8], declared: Option<u64>) -> CallError {
    let received = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    // `received` counts decoded bytes and `declared` the bytes as sent, so
    // the two are never read as a fraction.
    let of = declared.map_or_else(String::new, |declared| {
        format!("; its `Content-Length` declared {declared} as sent")
    });
    CallError::new(
        &RESPONSE_TRUNCATED,
        format!("the body of {url} broke off after {received} bytes once decoded{of}"),
        &ResponseTruncatedDetails {
            url: url.to_owned(),
            received,
            expected: declared,
        },
    )
}
