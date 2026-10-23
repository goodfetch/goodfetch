//! A Session's proxy (spec #577; ADRs 0021, 0053, 0059, 0060 and 0092; ADR
//! 0037 / SA-22): the one proxy every request of a Session goes through,
//! read from the Session's settings when it opens, its password resolved
//! from its Secret reference then and held only as a `SecretString`. The
//! browser takes it as its browser context's `proxyServer`, so a page or a
//! request never has one of its own. An HTTP or HTTPS proxy's credentials
//! answer the browser's `Fetch.authRequired`; a SOCKS5 proxy's, which
//! Chromium cannot give, go through the loopback [`relay`] the Session
//! opens. [`party`] tells the proxy's own failures from the site's.

mod party;
mod relay;

use std::net::{IpAddr, SocketAddr};

use goodfetch_engine::CallError;
use goodfetch_engine::dns::{Dns, HostMapping, Resolution, ResolverConfiguration, Route};
use goodfetch_engine::run::secret_error;
use goodfetch_engine::runs::Run;
use goodfetch_engine::secrets::Purpose;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::page::proxy::{
    PROXY_AUTH_FAILED, PROXY_BYPASS, PROXY_PASSWORD, PROXY_SERVER, PROXY_UNAVAILABLE,
    PROXY_USERNAME, ProxyAuthFailedDetails, ProxyScheme, ProxyServer, ProxyUnavailableDetails,
};
use goodfetch_protocol::settings::{NETWORK_RESOLVE, SETTING_INVALID, SettingInvalidDetails};
use secrecy::SecretString;

pub(crate) use self::party::{Answer, Party, party, proxy_error};
use self::relay::Relay;

/// The rule that sends loopback through the proxy too, as Playwright does,
/// so a Session's every request takes its proxy.
const PROXIED_LOOPBACK: &str = "<-loopback>";

/// A Session's proxy, as it opened.
pub(crate) struct SessionProxy {
    server: ProxyServer,
    bypass: Option<String>,
    username: Option<String>,
    password: Option<SecretString>,
    /// The loopback relay of a SOCKS5 proxy with credentials.
    relay: Option<Relay>,
}

impl std::fmt::Debug for SessionProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionProxy")
            .field("server", &self.server.to_string())
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// `setting_invalid` for `key`, whose value needs `needs`.
fn needs(key: &str, needed: &str) -> CallError {
    CallError::new(
        &SETTING_INVALID,
        format!("`{key}` is set without `{needed}`, which it needs"),
        &SettingInvalidDetails {
            key: key.to_owned(),
            layer: None,
            reason: format!("`{key}` needs `{needed}`"),
        },
    )
}

impl SessionProxy {
    /// The proxy `settings`, a Session's, name for a Session of `run`, if
    /// they name one: its password resolved now, and a SOCKS5 proxy with
    /// credentials behind a loopback relay opened now.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for credentials without `proxy.server` or a
    /// password without a username, the Secret provider's codes when the
    /// password does not resolve, and `proxy_unavailable` when the relay
    /// cannot open or a SOCKS5 proxy's host has no address.
    pub(crate) async fn open(settings: &Resolved, run: &Run) -> Result<Option<Self>, CallError> {
        let text = |key: &str| settings.value(key).as_str().map(str::to_owned);
        let username = text(PROXY_USERNAME.name);
        let reference = settings.secret_reference(PROXY_PASSWORD.name);
        let Some(server) = text(PROXY_SERVER.name) else {
            if username.is_some() {
                return Err(needs(PROXY_USERNAME.name, PROXY_SERVER.name));
            }
            if reference.is_some() {
                return Err(needs(PROXY_PASSWORD.name, PROXY_SERVER.name));
            }
            return Ok(None);
        };
        let server = ProxyServer::parse(&server)
            .expect("invariant: `proxy.server`'s value was checked against its kind");
        let password = match (&username, reference) {
            (None, Some(_)) => return Err(needs(PROXY_PASSWORD.name, PROXY_USERNAME.name)),
            (_, None) => None,
            (Some(_), Some(reference)) => {
                let layer = settings
                    .get(PROXY_PASSWORD.name)
                    .map(|effective| effective.layer);
                let purpose = Purpose {
                    placeholder: PROXY_PASSWORD.name,
                    domain: Some(&server.host),
                    domains: &[],
                    layer,
                    run_id: Some(run.id()),
                };
                Some(
                    run.secrets()
                        .resolve(&reference, &purpose)
                        .await
                        .map_err(|error| secret_error(&error))?,
                )
            }
        };
        let relay = match (&username, server.scheme.is_socks()) {
            (Some(username), true) => {
                let address = address_of(&server, settings, run).await?;
                let password = password.clone().unwrap_or_else(|| SecretString::from(""));
                let relay = Relay::open(run, address, username.clone(), password)
                    .await
                    .map_err(|error| unavailable(&server, "", &error.to_string()))?;
                Some(relay)
            }
            _ => None,
        };
        Ok(Some(Self {
            server,
            bypass: text(PROXY_BYPASS.name),
            username,
            password,
            relay,
        }))
    }

