//! Steps 3 to 8 of the pipeline for what the last route sends on, as one
//! function of the request: the egress check and the domain policy, the
//! places of the steps later tickets fill, and the command that sends the
//! request on. Pure, so the per-request benchmark runs it as the pipeline
//! does (ADR 0071).

use std::collections::BTreeMap;

use goodfetch_cdp::generated::fetch;
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};

/// What to send instead of what a request carries; each part replaces the
/// request's whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Overrides {
    pub(crate) url: Option<String>,
    pub(crate) method: Option<String>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    /// In Base64.
    pub(crate) post_data: Option<String>,
}

impl Overrides {
    /// These overrides, each part `later` names taken over its own.
    pub(crate) fn then(self, later: Self) -> Self {
        Self {
            url: later.url.or(self.url),
            method: later.method.or(self.method),
            headers: later.headers.or(self.headers),
            post_data: later.post_data.or(self.post_data),
        }
    }
}

/// Steps 3 to 7 for the request paused at `id` to `url`, with `overrides`,
/// under `policy`, and step 8's command; `fetch` pauses its response.
///
/// # Errors
///
/// The domain policy's refusal: the request goes no further.
pub(crate) fn send_on(
    id: &fetch::RequestId,
    url: &str,
    overrides: &Overrides,
    policy: &Policy,
    fetch: bool,
) -> Result<fetch::ContinueRequestParams, Refused> {
    // Step 3: the egress check and the domain policy.
    let request = egress::Request {
        url,
        kind: RequestKind::Page,
    };
    if let Judgment::Refused(refused) = egress::judge(&request, policy) {
        return Err(refused);
    }
    // Step 4: the never-block list, the Filter lists and
    // `browser.block_resources` (#713).
    // Step 5: the Response cache and a replaying Run's Response recording
    // (area 6).
    // Step 6: the page's navigation gate, then its lease check (#720).
    // Step 7: the engine's own additions, which no route read. The
    // Session's cookies are the browser's own: it puts them on the request
    // as it sends it on, whatever headers a route set, and the routes' view
    // leaves them out. Secret placeholders and the Session's credentials
    // (#654, #733) are resolved here.
    // Step 8: the network.
    Ok(fetch::ContinueRequestParams {
        request_id: id.clone(),
        url: overrides.url.clone(),
        method: overrides.method.clone(),
        post_data: overrides.post_data.clone(),
        headers: overrides.headers.as_ref().map(entries),
        intercept_response: fetch.then_some(true),
    })
}

/// Headers as the browser takes them.
pub(crate) fn entries(headers: &BTreeMap<String, String>) -> Vec<fetch::HeaderEntry> {
    headers
        .iter()
        .map(|(name, value)| fetch::HeaderEntry {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}
