//! The hops of a page's navigation that never reach step 3 as a request of
//! their own (ADRs 0017 and 0120; spec #563's story 5): a redirect's
//! `Location`, and a navigation the page schedules (a `Refresh` header, a
//! `<meta>` refresh or a script), to a scheme other than HTTP's. The
//! browser sends no request for them, so the interception judges them here
//! and refuses them with `egress_refused` (`rule: "scheme"`); a hop to
//! HTTP is judged at step 3 as its request leaves.

use goodfetch_cdp::generated::fetch;
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};
use goodfetch_protocol::egress::EgressRule;
use url::Url;

use crate::driver::cdp_response;

/// The refusal of the redirect answered to `url` with `status` and
/// `headers`, when its `Location` names a scheme no request may use; a
/// `Location` the browser cannot follow it fails itself.
pub(super) fn redirect(
    url: &str,
    status: u16,
    headers: &[fetch::HeaderEntry],
    policy: &Policy,
) -> Option<Refused> {
    if !cdp_response::redirects(status) {
        return None;
    }
    let location = headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("location"))?;
    let target = Url::parse(url).ok()?.join(location.value.trim()).ok()?;
    by_scheme(target.as_str(), policy)
}

/// The refusal of a navigation the page scheduled to `url`, when it names a
/// scheme no request may use. A `javascript:` URL runs in the page's own
/// frame, whose origin the policy allowed; an `about:` or `blob:` URL is a
/// document of the page's own, which reaches no network: none of them is a
/// hop off the policy.
pub(super) fn scheduled(url: &str, policy: &Policy) -> Option<Refused> {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme)?;
    if OWN_DOCUMENTS
        .iter()
        .any(|own| scheme.eq_ignore_ascii_case(own))
    {
        return None;
    }
    by_scheme(url, policy)
}

/// The schemes of a document a page makes itself.
const OWN_DOCUMENTS: [&str; 3] = ["javascript", "about", "blob"];

/// The refusal of a hop to `url`, when its scheme is what refuses it.
fn by_scheme(url: &str, policy: &Policy) -> Option<Refused> {
    let request = egress::Request {
        url,
        kind: RequestKind::Page,
    };
    match egress::judge(&request, policy) {
        Judgment::Refused(refused) if refused.rule == EgressRule::Scheme => Some(refused),
        _ => None,
    }
}
