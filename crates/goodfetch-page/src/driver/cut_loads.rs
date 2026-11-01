//! How a load the browser reported finished is told cut all the same:
//! the pinned Chromium loses the failure of a body that broke off once its
//! response was read at the response stage and sent on (ADR 0118's #1732
//! note), so a load from the network with a body that counted no bytes is
//! cut. The browser-fetch tier's loads and a page's navigations are each
//! judged by it.

use goodfetch_cdp::generated::network;

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
