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

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ARRIVED_IN;
use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, PriceValue, Setting, ValueKind, ValueShape,
};
use crate::secret_reference::SecretReference;

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
    fixed_for_run: false,
    unbuilt: None,
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
    fixed_for_run: false,
    unbuilt: None,
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
    fixed_for_run: false,
    unbuilt: None,
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
    fixed_for_run: false,
    unbuilt: None,
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
    /// The URL the request went to; none when the proxy failed the
    /// Session's open, before any request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// The browser's network error, such as
    /// `net::ERR_TUNNEL_CONNECTION_FAILED`, or the proxy's own error its
    /// `Proxy-Status` names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: Option<String>,
}

/// The Session's loopback proxy relay that could not open (ADR 0037 /
/// SA-22; spec #577's review, L76): the engine's own failure, never the
/// proxy's.
pub const PROXY_RELAY_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "proxy_relay_unavailable",
    description: "The engine could not open the loopback relay a Session's proxy goes through, such as when the machine has no free local port: the proxy itself was never tried. The details name the proxy, never its credentials, and the local error. Retry once the machine frees ports or file descriptors.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `proxy_relay_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxyRelayUnavailableDetails {
    /// The proxy, `scheme://host:port`, without its credentials.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy: String,
    /// The local error that kept the relay from opening.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: String,
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

/// A Proxy provider that answered a Session's request through its proxy
/// with a failure of its own (ADR 0127's translation point of each
/// built-in adapter, spec #599; area 7's code, spec #596).
pub const PROXY_PROVIDER_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "proxy_provider_unavailable",
    description: "The Session's Proxy provider answered through its proxy with a failure of its own: its traffic quota or balance is spent (`quota_exhausted`), the Session's sticky exit is gone (`sticky_session_expired`), or a failure Goodfetch does not recognize (`unrecognized`, whose text is logged, never returned). The details name the provider, the proxy and the failure, never the credentials. A provider's refusal of the credentials is `proxy_auth_failed` instead. Retry later, or open a new Session for a new exit.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What a Proxy provider's failure was, as its adapter read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProviderFailure {
    /// The account's traffic quota or balance is spent.
    QuotaExhausted,
    /// The Session's sticky exit is gone.
    StickySessionExpired,
    /// A failure the adapter does not recognize.
    Unrecognized,
}

/// What `proxy_provider_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxyProviderUnavailableDetails {
    /// The Proxy provider, as `proxy.provider` names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provider: String,
    /// The proxy, `scheme://host:port`, without its credentials.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy: String,
    /// The URL the request went to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// The failure.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: ProxyProviderFailure,
}

/// A proxy kind, the pool a Session's proxy is drawn from (ADR 0021), in
/// the order of the proxy tiers, cheapest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProxyKind {
    /// A data centre's addresses.
    Datacenter,
    /// Addresses an ISP assigns, hosted in a data centre.
    Isp,
    /// Home connections' addresses.
    Residential,
    /// Mobile networks' addresses.
    Mobile,
}

/// The proxy kinds' names, as `proxy.kind` takes them, cheapest first.
pub const PROXY_KINDS: &[&str] = &["datacenter", "isp", "residential", "mobile"];

impl ProxyKind {
    /// Every kind, cheapest first.
    pub const ALL: [Self; 4] = [Self::Datacenter, Self::Isp, Self::Residential, Self::Mobile];

    /// Its name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Datacenter => "datacenter",
            Self::Isp => "isp",
            Self::Residential => "residential",
            Self::Mobile => "mobile",
        }
    }

    /// The kind `name` names, if it names one.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

impl fmt::Display for ProxyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An ISO 3166-1 alpha-2 country code, such as `US`, as `proxy.country`
/// and a `list` server's `country` give it, in either case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CountryCode(String);

