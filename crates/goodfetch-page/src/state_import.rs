//! Shared importer for run-only Session sources (`state_file`, `cookie_header`)
//! and Saved identities (ADR 0056; ticket #735).
//!
//! Pure parsing and format detection live here with no CDP dependency, so the
//! `storage_state` and `cookies_txt` fuzz targets exercise the exact parsers
//! that Sessions run (ADR 0067), and `cdp::driver::carry` converts
//! [`ImportedState`] into a browser context's `Carried` state.

use std::fmt;

use goodfetch_engine::secrets;
use goodfetch_protocol::session_export::{SameSite, SessionExport};
use goodfetch_protocol::settings::session_source::StateFileFormat;
use serde::Deserialize;
use serde_json::Value;

/// Maximum bytes read from a `state_file` (matches `session_bundle`'s 32 MiB cap).
pub(crate) const MAX_STATE_FILE_BYTES: usize = 32 * 1024 * 1024;

/// A parsed cookie independent of CDP wire types.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImportedCookie {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) domain: String,
    pub(crate) path: String,
    pub(crate) expires: Option<f64>,
    pub(crate) http_only: bool,
    pub(crate) secure: bool,
    pub(crate) same_site: Option<SameSite>,
}

/// One origin's imported client-side storage from `storageState` or a
/// `session-export` bundle.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ImportedOrigin {
    pub(crate) origin: String,
    pub(crate) local_storage: Vec<(String, String)>,
    /// Playwright 1.64 `indexedDB` array for this origin, when present.
    pub(crate) indexed_db: Option<Value>,
    /// Playwright 1.64 `opfs` entries for this origin, when present.
    pub(crate) opfs: Option<Value>,
}

/// Cookies and per-origin storage parsed from a `state_file` or `cookie_header`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ImportedState {
    pub(crate) cookies: Vec<ImportedCookie>,
    pub(crate) origins: Vec<ImportedOrigin>,
}

/// Why a state file or `Cookie` header could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportError(pub(crate) String);

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Detects the format of `bytes` when `state_file.format` is omitted.
pub(crate) fn detect_format(bytes: &[u8]) -> StateFileFormat {
    if bytes.starts_with(b"age-encryption.org/")
        || bytes.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----")
    {
        return StateFileFormat::SessionExport;
    }
    let trimmed = bytes.iter().copied().find(|b| !b.is_ascii_whitespace());
    match trimmed {
        Some(b'{') => StateFileFormat::StorageState,
        Some(b'[') => StateFileFormat::CdpCookies,
        _ => StateFileFormat::CookiesTxt,
    }
}

/// Wire shape of a cookie inside Playwright's `storageState` or a CDP
/// `Network.Cookie` array.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonCookie {
    name: String,
    value: String,
    domain: String,
    #[serde(default = "default_path")]
    path: String,
    #[serde(default)]
    expires: Option<f64>,
    #[serde(default)]
    http_only: bool,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    same_site: Option<String>,
}

fn default_path() -> String {
    "/".to_owned()
}

#[derive(Deserialize)]
struct StorageEntry {
    name: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageOrigin {
    origin: String,
    #[serde(default)]
    local_storage: Vec<StorageEntry>,
    #[serde(default, rename = "indexedDB")]
    indexed_db: Option<Value>,
    #[serde(default)]
    opfs: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageStateFile {
    #[serde(default)]
    cookies: Vec<JsonCookie>,
    #[serde(default)]
    origins: Vec<StorageOrigin>,
    /// Playwright 1.64 virtual-authenticator credentials, accepted so files
    /// carrying them parse cleanly.
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "accepted from Playwright 1.64 storageState schema"
    )]
    virtual_authenticators: Option<Value>,
}

fn parse_same_site(raw: Option<&str>) -> Option<SameSite> {
    match raw?.to_ascii_lowercase().as_str() {
        "strict" => Some(SameSite::Strict),
        "lax" => Some(SameSite::Lax),
        "none" => Some(SameSite::None),
        _ => None,
    }
}

fn into_imported_cookie(cookie: JsonCookie) -> Option<ImportedCookie> {
    if cookie.name.is_empty() || cookie.domain.is_empty() {
        return None;
    }
    let expires = cookie.expires.filter(|exp| exp.is_finite() && *exp > 0.0);
    Some(ImportedCookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        path: if cookie.path.is_empty() {
            "/".to_owned()
        } else {
            cookie.path
        },
        expires,
        http_only: cookie.http_only,
        secure: cookie.secure,
        same_site: parse_same_site(cookie.same_site.as_deref()),
    })
}

/// Parses a Playwright 1.64 `storageState` JSON document.
pub(crate) fn parse_storage_state(bytes: &[u8]) -> Result<ImportedState, ImportError> {
    let parsed: StorageStateFile = serde_json::from_slice(bytes)
        .map_err(|error| ImportError(format!("invalid Playwright storageState JSON: {error}")))?;
    let cookies = parsed
        .cookies
        .into_iter()
        .filter_map(into_imported_cookie)
        .collect();
    let origins = parsed
        .origins
        .into_iter()
        .filter(|o| o.origin.starts_with("http://") || o.origin.starts_with("https://"))
        .map(|o| ImportedOrigin {
            origin: o.origin,
            local_storage: o
                .local_storage
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect(),
            indexed_db: o.indexed_db.filter(|v| !v.is_null()),
            opfs: o.opfs.filter(|v| !v.is_null()),
        })
        .collect();
    Ok(ImportedState { cookies, origins })
}

