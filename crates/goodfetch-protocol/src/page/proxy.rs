//! A Session's proxy (ADRs 0021, 0053 and 0092; ADR 0037 / SA-22): the
//! settings `proxy.server`, `proxy.bypass`, `proxy.username` and
//! `proxy.password`, with Playwright's fields, set on a Run, whose Sessions
//! all take them, or on one Session, never on a page or a request, so the
//! address a site sees never changes under the same cookies. Only the
//! program's own code, configuration and an Operator policy set them: a
//! Recipe may expose them but never sets them, and a Client acting for a
//! model is refused with `setting_locked`, since whoever sets `proxy.server`
//! receives `proxy.password`. A URL never carries credentials, and `socks4`,
//! which carries no password, is refused.
//!
//! A proxy that cannot be reached or refuses the tunnel raises
//! `proxy_unavailable`, never `navigation_failed` (pitfall H12), and a
//! proxy that refuses the Session's credentials with a 407 raises
//! `proxy_auth_failed` (ADR 0053).

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ARRIVED_IN;
use crate::catalog::{Authority, ErrorCodeDefinition, ErrorGroup, Setting, ValueKind, ValueShape};

/// A proxy's scheme, as `proxy.server` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProxyScheme {
    /// An HTTP proxy, reached in clear.
    Http,
    /// An HTTP proxy reached over TLS.
    Https,
    /// A SOCKS5 proxy.
    Socks5,
    /// A SOCKS5 proxy that resolves the names it is given.
    Socks5h,
}

impl ProxyScheme {
    /// Its name in a URL.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
            Self::Socks5h => "socks5h",
        }
    }

    /// Whether it is one of SOCKS5's.
    pub fn is_socks(self) -> bool {
        matches!(self, Self::Socks5 | Self::Socks5h)
    }

    fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
            Self::Socks5 | Self::Socks5h => 1080,
        }
    }
}

/// `proxy.server` as read: its scheme, host and port, never credentials.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProxyServer {
    /// Its scheme.
    pub scheme: ProxyScheme,
    /// Its host: a name, an IPv4 address, or an IPv6 address without its
    /// brackets.
    pub host: String,
    /// Its port, the scheme's own when the URL names none.
    pub port: u16,
}

/// Why a `proxy.server` value is refused, as `setting_invalid`'s reason.
const SCHEMES: &str = "expected an `http://`, `https://`, `socks5://` or `socks5h://` URL";

impl ProxyServer {
    /// Reads `text`, a URL such as `http://proxy.example:3128` or
    /// `socks5://127.0.0.1:1080`.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for `setting_invalid`: a scheme other than the
    /// four, `socks4` among them, credentials in the URL, a path, or no
    /// host.
    pub fn parse(text: &str) -> Result<Self, String> {
        let Some((scheme, rest)) = text.split_once("://") else {
            return Err(format!("{SCHEMES}, found `{text}`"));
        };
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "http" => ProxyScheme::Http,
            "https" => ProxyScheme::Https,
            "socks5" => ProxyScheme::Socks5,
            "socks5h" => ProxyScheme::Socks5h,
            "socks4" | "socks4a" | "socks" => {
                return Err(format!(
                    "`{scheme}` carries no password and is not taken: use `socks5://` or `socks5h://`, with `proxy.username` and `proxy.password` for its credentials"
                ));
            }
            _ => return Err(format!("{SCHEMES}, found `{text}`")),
        };
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.contains('@') {
            return Err(
                "a proxy URL never carries credentials: give them as `proxy.username` and `proxy.password`, the password a Secret reference".to_owned(),
            );
        }
        if authority.contains(['/', '?', '#']) {
            return Err(format!(
                "a proxy URL names only a host and a port, found `{text}`"
            ));
        }
        let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
            let Some((host, after)) = bracketed.split_once(']') else {
                return Err(format!("an IPv6 host's `]` is missing in `{text}`"));
            };
            match after.strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None if after.is_empty() => (host, None),
                None => return Err(format!("{SCHEMES}, found `{text}`")),
            }
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        if host.is_empty() {
            return Err(format!("a proxy URL names its host, found `{text}`"));
        }
        let port = match port {
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    format!("a proxy's port is a number from 1 to 65535, found `{port}`")
                })?,
            None => scheme.default_port(),
        };
        Ok(Self {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
        })
    }
}

/// `scheme://host:port`, with the brackets of an IPv6 host.
impl fmt::Display for ProxyServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = self.scheme.as_str();
        if self.host.contains(':') {
            write!(formatter, "{scheme}://[{}]:{}", self.host, self.port)
        } else {
            write!(formatter, "{scheme}://{}:{}", self.host, self.port)
        }
    }
}

