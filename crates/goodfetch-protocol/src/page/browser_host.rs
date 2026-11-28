//! Browser host configuration and startup announcement (ADRs 0022, 0059 and
//! 0096; ticket #753): `browser.host` selects a Browser host beside a browser
//! in place of launching one locally, so the same program runs locally or
//! remotely by configuration alone.

use std::net::{IpAddr, SocketAddr};

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ARRIVED_IN;
use crate::catalog::{Authority, Product, Setting, ValueKind, ValueShape};
use crate::connection::ProductVersion;
use crate::secret_reference::SecretReference;

/// What `browser.host` holds: the Browser host a Run's Sessions drive their
/// browsers through (ADRs 0022 and 0096).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserHostSetting {
    /// The Browser host's address, such as `127.0.0.1:9444`,
    /// `ws://127.0.0.1:9444` or `wss://browser-host.example:9444`. A
    /// non-loopback address without TLS (`wss://`) is refused, as `daemon run`
    /// refuses one (ADRs 0017 and 0022).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: String,
    /// Path to the file holding the Browser host's authentication token (what
    /// `browser-host --token-file` reads).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token_file: Option<String>,
    /// Secret reference holding the Browser host's authentication token (such
    /// as `env:BROWSER_HOST_TOKEN`), never a bare token value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: Option<String>,
}

impl crate::catalog::SettingJson for BrowserHostSetting {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// A parsed Browser host endpoint address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserHostAddress {
    /// Host or IP literal (without brackets for IPv6).
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Whether TLS (`wss://`) is requested.
    pub tls: bool,
}

impl BrowserHostAddress {
    /// Parses a Browser host address (`host:port`, `ws://host:port`, or
    /// `wss://host:port`) and refuses a non-loopback address without TLS.
    ///
    /// # Errors
    ///
    /// Human-readable reason for `setting_invalid` or CLI usage refusal.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        let (tls, rest) = if let Some(rest) = trimmed.strip_prefix("wss://") {
            (true, rest)
        } else if let Some(rest) = trimmed.strip_prefix("ws://") {
            (false, rest)
        } else if trimmed.contains("://") {
            return Err(format!(
                "`{raw}` names no Browser host: give `<host>:<port>`, `ws://<host>:<port>` or `wss://<host>:<port>`"
            ));
        } else {
            (false, trimmed)
        };
        let authority = rest.trim_end_matches('/');
        if authority.contains(['/', '?', '#', '@']) {
            return Err(format!(
                "`{raw}` must hold only a host and port, with no path, query or credentials"
            ));
        }
        let (host, port) = split_host_port(authority).ok_or_else(|| {
            format!("`{raw}` has no valid host and port, such as `127.0.0.1:9444`")
        })?;
        if !tls && !is_loopback_host(&host) {
            return Err(format!(
                "`{raw}` names a non-loopback address without TLS: use loopback (`127.0.0.1`, `[::1]`, `localhost`) or configure TLS (`wss://`)"
            ));
        }
        Ok(Self { host, port, tls })
    }

    /// Resolves to a loopback `SocketAddr` when `host` is a loopback literal or
    /// `localhost`.
    #[must_use]
    pub fn loopback_socket_addr(&self) -> Option<SocketAddr> {
        if self.host.eq_ignore_ascii_case("localhost") {
            return Some(SocketAddr::new(
                IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                self.port,
            ));
        }
        let ip: IpAddr = self.host.parse().ok()?;
        ip.is_loopback().then(|| SocketAddr::new(ip, self.port))
    }
}

/// Splits `host:port` or `[ipv6]:port` into `(host, port)`.
fn split_host_port(authority: &str) -> Option<(String, u16)> {
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, after) = bracketed.split_once(']')?;
        let port = after.strip_prefix(':')?.parse::<u16>().ok()?;
        (!host.is_empty()).then(|| (host.to_owned(), port))
    } else {
        let (host, port) = authority.rsplit_once(':')?;
        if host.is_empty() || host.contains(':') {
            return None;
        }
        let port = port.parse::<u16>().ok()?;
        Some((host.to_owned(), port))
    }
}

