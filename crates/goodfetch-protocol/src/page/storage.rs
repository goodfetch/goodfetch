//! SDK-only cookies on a Session and Web Storage on a page (ADR 0092;
//! ticket #746): `session.cookies`, `session.add_cookies`,
//! `session.clear_cookies`, `page.local_storage` and `page.session_storage`.
//!
//! Each operation is marked [`Reach::SdkOnly`] in the catalog so no CLI
//! command, MCP tool, skill, Framework adapter or Code mode surface offers it
//! and cookies and storage never reach a model (ADRs 0011, 0014 and 0092).
//! On a Session whose source binds its domains (`running_browser`,
//! `profile_directory`, or `cdp_url` naming `domains`), any cookie or storage
//! access outside those registrable domains fails with `domain_not_allowed`
//! (ADR 0056).

use std::collections::BTreeMap;
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId, SessionId};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};
use crate::point_in_time::PointInTime;

/// A cookie's `SameSite` attribute (`strict`, `lax`, or `none`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CookieSameSite {
    /// Sent only on requests of its own site.
    Strict,
    /// Also sent on top-level navigations from another site.
    Lax,
    /// Sent on every request, which needs `secure`.
    None,
}

/// One cookie of a Session, carrying Playwright's fields with an RFC 3339
/// UTC expiry (`null` for a session cookie; ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cookie {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
    /// The domain it is sent to, with a leading `.` for its subdomains too.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// The path it is sent under.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// When it expires, in RFC 3339 UTC; `null` for a session cookie.
    #[serde(default, with = "crate::point_in_time::nullable")]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub expires: Option<SystemTime>,
    /// Whether scripts cannot read it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub http_only: bool,
    /// Whether it is sent over HTTPS only.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secure: bool,
    /// Its `SameSite` attribute (`strict`, `lax`, or `none`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub same_site: CookieSameSite,
}

/// One cookie to add to a Session via `session.add_cookies` (ADR 0092).
///
/// Provide either `url`, or both `domain` and `path`, matching Playwright's
/// `addCookies`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewCookie {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: String,
    /// The URL to derive domain, path and `secure` from; mutually exclusive
    /// with `domain` and `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// The domain it is sent to; requires `path` and cannot be combined with
    /// `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: Option<String>,
    /// The path it is sent under; requires `domain` and cannot be combined
    /// with `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
    /// When it expires, in RFC 3339 UTC; omitted or `null` for a session
    /// cookie.
    #[serde(
        default,
        with = "crate::point_in_time::nullable",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub expires: Option<SystemTime>,
    /// Whether scripts cannot read it; defaults to `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub http_only: Option<bool>,
    /// Whether it is sent over HTTPS only; defaults to `true` for `https:`
    /// URLs and `false` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secure: Option<bool>,
    /// Its `SameSite` attribute; defaults to the browser's default (`lax`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub same_site: Option<CookieSameSite>,
}

/// What `session.cookies` takes: the Session and an optional list of URLs to
/// filter cookies by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionCookiesParams {
    /// The Session whose cookies to read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// When present, return only cookies that would be sent to at least one of
    /// these URLs. On a domain-bound Session, any URL outside the source's
    /// `domains` fails with `domain_not_allowed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub urls: Option<Vec<String>>,
}

/// What `session.cookies` answers: the matching cookies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCookiesResult {
    /// The matching cookies, bounded to the Session source's `domains` when
    /// bound.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cookies: Vec<Cookie>,
}

/// What `session.add_cookies` takes: the Session and the cookies to add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddCookiesParams {
    /// The Session to add cookies to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// The cookies to set. On a domain-bound Session, any cookie outside the
    /// source's `domains` fails the whole call with `domain_not_allowed`
    /// without writing any cookie.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cookies: Vec<NewCookie>,
}

/// What `session.clear_cookies` takes: the Session and optional exact-match
/// filters (`name`, `domain`, `path`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearCookiesParams {
    /// The Session whose cookies to clear.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// Clear only cookies with this exact name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: Option<String>,
    /// Clear only cookies for this domain. On a domain-bound Session, a domain
    /// outside the source's `domains` fails with `domain_not_allowed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: Option<String>,
    /// Clear only cookies with this exact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
}