    /// The `proxyServer` the browser context takes: the relay's address for
    /// a SOCKS5 proxy with credentials, and Chromium's `socks5` for
    /// `socks5h`, whose SOCKS5 proxies resolve every name they are given.
    pub(crate) fn browser_server(&self) -> String {
        if let Some(relay) = &self.relay {
            return format!("socks5://{}", relay.address());
        }
        match self.server.scheme {
            ProxyScheme::Socks5h => ProxyServer {
                scheme: ProxyScheme::Socks5,
                ..self.server.clone()
            }
            .to_string(),
            ProxyScheme::Http | ProxyScheme::Https | ProxyScheme::Socks5 => self.server.to_string(),
        }
    }

    /// The `proxyBypassList` the browser context takes, as Playwright builds
    /// it from `proxy.bypass`: each entry trimmed, a leading dot made a
    /// wildcard, and loopback sent through the proxy too.
    pub(crate) fn browser_bypass(&self) -> String {
        let mut rules: Vec<String> = self
            .bypass
            .iter()
            .flat_map(|bypass| bypass.split(','))
            .map(str::trim)
            .filter(|rule| !rule.is_empty())
            .map(|rule| {
                if rule.starts_with('.') {
                    format!("*{rule}")
                } else {
                    rule.to_owned()
                }
            })
            .collect();
        if !rules.iter().any(|rule| rule == PROXIED_LOOPBACK) {
            rules.push(PROXIED_LOOPBACK.to_owned());
        }
        rules.join(";")
    }

    /// Whether the browser asks the engine for this proxy's credentials:
    /// an HTTP or HTTPS proxy's, whose 407 then reaches the engine.
    pub(crate) fn answers_authentication(&self) -> bool {
        !self.server.scheme.is_socks()
    }

    /// The credentials the engine answers the proxy's 407 with, if it has
    /// any.
    pub(crate) fn credentials(&self) -> Option<(&str, Option<&SecretString>)> {
        self.username
            .as_deref()
            .map(|username| (username, self.password.as_ref()))
    }

    /// `proxy_unavailable` for a request to `url` that met `error`, the
    /// browser's network error or the proxy's own.
    pub(crate) fn unavailable(&self, url: &str, error: &str) -> CallError {
        unavailable(&self.server, url, error)
    }

    /// `proxy_auth_failed` for a request to `url` the proxy answered with
    /// a 407.
    pub(crate) fn auth_failed(&self, url: &str) -> CallError {
        let given = match &self.username {
            Some(username) => format!("refused the credentials of `{username}`"),
            None => "asked for credentials the Session does not have".to_owned(),
        };
        CallError::new(
            &PROXY_AUTH_FAILED,
            format!(
                "the proxy {} {given}: set `proxy.username` and `proxy.password`",
                self.server
            ),
            &ProxyAuthFailedDetails {
                proxy: self.server.to_string(),
                url: url.to_owned(),
                username: self.username.clone(),
            },
        )
    }

    /// The Error code a request to `url` that failed with the browser's
    /// network error `error` raises, when it is the proxy's own failure:
    /// `proxy_auth_failed` when the proxy refused the relay's credentials,
    /// `proxy_unavailable` otherwise.
    pub(crate) fn failure(&self, url: &str, error: &str) -> Option<CallError> {
        if party(Answer::Failure { error }) != Party::Proxy {
            return None;
        }
        Some(if self.refused_credentials() {
            self.auth_failed(url)
        } else {
            self.unavailable(url, error)
        })
    }

    /// Whether the proxy refused the Session's credentials on a connection
    /// the relay opened.
    pub(crate) fn refused_credentials(&self) -> bool {
        self.relay.as_ref().is_some_and(Relay::refused_credentials)
    }
}

/// The address of `server`'s host: an IP address as it is, a name from the
/// Run's `network.resolve` mapping first, then the resolver (ADR 0059).
async fn address_of(
    server: &ProxyServer,
    settings: &Resolved,
    run: &Run,
) -> Result<SocketAddr, CallError> {
    if let Ok(ip) = server.host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, server.port));
    }
    let mapping = settings.hosts(NETWORK_RESOLVE.name);
    let configuration = ResolverConfiguration::system()
        .map_err(|error| unavailable(server, "", &error.to_string()))?;
    let dns = Dns::new(
        HostMapping::new(
            mapping
                .iter()
                .map(|(host, addresses)| (host.as_str(), addresses.clone())),
        ),
        configuration,
        run.clock(),
    );
    match dns.lookup(&server.host, Route::Direct).await {
        Ok(Resolution::Addresses { addresses, .. }) => Ok(SocketAddr::new(
            *addresses
                .first()
                .expect("invariant: a resolution's addresses are never empty"),
            server.port,
        )),
        Ok(Resolution::ByProxy) => Err(unavailable(server, "", "the proxy's host has no address")),
        Err(error) => Err(unavailable(server, "", &error.to_string())),
    }
}

/// `proxy_unavailable` for `server`, met by a request to `url`.
fn unavailable(server: &ProxyServer, url: &str, error: &str) -> CallError {
    CallError::new(
        &PROXY_UNAVAILABLE,
        format!("the proxy {server} is unavailable: {error}"),
        &ProxyUnavailableDetails {
            proxy: server.to_string(),
            url: url.to_owned(),
            error: Some(error.to_owned()),
        },
    )
}
