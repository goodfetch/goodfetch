//! `page.request` (ADRs 0020, 0085, 0092, 0118, 0119 and 0121): Playwright's
//! request API on a page's Session, sent from inside the Session's browser
//! through the browser-fetch tier, so the Session's identity, its cookies
//! among it, never changes, and from a context no service worker controls,
//! so no service worker answers it and no page script reads it. It answers
//! the response whatever its status, a 429 included, and keeps its body,
//! which `response.body` reads: whole, or `response_truncated` for one that
//! broke off. Its body is in Base64 (RFC 4648, with padding).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId, PageResponse};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};

/// The most redirects `page.request` follows, and its default: the
/// browser-fetch tier's own count, as the browser follows at most 19 in one
/// navigation (ADR 0118).
pub const MAX_REDIRECTS: u32 = 19;

/// What `page.request` takes: the page whose Session sends the request, and
/// the request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageRequestParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its method; `GET` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: Option<String>,
    /// Headers set over those the browser sends, each name matched without
    /// regard to case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: Option<BTreeMap<String, String>>,
    /// Its body, in Base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub post_data: Option<String>,
    /// How many redirects it follows, from 0, which answers the 3xx itself,
    /// to 19, the default; past its count it raises `too_many_redirects`,
    /// and a count above 19 is refused with `setting_invalid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_redirects: Option<u32>,
}

/// `page.request`.
pub const PAGE_REQUEST: Operation = Operation {
    name: "page.request",
    description: "Sends a request from inside the page's Session's browser, with its cookies, through the browser-fetch tier and the page's interception (its domain policy, never its routes), from a context no service worker controls, and answers the response whatever its status, a 429 included; `response.body` reads its kept body, and raises `response_truncated` for one that broke off. A count of redirects above 19 is refused with `setting_invalid`, a chain past its count raises `too_many_redirects`, and a body whose content coding fails to decode `navigation_failed`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<PageRequestParams>,
    result: schema_of::<PageResponse>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