/// ISO 3166-1's officially assigned alpha-2 codes, the 249 countries and
/// territories, in order. A code it reserves without assigning, such as
/// `EU` or `UK`, or one it never assigned, such as `ZZ`, names no country
/// a proxy exit is in.
const ASSIGNED_COUNTRIES: [&str; 249] = [
    "AD", "AE", "AF", "AG", "AI", "AL", "AM", "AO", "AQ", "AR", "AS", "AT", "AU", "AW", "AX", "AZ",
    "BA", "BB", "BD", "BE", "BF", "BG", "BH", "BI", "BJ", "BL", "BM", "BN", "BO", "BQ", "BR", "BS",
    "BT", "BV", "BW", "BY", "BZ", "CA", "CC", "CD", "CF", "CG", "CH", "CI", "CK", "CL", "CM", "CN",
    "CO", "CR", "CU", "CV", "CW", "CX", "CY", "CZ", "DE", "DJ", "DK", "DM", "DO", "DZ", "EC", "EE",
    "EG", "EH", "ER", "ES", "ET", "FI", "FJ", "FK", "FM", "FO", "FR", "GA", "GB", "GD", "GE", "GF",
    "GG", "GH", "GI", "GL", "GM", "GN", "GP", "GQ", "GR", "GS", "GT", "GU", "GW", "GY", "HK", "HM",
    "HN", "HR", "HT", "HU", "ID", "IE", "IL", "IM", "IN", "IO", "IQ", "IR", "IS", "IT", "JE", "JM",
    "JO", "JP", "KE", "KG", "KH", "KI", "KM", "KN", "KP", "KR", "KW", "KY", "KZ", "LA", "LB", "LC",
    "LI", "LK", "LR", "LS", "LT", "LU", "LV", "LY", "MA", "MC", "MD", "ME", "MF", "MG", "MH", "MK",
    "ML", "MM", "MN", "MO", "MP", "MQ", "MR", "MS", "MT", "MU", "MV", "MW", "MX", "MY", "MZ", "NA",
    "NC", "NE", "NF", "NG", "NI", "NL", "NO", "NP", "NR", "NU", "NZ", "OM", "PA", "PE", "PF", "PG",
    "PH", "PK", "PL", "PM", "PN", "PR", "PS", "PT", "PW", "PY", "QA", "RE", "RO", "RS", "RU", "RW",
    "SA", "SB", "SC", "SD", "SE", "SG", "SH", "SI", "SJ", "SK", "SL", "SM", "SN", "SO", "SR", "SS",
    "ST", "SV", "SX", "SY", "SZ", "TC", "TD", "TF", "TG", "TH", "TJ", "TK", "TL", "TM", "TN", "TO",
    "TR", "TT", "TV", "TW", "TZ", "UA", "UG", "UM", "US", "UY", "UZ", "VA", "VC", "VE", "VG", "VI",
    "VN", "VU", "WF", "WS", "YE", "YT", "ZA", "ZM", "ZW",
];

impl CountryCode {
    /// Reads `text`: one of ISO 3166-1's assigned alpha-2 codes, in either
    /// case, kept in upper case (the deep review's 599-3: a hosted Proxy
    /// provider offers every country, so no other check stands between a
    /// code that names none and the provider's own refusal).
    ///
    /// # Errors
    ///
    /// What is wrong with it, for `setting_invalid`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let code = text.to_ascii_uppercase();
        if ASSIGNED_COUNTRIES.binary_search(&code.as_str()).is_ok() {
            Ok(Self(code))
        } else {
            Err(format!(
                "expected an assigned ISO 3166-1 alpha-2 country code such as `US` or `GB`, found `{text}`"
            ))
        }
    }

    /// Its two letters, in upper case.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CountryCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// `proxy.country`'s values: a country code [`CountryCode::parse`] reads.
pub const COUNTRY_CODE_SHAPE: ValueShape = ValueShape {
    name: "CountryCode",
    check: |value| match value.as_str() {
        Some(text) => CountryCode::parse(text).map(drop),
        None => Err(format!(
            "expected an ISO 3166-1 alpha-2 country code such as `US`, found `{value}`"
        )),
    },
    schema: || {
        json!({
            "type": "string",
            "pattern": "^[A-Za-z]{2}$",
            "description": "An ISO 3166-1 alpha-2 country code, such as `US`.",
        })
    },
    examples: || vec![json!("US")],
};

/// One server of the `list` Proxy provider: the proxy, its kind and the
/// country its exit is in, if the user names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListServer {
    /// The proxy.
    pub server: ProxyServer,
    /// Its proxy kind.
    pub kind: ProxyKind,
    /// Its exit's country.
    pub country: Option<CountryCode>,
}

/// One entry of `proxy.providers`, as read: what configuration says of one
/// Proxy provider.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProxyProviderConfiguration {
    /// The account's username, which a provider's sticky username builds
    /// on.
    pub username: Option<String>,
    /// The Secret reference of the account's password.
    pub password: Option<SecretReference>,
    /// The provider's proxy, for a provider that has one.
    pub endpoint: Option<ProxyServer>,
    /// The user's own servers, for `list`.
    pub servers: Vec<ListServer>,
    /// The price of a GB (10^9 bytes) through it, for a metered proxy.
    pub price: Option<PriceValue>,
    /// The zone of each proxy kind, for `bright_data`, whose customers name
    /// their zones; a kind it leaves out is the zone of the kind's own name.
    pub zones: BTreeMap<ProxyKind, String>,
}