/// `proxy.server`'s values: a proxy URL [`ProxyServer::parse`] reads.
pub const PROXY_SERVER_SHAPE: ValueShape = ValueShape {
    name: "ProxyServer",
    check: |value| match value.as_str() {
        Some(text) => ProxyServer::parse(text).map(drop),
        None => Err(format!("{SCHEMES}, found `{value}`")),
    },
    schema: || {
        json!({
            "type": "string",
            "pattern": "^(?i:https?|socks5h?)://[^@/?#]+/?$",
            "description": "A proxy URL, `http://`, `https://`, `socks5://` or `socks5h://`, with a host and an optional port and never credentials, such as `http://proxy.example:3128`.",
        })
    },
    examples: || {
        vec![
            json!("http://proxy.example:3128"),
            json!("socks5h://127.0.0.1:1080"),
        ]
    },
};

fn none() -> Value {
    Value::Null
}

/// `proxy.server` (ADRs 0021 and 0092).
pub const PROXY_SERVER: Setting = Setting {
    name: "proxy.server",
    description: "The proxy every request of the Session goes through, as Playwright's `server`: an `http://`, `https://`, `socks5://` or `socks5h://` URL with a host and a port, such as `http://proxy.example:3128`. None by default: the machine's own address. Set on a Run, every Session of it takes it; set in `run.new_session`'s `settings`, that Session alone; a page or a request has no proxy of its own, so the address a site sees never changes under the same cookies. A URL carrying credentials and `socks4` are refused with `setting_invalid`: give the credentials as `proxy.username` and `proxy.password`. Set only by the program's own code, configuration and an Operator policy: a Recipe may expose it but never sets it, and a Client acting for a model is refused with `setting_locked`, since whoever sets it receives `proxy.password`. On Chromium, a SOCKS5 proxy's credentials go through a loopback relay the engine opens for the Session.",
    kind: ValueKind::Shape(&PROXY_SERVER_SHAPE),
    default: none,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `proxy.bypass` (ADR 0021).
pub const PROXY_BYPASS: Setting = Setting {
    name: "proxy.bypass",
    description: "The hosts the Session reaches without its proxy, as Playwright's `bypass`: a comma-separated list such as `.example.com, chromium.org`, a leading dot naming a domain's subdomains. None by default: every request goes through `proxy.server`. Set as `proxy.server` is.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `proxy.username` (ADRs 0021 and 0092).
pub const PROXY_USERNAME: Setting = Setting {
    name: "proxy.username",
    description: "The username the Session gives `proxy.server`, as Playwright's `username`. None by default. Set as `proxy.server` is.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `proxy.password` (ADRs 0015, 0021 and 0092).
pub const PROXY_PASSWORD: Setting = Setting {
    name: "proxy.password",
    description: "The Secret reference of the password the Session gives `proxy.server` with `proxy.username`, such as `env:PROXY_PASSWORD`: never a value, resolved when the Session opens and never written to a log, a trace, a process's arguments or a file. None by default. Set as `proxy.server` is.",
    kind: ValueKind::SecretReference,
    default: none,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// A proxy that cannot be reached or refuses the tunnel (pitfall H12).
pub const PROXY_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "proxy_unavailable",
    description: "The Session's proxy could not be reached, its TLS failed, or it refused the tunnel or the request with an error of its own: the details name the proxy, never its credentials, and the browser's network error when it named one. It is the proxy's failure, never the site's and never `navigation_failed`: retry later, or through another proxy.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `proxy_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxyUnavailableDetails {
    /// The proxy, `scheme://host:port`, without its credentials.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy: String,
    /// The URL the request went to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The browser's network error, such as
    /// `net::ERR_TUNNEL_CONNECTION_FAILED`, or the proxy's own error its
    /// `Proxy-Status` names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: Option<String>,
}

/// A proxy that refused the Session's credentials (ADR 0053).
pub const PROXY_AUTH_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "proxy_auth_failed",
    description: "The Session's proxy answered `407 Proxy Authentication Required`: the Session gave it no credentials, or it refused those of `proxy.username` and `proxy.password`. The details name the proxy and the username, never the password. Fix the credentials; retrying with the same ones fails the same way.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `proxy_auth_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxyAuthFailedDetails {
    /// The proxy, `scheme://host:port`, without its credentials.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy: String,
    /// The URL the request went to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The username the Session gave, if it gave one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub username: Option<String>,
}