/// Whether `host` is a loopback IP address or `localhost`.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Validates a [`BrowserHostSetting`] value.
fn check_browser_host(value: &Value) -> Result<(), String> {
    let config: BrowserHostSetting = serde_json::from_value(value.clone())
        .map_err(|error| format!("expected a Browser host configuration: {error}"))?;
    BrowserHostAddress::parse(&config.address)?;
    match (&config.token_file, &config.token) {
        (Some(_), Some(_)) => Err(
            "set either `token_file` or `token` on `browser.host`, not both".to_owned(),
        ),
        (None, None) => Err(
            "`browser.host` requires `token_file` or a `token` Secret reference (such as `env:BROWSER_HOST_TOKEN`)".to_owned(),
        ),
        (Some(path), None) => {
            if path.trim().is_empty() {
                Err("`browser.host.token_file` must not be empty".to_owned())
            } else {
                Ok(())
            }
        }
        (None, Some(reference)) => SecretReference::parse(reference)
            .map(drop)
            .map_err(|error| {
                format!(
                    "`browser.host.token` must be a Secret reference (such as `env:BROWSER_HOST_TOKEN`), never a bare token: {error}"
                )
            }),
    }
}

/// `browser.host`'s values: a [`BrowserHostSetting`].
pub const BROWSER_HOST_SHAPE: ValueShape = ValueShape {
    name: "BrowserHostSetting",
    check: check_browser_host,
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<BrowserHostSetting>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    examples: || {
        vec![
            json!({
                "address": "127.0.0.1:9444",
                "token_file": "/run/goodfetch/browser-host.token",
            }),
            json!({
                "address": "wss://browser-host.example:9444",
                "token": "env:BROWSER_HOST_TOKEN",
            }),
        ]
    },
};

/// `browser.host` (ADRs 0022 and 0096; ticket #753).
pub const BROWSER_HOST: Setting = Setting {
    name: "browser.host",
    description: "The Browser host a Run's Sessions drive their browsers through (ADRs 0022 and 0096): `{ address, token_file }` or `{ address, token }`, where `token` is a Secret reference such as `env:BROWSER_HOST_TOKEN`. Left unset (`null`), Sessions launch browsers locally; changing this setting alone moves the same program between a local browser and a Browser host. A non-loopback address without TLS (`wss://`) is refused. Only the program's own code, configuration and Operator policy set it.",
    kind: ValueKind::Shape(&BROWSER_HOST_SHAPE),
    default: || Value::Null,
    clearable: true,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The JSON line `browser-host --listen` prints to stdout once its listener is
/// bound, reporting its bound address (including when listening on port 0,
/// ADR 0059).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserHostReady {
    /// The bound listening address, such as `127.0.0.1:54321`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: String,
    /// The Browser host's process ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pid: u32,
    /// The Product of the executable running the Browser host (ADR 0095).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// The Product version of the executable running the Browser host.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProductVersion,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{BROWSER_HOST_SHAPE, BrowserHostAddress};

    #[test]
    fn loopback_addresses_are_accepted_without_tls() {
        for raw in [
            "127.0.0.1:9444",
            "ws://127.0.0.1:9444",
            "localhost:9444",
            "[::1]:9444",
            "ws://[::1]:9444",
        ] {
            let parsed = BrowserHostAddress::parse(raw).unwrap();
            assert!(!parsed.tls);
            assert!(parsed.loopback_socket_addr().is_some());
        }
    }

    #[test]
    fn non_loopback_address_without_tls_is_refused() {
        for raw in [
            "192.0.2.10:9444",
            "ws://192.0.2.10:9444",
            "browser-host.example:9444",
            "0.0.0.0:9444",
        ] {
            assert!(
                BrowserHostAddress::parse(raw).is_err(),
                "should refuse {raw}"
            );
        }
        let tls = BrowserHostAddress::parse("wss://browser-host.example:9444").unwrap();
        assert!(tls.tls);
        assert_eq!(tls.host, "browser-host.example");
        assert_eq!(tls.port, 9444);
    }

    #[test]
    fn browser_host_setting_requires_token_file_or_secret_reference() {
        assert!(
            (BROWSER_HOST_SHAPE.check)(&json!({
                "address": "127.0.0.1:9444",
                "token_file": "/tmp/token",
            }))
            .is_ok()
        );
        assert!(
            (BROWSER_HOST_SHAPE.check)(&json!({
                "address": "127.0.0.1:9444",
                "token": "env:BROWSER_HOST_TOKEN",
            }))
            .is_ok()
        );
        assert!(
            (BROWSER_HOST_SHAPE.check)(&json!({
                "address": "127.0.0.1:9444",
            }))
            .is_err()
        );
        assert!(
            (BROWSER_HOST_SHAPE.check)(&json!({
                "address": "127.0.0.1:9444",
                "token": "bare-secret-value",
            }))
            .is_err()
        );
    }
}