/// Parses a CDP `Network.Cookie` JSON array.
pub(crate) fn parse_cdp_cookies(bytes: &[u8]) -> Result<ImportedState, ImportError> {
    let list: Vec<JsonCookie> = serde_json::from_slice(bytes)
        .map_err(|error| ImportError(format!("invalid CDP Network.Cookie array: {error}")))?;
    let cookies = list.into_iter().filter_map(into_imported_cookie).collect();
    Ok(ImportedState {
        cookies,
        origins: Vec::new(),
    })
}

/// Parses a Netscape `cookies.txt` file (7 tab-separated columns, supporting
/// `#HttpOnly_` prefixes and ignoring `#` comments and blank lines).
pub(crate) fn parse_cookies_txt(bytes: &[u8]) -> Result<ImportedState, ImportError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ImportError(format!("cookies.txt is not valid UTF-8: {error}")))?;
    let mut cookies = Vec::new();
    for (line_no, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (http_only, rest) = if let Some(stripped) = line.strip_prefix("#HttpOnly_") {
            (true, stripped)
        } else if line.starts_with('#') {
            continue;
        } else {
            (false, line)
        };
        let mut fields = rest.splitn(7, '\t');
        let (
            Some(domain),
            Some(_flag),
            Some(path),
            Some(secure),
            Some(expiry),
            Some(name),
            Some(value),
        ) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        )
        else {
            return Err(ImportError(format!(
                "cookies.txt line {} does not have 7 tab-separated fields",
                line_no + 1
            )));
        };
        let domain = domain.trim();
        let name = name.trim();
        if domain.is_empty() || name.is_empty() {
            continue;
        }
        let expires = expiry
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|exp| exp.is_finite() && *exp > 0.0);
        let secure = secure.trim().eq_ignore_ascii_case("true");
        let path = if path.is_empty() { "/" } else { path };
        cookies.push(ImportedCookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: domain.to_owned(),
            path: path.to_owned(),
            expires,
            http_only,
            secure,
            same_site: None,
        });
    }
    Ok(ImportedState {
        cookies,
        origins: Vec::new(),
    })
}

/// Converts an opened `session-export` bundle into [`ImportedState`].
pub(crate) fn from_session_export(export: SessionExport) -> ImportedState {
    let cookies = export
        .cookies
        .into_iter()
        .filter(|c| !c.name.is_empty() && !c.domain.is_empty())
        .map(|c| ImportedCookie {
            name: c.name,
            value: c.value.expose().to_owned(),
            domain: c.domain,
            path: if c.path.is_empty() {
                "/".to_owned()
            } else {
                c.path
            },
            expires: c.expires.and_then(|e| u32::try_from(e).ok().map(f64::from)),
            http_only: c.http_only,
            secure: c.secure,
            same_site: c.same_site,
        })
        .collect();
    let origins = export
        .origins
        .into_iter()
        .map(|o| ImportedOrigin {
            origin: o.origin,
            local_storage: o
                .local_storage
                .into_iter()
                .map(|item| (item.name, item.value.expose().to_owned()))
                .collect(),
            indexed_db: None,
            opfs: None,
        })
        .collect();
    ImportedState { cookies, origins }
}

/// Parses a raw `Cookie:` header for a single `domain`.
pub(crate) fn parse_cookie_header(
    header: &str,
    domain: &str,
) -> Result<ImportedState, ImportError> {
    let domain = domain.trim().trim_start_matches('.');
    if domain.is_empty() || domain.contains('/') || domain.contains(char::is_whitespace) {
        return Err(ImportError(format!(
            "cookie_header domain `{domain}` is not a valid host"
        )));
    }
    let body = header
        .trim()
        .strip_prefix("Cookie:")
        .or_else(|| header.trim().strip_prefix("cookie:"))
        .unwrap_or(header.trim());
    let mut cookies = Vec::new();
    for part in body.split(';') {
        let pair = part.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((name, value)) = pair.split_once('=') else {
            return Err(ImportError(format!(
                "cookie_header pair `{pair}` is missing `=`"
            )));
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(ImportError(
                "cookie_header contains an empty cookie name".to_owned(),
            ));
        }
        cookies.push(ImportedCookie {
            name: name.to_owned(),
            value: value.trim().to_owned(),
            domain: domain.to_owned(),
            path: "/".to_owned(),
            expires: None,
            http_only: false,
            secure: true,
            same_site: None,
        });
    }
    if cookies.is_empty() {
        return Err(ImportError(
            "cookie_header contains no `name=value` cookie pairs".to_owned(),
        ));
    }
    Ok(ImportedState {
        cookies,
        origins: Vec::new(),
    })
}

/// Filters `state` to cookies and origins whose host is allowed by `domains`.
/// An empty `domains` slice keeps everything.
pub(crate) fn filter_domains(state: ImportedState, domains: &[String]) -> ImportedState {
    if domains.is_empty() {
        return state;
    }
    let cookies = state
        .cookies
        .into_iter()
        .filter(|cookie| {
            let host = cookie.domain.trim_start_matches('.');
            secrets::allowed(host, domains)
        })
        .collect();
    let origins = state
        .origins
        .into_iter()
        .filter(|origin| {
            url::Url::parse(&origin.origin)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .is_some_and(|host| secrets::allowed(&host, domains))
        })
        .collect();
    ImportedState { cookies, origins }
}

/// Fuzz target entry point for Playwright `storageState` JSON (ADR 0067).
pub fn fuzz_storage_state(data: &[u8]) {
    let _ = parse_storage_state(data);
}

/// Fuzz target entry point for Netscape `cookies.txt` (ADR 0067).
pub fn fuzz_cookies_txt(data: &[u8]) {
    let _ = parse_cookies_txt(data);
}

#[cfg(test)]
mod tests;
