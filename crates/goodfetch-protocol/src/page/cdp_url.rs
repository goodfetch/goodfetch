//! A `cdp_url` Session source the engine cannot open (ADR 0115): the
//! Error codes of the probe that opens it, one per remedy. Nothing at the
//! address answered (`cdp_url_unreachable`), something answered that is
//! not a browser speaking CDP (`cdp_url_not_browser`), or the browser's
//! service refused the credentials sent (`cdp_url_unauthorized`). The call
//! that opens the Session raises them, never `connect()`. ADR 0056's
//! address rules, which judge the address before any connection, raise
//! `crate::egress::CDP_URL_NOT_ALLOWED`.
//!
//! Each code's details carry the address, without its query string or any
//! credentials in it, and the settings layer that gave it (ADR 0083); none
//! shows a header's value or a Secret's (ADR 0015).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ARRIVED_IN;
use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup};
use crate::settings::SettingsLayer;

/// Nothing at a `cdp_url`'s address answered (ADR 0115).
pub const CDP_URL_UNREACHABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "cdp_url_unreachable",
    description: "Nothing answered at a `cdp_url` Session source's address when the call that opens the Session connected to it: its name did not resolve, the connection was refused or reset, or TLS failed. It fails at once, never waiting for the address. The details name the address without its query or credentials, the settings layer that gave it, the failure and the fix: start the browser with remote debugging on that port, or correct the address.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Something answered at a `cdp_url`'s address that is not a browser
/// speaking CDP (ADR 0115).
pub const CDP_URL_NOT_BROWSER: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "cdp_url_not_browser",
    description: "Something answered at a `cdp_url` Session source's address, but no browser speaks CDP there: an HTTP status and content type, a WebSocket that refused the upgrade, a CDP target that is not a browser (`page`, `node`, `worker`), or a product the engine does not drive, which `observed` says. The details name the address without its query or credentials, the settings layer that gave it and the fix: give the browser's endpoint, `http://host:port` or `ws://…/devtools/browser/<id>`, and when the same host serves `/json/version`, the browser address found there, which `browser_url` holds.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A `cdp_url`'s browser service refused the credentials sent (ADR 0115).
pub const CDP_URL_UNAUTHORIZED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "cdp_url_unauthorized",
    description: "A `cdp_url` Session source's address answered 401 or 403 to the HTTP request or the WebSocket upgrade. The details name the address without its query or credentials, the settings layer that gave it, the status, each header name sent with its Secret reference, never its value, and the fix: check the Secret references in `headers`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Why nothing at a `cdp_url`'s address answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdpUrlConnectFailure {
    /// The address's name did not resolve.
    NameNotResolved,
    /// The connection was refused: nothing listens on that port.
    ConnectionRefused,
    /// The connection was reset before an answer.
    ConnectionReset,
    /// The TLS handshake failed.
    TlsFailed,
}

/// What `cdp_url_unreachable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpUrlUnreachableDetails {
    /// The `cdp_url`, without its query string or any credentials in it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The settings layer that gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// Why nothing answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: CdpUrlConnectFailure,
    /// What would let it answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `cdp_url_not_browser` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpUrlNotBrowserDetails {
    /// The `cdp_url`, without its query string or any credentials in it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The settings layer that gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// What answered: an HTTP status and content type, a WebSocket that
    /// refused the upgrade, a CDP target that is not a browser, or a
    /// product the engine does not drive.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub observed: String,
    /// The browser's address the same host's `/json/version` lists, when
    /// it serves one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_url: Option<String>,
    /// The address to give instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `cdp_url_unauthorized` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CdpUrlUnauthorizedDetails {
    /// The `cdp_url`, without its query string or any credentials in it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The settings layer that gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: SettingsLayer,
    /// The status it answered, 401 or 403.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: u16,
    /// Each header name sent, with the Secret reference its value came
    /// from; never the value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headers: BTreeMap<String, String>,
    /// What to check.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        CDP_URL_UNREACHABLE,
        CDP_URL_NOT_BROWSER,
        CDP_URL_UNAUTHORIZED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