/// The fields an entry of `proxy.providers` takes.
const PROVIDER_FIELDS: [&str; 6] = [
    "username", "password", "endpoint", "servers", "price", "zones",
];

/// The fields a `list` server takes.
const SERVER_FIELDS: [&str; 3] = ["server", "kind", "country"];

impl ProxyProviderConfiguration {
    /// Reads `value`, one entry of `proxy.providers`.
    ///
    /// # Errors
    ///
    /// What is wrong with it, for `setting_invalid`.
    pub fn read(value: &Value) -> Result<Self, String> {
        let Some(entry) = value.as_object() else {
            return Err(format!(
                "a Proxy provider's entry is a table of {}, found `{value}`",
                PROVIDER_FIELDS.join(", ")
            ));
        };
        if let Some(unknown) = entry
            .keys()
            .find(|field| !PROVIDER_FIELDS.contains(&field.as_str()))
        {
            return Err(format!(
                "a Proxy provider's entry takes {}, not `{unknown}`",
                PROVIDER_FIELDS.join(", ")
            ));
        }
        let text = |field: &str| -> Result<Option<&str>, String> {
            match entry.get(field) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text)),
                Some(other) => Err(format!("`{field}` is text, found `{other}`")),
            }
        };
        let password = text("password")?
            .map(|reference| {
                SecretReference::parse(reference).map_err(|error| {
                    format!(
                        "`password` is a Secret reference such as `env:NAME`: {error}; a value is never taken in its place"
                    )
                })
            })
            .transpose()?;
        let endpoint = text("endpoint")?.map(ProxyServer::parse).transpose()?;
        if let Some(endpoint) = endpoint.as_ref()
            && endpoint.scheme == ProxyScheme::Https
        {
            return Err(format!(
                "a Proxy provider's `endpoint` is `http://`, `socks5://` or `socks5h://`: the Session's loopback relay speaks no TLS to it, so its credentials would cross in clear, found `{endpoint}`"
            ));
        }
        let price = text("price")?
            .map(|price| {
                PriceValue::parse(price).ok_or_else(|| {
                    format!(
                        "`price` is the price of a GB as an amount and its currency, such as `5 USD`, found `{price}`"
                    )
                })
            })
            .transpose()?;
        let zones = match entry.get("zones") {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(Value::Object(zones)) => zones
                .iter()
                .map(|(kind, zone)| zone_of(kind, zone))
                .collect::<Result<_, _>>()?,
            Some(other) => {
                return Err(format!(
                    "`zones` is a table of a zone name by proxy kind, such as `{{ residential = \"residential_proxy1\" }}`, found `{other}`"
                ));
            }
        };
        let servers = match entry.get("servers") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(servers)) => servers
                .iter()
                .map(ListServer::read)
                .collect::<Result<_, _>>()?,
            Some(other) => return Err(format!("`servers` is a list, found `{other}`")),
        };
        Ok(Self {
            username: text("username")?.map(str::to_owned),
            password,
            endpoint,
            servers,
            price,
            zones,
        })
    }
}

/// One entry of a provider's `zones`: the proxy kind `kind` names, and its
/// zone's name `zone`, letters, digits and `_`, since a `-` parts Bright
/// Data's username.
fn zone_of(kind: &str, zone: &Value) -> Result<(ProxyKind, String), String> {
    let kind = ProxyKind::named(kind).ok_or_else(|| {
        format!(
            "a key of `zones` is a proxy kind, one of {}, found `{kind}`",
            PROXY_KINDS.join(", ")
        )
    })?;
    match zone.as_str() {
        Some(name)
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
        {
            Ok((kind, name.to_owned()))
        }
        Some(name) => Err(format!(
            "a zone's name in `zones` is letters, digits and `_`, as Bright Data names zones, found `{name}`"
        )),
        None => Err(format!("a zone's name in `zones` is text, found `{zone}`")),
    }
}

