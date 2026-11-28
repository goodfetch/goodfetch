//! What a session export bundle holds (ADRs 0015 and 0056): a Session's
//! cookies and the storage of each of its origins, `sessionStorage`
//! included, which only this format carries. The bundle encrypts it with
//! `age` to a public key or a passphrase; this is the document inside, a
//! format of the registry (ADR 0066), so a newer engine imports what an
//! older one exported.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::ProtocolVersion;
use crate::page::proxy::ProxyKind;

/// The proxy binding a Saved identity or session export bundle carries
/// (ADR 0035): the proxy server (scheme, host, port — never credentials),
/// provider, country, kind, and sticky session name when bound through a
/// proxy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedProxyBinding {
    /// The proxy server URL without credentials (`scheme://host:port`), when
    /// configured via `proxy.server`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub server: Option<String>,
    /// The Proxy provider name (`proxy.provider`), when provided by a provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provider: Option<String>,
    /// The ISO 3166-1 alpha-2 country code (`proxy.country`), when bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub country: Option<String>,
    /// The proxy kind (`proxy.kind`), when bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: Option<ProxyKind>,
    /// The sticky session identifier, when the provider supports sticky exits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sticky_session: Option<String>,
}

/// The identity metadata a session export bundle carries beside its cookies
/// and storage so an import keeps its Access mode, Identity profile seed,
/// proxy binding, and machine binding unchanged (ADRs 0035 and 0056).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportedIdentity {
    /// The Access mode the Session ran under (`standard`, `stealth`, or `identified`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub access_mode: String,
    /// The Identity profile seed that regenerates the Session's Identity profile.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub identity_seed: u64,
    /// The browser family (`chromium`, `firefox`, or `webkit`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_family: String,
    /// The Session's effective user agent when saved, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub user_agent: Option<String>,
    /// The Session's BCP 47 locale (language), if set or inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locale: Option<String>,
    /// The Session's IANA time zone ID, if set or inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timezone_id: Option<String>,
    /// The Session's proxy binding; none for direct egress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_binding: Option<SavedProxyBinding>,
    /// The importing machine's identity when bound to its machine (ADR 0056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub machine_binding: Option<String>,
    /// Whether the Session was lent to external code via `page.lend()` (ADR 0089).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub was_lent: bool,
}

/// The document a session export bundle encrypts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionExport {
    /// The version of this format the bundle was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format_version: ProtocolVersion,
    /// The Session's cookies.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cookies: Vec<ExportedCookie>,
    /// The storage of each origin the Session kept any in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origins: Vec<ExportedOrigin>,
    /// The Saved identity metadata carried by the bundle, when exported from
    /// a Saved identity or carrying identity bindings (ADRs 0035 and 0056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub identity: Option<ExportedIdentity>,
}

/// One cookie of the Session, with the attributes a browser sets it by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportedCookie {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: SessionValue,
    /// The domain it is sent to, with a leading `.` for its subdomains too.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// The path it is sent under.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// When it expires, in seconds since the Unix epoch; none for a cookie
    /// of the browser's session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expires: Option<i64>,
    /// Whether scripts cannot read it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub http_only: bool,
    /// Whether it is sent over HTTPS only.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secure: bool,
    /// Its `SameSite` attribute; none where the cookie set none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub same_site: Option<SameSite>,
}

/// A cookie's `SameSite` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SameSite {
    /// Sent only on requests of its own site.
    Strict,
    /// Also sent on top-level navigations from another site.
    Lax,
    /// Sent on every request, which needs `secure`.
    None,
}

/// The storage one origin of the Session kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportedOrigin {
    /// The origin, such as `https://shop.example`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: String,
    /// Its `localStorage` items.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub local_storage: Vec<StorageItem>,
    /// Its `sessionStorage` items, which Playwright's `storageState` has
    /// no field for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_storage: Vec<StorageItem>,
}

/// One item of an origin's storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageItem {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: SessionValue,
}

/// A cookie's or a storage item's value, which may be a login: its `Debug`
/// never shows it (STYLE.md), and only [`SessionValue::expose`] reads it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SessionValue(String);

impl SessionValue {
    /// `value` as a Session's value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value itself, for the browser that sets it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionValue([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::SessionValue;

    /// A Session's value never reaches a log line or a panic message
    /// through `Debug`.
    #[test]
    fn a_session_value_debugs_without_itself() {
        assert_eq!(
            format!("{:?}", SessionValue::new("c4n4ry-session-cookie")),
            "SessionValue([redacted])"
        );
    }
}
