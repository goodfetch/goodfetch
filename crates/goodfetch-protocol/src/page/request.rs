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
    /// Headers whose value holds a Secret placeholder, set over the
    /// browser's own and named in none of `headers`: the engine adds each
    /// value at its interception, at every hop whose URL is on the
    /// placeholder's bound domains, and fails the request with
    /// `secret_domain_not_allowed` at a hop outside them (ADR 0092).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secret_headers: Option<BTreeMap<String, SecretHeader>>,
    /// Strings of its body, which is then JSON, that hold a Secret
    /// placeholder's value, each set at its JSON Pointer by the engine at
    /// its interception, checked as `secret_headers` are at every hop that
    /// sends the body (ADR 0092).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub json_secrets: Option<Vec<JsonSecret>>,
}

/// A header value that holds a Secret placeholder (ADR 0092). The value
/// the engine builds from the placeholder's is masked wherever the engine
/// returns, sends or keeps it, its Base64 in `Basic` included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "form", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretHeader {
    /// The placeholder's value after `prefix`, such as `Bearer `.
    Value {
        /// The Secret placeholder.
        #[schemars(extend("arrived_in" = "0.1"))]
        placeholder: String,
        /// The text before the value; none when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        prefix: Option<String>,
    },
    /// HTTP Basic authentication's credentials (RFC 7617): `Basic` and the
    /// Base64 of `username`, a colon and the placeholder's value.
    Basic {
        /// The user ID, which holds no colon.
        #[schemars(extend("arrived_in" = "0.1"))]
        username: String,
        /// The Secret placeholder whose value is the password.
        #[schemars(extend("arrived_in" = "0.1"))]
        placeholder: String,
    },
}

impl SecretHeader {
    /// Its Secret placeholder.
    pub fn placeholder(&self) -> &str {
        match self {
            Self::Value { placeholder, .. } | Self::Basic { placeholder, .. } => placeholder,
        }
    }
}

/// A string of a JSON body that holds a Secret placeholder's value
/// (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonSecret {
    /// Where the string goes, as a JSON Pointer (RFC 6901): a member of an
    /// object, added or replaced, or an element an array holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pointer: String,
    /// The Secret placeholder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
}

/// `page.request`.
pub const PAGE_REQUEST: Operation = Operation {
    name: "page.request",
    description: "Sends a request from inside the page's Session's browser, with its cookies, through the browser-fetch tier and the page's interception (its domain policy, never its routes), from a context no service worker controls, and answers the response whatever its status, a 429 included; `response.body` reads its kept body, and raises `response_truncated` for one that broke off. A count of redirects above 19 is refused with `setting_invalid`, a chain past its count raises `too_many_redirects`, and a body whose content coding fails to decode `navigation_failed`. A header value or a JSON body string may hold a Secret placeholder, whose value the engine adds at its interception after every page script and service worker: an unbound placeholder raises `secret_unbound`, a hop outside its bound domains `secret_domain_not_allowed`, and a header value holding a CR, an LF or a NUL `header_value_invalid`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<PageRequestParams>,
    result: schema_of::<PageResponse>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