impl ListServer {
    fn read(value: &Value) -> Result<Self, String> {
        let Some(entry) = value.as_object() else {
            return Err(format!(
                "a `list` server is a table of {}, found `{value}`",
                SERVER_FIELDS.join(", ")
            ));
        };
        if let Some(unknown) = entry
            .keys()
            .find(|field| !SERVER_FIELDS.contains(&field.as_str()))
        {
            return Err(format!(
                "a `list` server takes {}, not `{unknown}`",
                SERVER_FIELDS.join(", ")
            ));
        }
        let server = match entry.get("server").and_then(Value::as_str) {
            Some(server) => ProxyServer::parse(server)?,
            None => return Err("a `list` server names its `server`, a proxy URL".to_owned()),
        };
        if server.scheme == ProxyScheme::Https {
            return Err(format!(
                "a `list` server is `http://`, `socks5://` or `socks5h://`: its bytes are counted on the Session's loopback relay, which an `https://` proxy's TLS does not pass, found `{server}`"
            ));
        }
        let kind = match entry.get("kind").and_then(Value::as_str) {
            Some(kind) => ProxyKind::named(kind).ok_or_else(|| {
                format!(
                    "a `list` server's `kind` is one of {}, found `{kind}`",
                    PROXY_KINDS.join(", ")
                )
            })?,
            None => {
                return Err(format!(
                    "a `list` server names its `kind`, one of {}",
                    PROXY_KINDS.join(", ")
                ));
            }
        };
        let country = match entry.get("country") {
            None | Some(Value::Null) => None,
            Some(Value::String(country)) => Some(CountryCode::parse(country)?),
            Some(other) => return Err(format!("`country` is text, found `{other}`")),
        };
        Ok(Self {
            server,
            kind,
            country,
        })
    }
}

/// An entry of `proxy.providers`'s values.
pub const PROXY_PROVIDER_SHAPE: ValueShape = ValueShape {
    name: "ProxyProviderConfiguration",
    check: |value| ProxyProviderConfiguration::read(value).map(drop),
    schema: || {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "username": { "type": "string", "description": "The account's username." },
                "password": {
                    "type": "string",
                    "pattern": crate::secret_reference::REFERENCE_PATTERN,
                    "description": "The Secret reference of the account's password, such as `env:PROXY_PASSWORD`.",
                },
                "endpoint": { "type": "string", "description": "The provider's proxy URL." },
                "servers": {
                    "type": "array",
                    "description": "The user's own servers, for `list` only.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["server", "kind"],
                        "properties": {
                            "server": { "type": "string", "pattern": "^(?i:http|socks5h?)://[^@/?#]+/?$" },
                            "kind": { "enum": PROXY_KINDS },
                            "country": { "type": "string", "pattern": "^[A-Za-z]{2}$" },
                        },
                    },
                },
                "price": {
                    "type": "string",
                    "pattern": "^(0|[1-9][0-9]*)(\\.[0-9]+)? [A-Z]{3}$",
                    "description": "The price of a GB (10^9 bytes) through it, as an amount and its currency.",
                },
                "zones": {
                    "type": "object",
                    "description": "The zone of each proxy kind, for `bright_data` only: a kind left out is the zone of its own name.",
                    "propertyNames": { "enum": PROXY_KINDS },
                    "additionalProperties": { "type": "string", "pattern": "^[A-Za-z0-9_]+$" },
                },
            },
        })
    },
    examples: || {
        vec![json!({
            "username": "account",
            "password": "env:PROXY_PASSWORD",
            "endpoint": "http://proxy.example:8000",
            "servers": [
                { "server": "http://203.0.113.7:3128", "kind": "datacenter", "country": "US" },
            ],
            "price": "5 USD",
            "zones": { "residential": "residential_proxy1" },
        })]
    },
};