/// What `session.add_cookies` and `session.clear_cookies` answer once applied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCookiesChanged {}

/// One `WebStorage` action on `page.local_storage` or `page.session_storage`
/// (ADR 0092): `items`, `get_item`, `set_item`, `remove_item`, or `clear`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WebStorageAction {
    /// Reads all key-value pairs of the page's current origin.
    Items,
    /// Reads one key of the page's current origin.
    GetItem {
        /// The storage key to read.
        #[schemars(extend("arrived_in" = "0.1"))]
        key: String,
    },
    /// Sets one key-value pair on the page's current origin.
    SetItem {
        /// The storage key to set.
        #[schemars(extend("arrived_in" = "0.1"))]
        key: String,
        /// The string value to store.
        #[schemars(extend("arrived_in" = "0.1"))]
        value: String,
    },
    /// Removes one key from the page's current origin.
    RemoveItem {
        /// The storage key to remove.
        #[schemars(extend("arrived_in" = "0.1"))]
        key: String,
    },
    /// Clears all keys of the page's current origin.
    Clear,
}

/// What `page.local_storage` and `page.session_storage` take: the page and
/// the `WebStorage` action to perform on its current origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageStorageParams {
    /// The page whose current origin's storage to access.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The `WebStorage` action to perform.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub action: WebStorageAction,
}

/// What `page.local_storage` and `page.session_storage` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageStorageResult {
    /// All key-value pairs of the page's current origin (for `items`).
    Items {
        /// The stored items, ordered by key.
        #[schemars(extend("arrived_in" = "0.1"))]
        items: BTreeMap<String, String>,
    },
    /// The value of one key, or `null` when absent (for `get_item`).
    Item {
        /// The stored value, or `null` when the key is absent.
        #[schemars(extend("arrived_in" = "0.1"))]
        value: Option<String>,
    },
    /// Confirmation that `set_item`, `remove_item`, or `clear` completed.
    Updated {},
}

/// `session.cookies`.
pub const SESSION_COOKIES: Operation = Operation {
    name: "session.cookies",
    description: "Reads a Session's cookies carrying Playwright's fields with RFC 3339 UTC expiries (`null` for session cookies). Offered in the SDKs only so cookies never reach a model, and bounded to a source's `domains` (`domain_not_allowed` outside; ADR 0092).",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<SessionCookiesParams>,
    result: schema_of::<SessionCookiesResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.add_cookies`.
pub const SESSION_ADD_COOKIES: Operation = Operation {
    name: "session.add_cookies",
    description: "Adds cookies to a Session's browser context, affecting subsequent navigations and `page.request` calls. Offered in the SDKs only, and bounded to a source's `domains` (`domain_not_allowed` outside; ADR 0092).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<AddCookiesParams>,
    result: schema_of::<SessionCookiesChanged>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.clear_cookies`.
pub const SESSION_CLEAR_COOKIES: Operation = Operation {
    name: "session.clear_cookies",
    description: "Clears cookies from a Session's browser context, optionally filtered by `name`, `domain`, or `path`. Offered in the SDKs only, and bounded to a source's `domains` (`domain_not_allowed` outside; ADR 0092).",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<ClearCookiesParams>,
    result: schema_of::<SessionCookiesChanged>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.local_storage`.
pub const PAGE_LOCAL_STORAGE: Operation = Operation {
    name: "page.local_storage",
    description: "Reads or mutates `window.localStorage` (`items`, `get_item`, `set_item`, `remove_item`, `clear`) on the page's current origin. Offered in the SDKs only, and bounded to a source's `domains` (`domain_not_allowed` outside; ADR 0092).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<PageStorageParams>,
    result: schema_of::<PageStorageResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.session_storage`.
pub const PAGE_SESSION_STORAGE: Operation = Operation {
    name: "page.session_storage",
    description: "Reads or mutates `window.sessionStorage` (`items`, `get_item`, `set_item`, `remove_item`, `clear`) on the page's current origin (per-page tab storage). Offered in the SDKs only, and bounded to a source's `domains` (`domain_not_allowed` outside; ADR 0092).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<PageStorageParams>,
    result: schema_of::<PageStorageResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