/// `proxy.provider` (ADRs 0021, 0092 and 0129).
pub const PROXY_PROVIDER: Setting = Setting {
    name: "proxy.provider",
    description: "The Proxy provider each Session of the Run, or the Session it is set on, draws its one proxy from when it opens, fixed for the Session's life: `bright_data`, `oxylabs` or `apify`, whose adapters build the provider's sticky-session username from the account in `proxy.providers.<name>` and ask for the provider's longest sticky lifetime, or `list`, the user's own servers in `proxy.providers.list.servers`. None by default. A Session that resolves both `proxy.server` and `proxy.provider` is refused with `setting_invalid`. Set as `proxy.server` is: a Recipe may expose it but never sets it, and a Client acting for a model is refused with `setting_locked`.",
    kind: ValueKind::Text,
    default: none,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `proxy.kind` (ADR 0021).
pub const PROXY_KIND: Setting = Setting {
    name: "proxy.kind",
    description: "The proxy kind a Session's proxy is drawn from, `datacenter`, `isp`, `residential` or `mobile`: the provider's default when unset, for `list` the kind of its first server. A kind the provider does not offer is refused with `setting_invalid` when the Session opens. A Recipe may set it; a Client acting for a model is refused with `setting_locked`, since the kinds' prices differ by an order of magnitude and the person pays.",
    kind: ValueKind::Choice(PROXY_KINDS),
    default: none,
    clearable: true,
    authority: Authority::NotByModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `proxy.country` (ADRs 0021 and 0092).
pub const PROXY_COUNTRY: Setting = Setting {
    name: "proxy.country",
    description: "The country, an ISO 3166-1 alpha-2 code such as `US`, a Session's proxy exit is in. None by default: any country the provider offers. A country the provider does not offer for the Session's proxy kind is refused with `setting_invalid` when the Session opens. A Recipe never sets it (`setting_locked`).",
    kind: ValueKind::Shape(&COUNTRY_CODE_SHAPE),
    default: none,
    clearable: true,
    authority: Authority::NotByRecipe,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// `proxy.sticky_lifetime` (ADR 0021).
pub const PROXY_STICKY_LIFETIME: Setting = Setting {
    name: "proxy.sticky_lifetime",
    description: "How long, in milliseconds, a Session asks its provider to keep its proxy exit: the provider's longest when unset. A Session that outlives it is named by the Run report's `proxy_exit_may_change` warning; the engine never looks the exit up. A `list` server's exit never changes.",
    kind: ValueKind::Duration,
    default: none,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

fn no_providers() -> Value {
    json!({})
}

/// Whether `value`, a `proxy.providers` table, names no provider's
/// `price`: the only values project configuration gives it (O12 of the
/// deep review, accepted by the owner on 2026-10-28).
fn names_no_price(value: &Value) -> bool {
    value
        .as_object()
        .into_iter()
        .flat_map(|providers| providers.values())
        .all(|entry| entry.get("price").is_none_or(Value::is_null))
}

/// `proxy.providers` (ADRs 0021, 0041 and 0101).
pub const PROXY_PROVIDERS: Setting = Setting {
    name: "proxy.providers",
    description: "What each Proxy provider takes, by name, such as `proxy.providers.list`: `username`, the account's; `password`, a Secret reference resolved when a Session opens; `endpoint`, the provider's proxy URL in place of its public one (`http://brd.superproxy.io:33335` for `bright_data`, `http://pr.oxylabs.io:7777` for `oxylabs`, `http://proxy.apify.com:8000` for `apify`), an `http://`, `socks5://` or `socks5h://` URL, never `https://`, which the Session's loopback relay speaks no TLS to; `servers`, for `list` only, the user's own proxies, each a table of `server` (an `http://`, `socks5://` or `socks5h://` URL), `kind` and an optional `country`; `zones`, for `bright_data` only, the name of each proxy kind's zone as its customer named it, such as `{ residential = \"residential_proxy1\" }`, a kind it leaves out being the zone of the kind's own name, and the entry's `password` every zone's; and `price`, the price of a GB (10^9 bytes) through the provider, such as `5 USD`, which a currency Limit on `proxy_bytes` charges: under such a Limit a Session of an unpriced provider, or one priced in another currency, is refused with `setting_invalid`. Empty by default. Set only in an Operator policy and user and project configuration: a Recipe's or a run's value is `setting_locked`, so no Client lowers a price, and so is a project configuration's entry that names a `price`, which only user configuration and an Operator policy set, since a currency Limit trusts it.",
    kind: ValueKind::Map(&ValueKind::Shape(&PROXY_PROVIDER_SHAPE)),
    default: no_providers,
    clearable: false,
    authority: Authority::ConfigurationProjectLimited(names_no_price),
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    arrived_in: ARRIVED_IN,
};

/// A Session's proxy's entries: its settings and its Error codes.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        PROXY_UNAVAILABLE,
        PROXY_RELAY_UNAVAILABLE,
        PROXY_AUTH_FAILED,
        PROXY_PROVIDER_UNAVAILABLE,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        PROXY_SERVER,
        PROXY_BYPASS,
        PROXY_USERNAME,
        PROXY_PASSWORD,
        PROXY_PROVIDER,
        PROXY_KIND,
        PROXY_COUNTRY,
        PROXY_STICKY_LIFETIME,
        PROXY_PROVIDERS,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
