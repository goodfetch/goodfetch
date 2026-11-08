//! A Session's proxy (spec #577; ADRs 0021, 0053, 0059, 0060 and 0092; ADR
//! 0037 / SA-22): the one proxy every request of a Session goes through,
//! read from the Session's settings when it opens, its password resolved
//! from its Secret reference then and held only as a `SecretString`. The
//! browser takes it as its browser context's `proxyServer`, so a page or a
//! request never has one of its own. An HTTP or HTTPS proxy's credentials
//! answer the browser's `Fetch.authRequired`; a SOCKS5 proxy's, which
//! Chromium cannot give, go through the loopback [`relay`] the Session
//! opens, an HTTP proxy of its own whose per-Session credentials answer it
//! instead. [`party`] tells the proxy's own failures from the site's, and
//! [`limits`] holds each navigation through the proxy to its limits, shared
//! by every run of the user (ADRs 0053 and 0088).
//!
//! A Session whose `proxy.provider` names a Proxy provider gets its one
//! proxy from the provider's driver ([`crate::proxy_provider`]) instead,
//! behind a loopback relay that charges every byte to its Run under the
//! proxy's kind; `proxy.server` and `proxy.provider` together are refused.

mod bypass;
mod limits;
mod party;
mod relay;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64;

use tokio::sync::Mutex;
use tokio::time::Instant;

use goodfetch_cdp::NetError;
use goodfetch_engine::CallError;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::egress::{EndpointPolicy, Gate};
use goodfetch_engine::http_client;
use goodfetch_engine::run::secret_error;
use goodfetch_engine::runs::Run;
use goodfetch_engine::secrets::Purpose;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::page::proxy::{
    CountryCode, PROXY_AUTH_FAILED, PROXY_BYPASS, PROXY_COUNTRY, PROXY_KIND, PROXY_PASSWORD,
    PROXY_PROVIDER, PROXY_PROVIDERS, PROXY_RELAY_UNAVAILABLE, PROXY_SERVER, PROXY_STICKY_LIFETIME,
    PROXY_UNAVAILABLE, PROXY_USERNAME, ProxyAuthFailedDetails, ProxyKind,
    ProxyProviderConfiguration, ProxyRelayUnavailableDetails, ProxyScheme, ProxyServer,
    ProxyUnavailableDetails,
};
use goodfetch_protocol::politeness::POLITENESS_PROXY_CONCURRENCY;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};
use secrecy::{ExposeSecret as _, SecretString};

use self::limits::{Account, ProxyLimits};
pub(crate) use self::limits::{ProxyPermit, TABLES};
pub use self::party::{Answer, Party, party};
pub(crate) use self::party::{PROXY_STATUS, proxy_error, refusal};
use self::relay::{Credentials, Mode, Relay, basic};
use crate::egress_relay::serve::{Met, Upstream};
use crate::egress_relay::{Chain, Outcomes};
use crate::proxy_provider::failure::{ProviderAnswer, ProviderFailure};
use crate::proxy_provider::{
    Chosen, ProxyDriver, ProxyUsage, Refusal, Wanted, apify, bright_data, list, oxylabs, priced,
};

/// A hosted provider adapter's translation point (ADR 0127): its reading of
/// its provider's own failed answer.
type Translation = fn(&ProviderAnswer) -> ProviderFailure;

/// The rule that sends loopback through the proxy too, as Playwright does,
/// so a Session's every request takes its proxy.
pub(crate) const PROXIED_LOOPBACK: &str = "<-loopback>";

/// A Session's proxy, as it opened.
pub(crate) struct SessionProxy {
    server: ProxyServer,
    bypass: Option<String>,
    username: Option<String>,
    password: Option<SecretString>,
    /// The loopback relay of a SOCKS5 proxy with credentials, and of every
    /// proxy a Proxy provider gave.
    relay: Option<Relay>,
    /// What a Proxy provider gave, if one did.
    provided: Option<Provided>,
    /// Its limits, shared by every run of the user, unless the Politeness
    /// file failed to open.
    limits: Option<Arc<ProxyLimits>>,
    /// What the Session's egress relay met on its connections to the
    /// proxy, by the authority each asked for: whether it failed to open,
    /// and whether the proxy refused the credentials.
    through_relay: Outcomes,
    /// How the proxy's own addresses are judged: as an Operator-configured
    /// endpoint's, under `egress.no_private_endpoints` (ADR 0017).
    judged: EndpointPolicy,
}

/// The proxy a Proxy provider gave a Session, for its life.
struct Provided {
    /// The provider's name, as `proxy.provider` gave it.
    provider: String,
    /// Its adapter's reading of its failures, for a hosted provider.
    translation: Option<Translation>,
    chosen: Chosen,
    /// What its bytes are charged to.
    usage: Arc<ProxyUsage>,
    /// When the Session opened, on its Run's clock.
    opened: Instant,
    /// Whether the Run was warned that the Session outlived its sticky
    /// lifetime.
    warned: Mutex<bool>,
}

/// Where a Session opens among its Run's, for its proxy.
pub(crate) struct Opening<'a> {
    /// The Session's ID, which a provider's sticky username names.
    pub(crate) session: &'a SessionId,
    /// Its position among the Run's Sessions, from 1, which picks its
    /// provider's server in turn.
    pub(crate) ordinal: u64,
    /// The currencies of the currency Limits on `proxy_bytes` that hold
    /// the Run, which a provider's price must be in.
    pub(crate) currencies: &'a [String],
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

/// `setting_invalid` for `key`, set at `settings`' layer for it, because of
/// `reason`.
fn invalid(settings: &Resolved, setting: &Setting, reason: String) -> CallError {
    let key = setting.name;
    CallError::new(
        &SETTING_INVALID,
        format!("`{key}` is refused: {reason}"),
        &SettingInvalidDetails {
            key: key.to_owned(),
            layer: settings.effective(setting).map(|effective| effective.layer),
            reason,
        },
    )
}

/// `setting_invalid` for what a Proxy provider's driver refused.
fn refused(settings: &Resolved, refusal: &Refusal) -> CallError {
    let names = |items: Vec<String>| {
        if items.is_empty() {
            "none".to_owned()
        } else {
            items.join(", ")
        }
    };
    match refusal {
        Refusal::KindNotOffered {
            provider,
            asked,
            offered,
        } => invalid(
            settings,
            &PROXY_KIND,
            format!(
                "the Proxy provider `{provider}` offers no `{asked}` proxy; it offers {}",
                names(offered.iter().map(ToString::to_string).collect())
            ),
        ),
        Refusal::CountryNotOffered {
            provider,
            asked,
            kind,
            offered,
        } => invalid(
            settings,
            &PROXY_COUNTRY,
            format!(
                "the Proxy provider `{provider}` offers no `{kind}` proxy in `{asked}`; it offers {}",
                names(offered.iter().map(ToString::to_string).collect())
            ),
        ),
        Refusal::Empty { provider } => invalid(
            settings,
            &PROXY_PROVIDERS,
            format!(
                "the Proxy provider `{provider}` offers no proxy: give `proxy.providers.{provider}.servers` in configuration"
            ),
        ),
        Refusal::Unpriced { provider, currency } => invalid(
            settings,
            &PROXY_PROVIDERS,
            format!(
                "a currency Limit on `proxy_bytes` in {currency} holds the Run, and the Proxy provider `{provider}` has no price in {currency}: set `proxy.providers.{provider}.price`, such as `5 {currency}` per GB, in configuration"
            ),
        ),
    }
}

/// The setting, `proxy.username` or `proxy.password`, whose value is longer
/// than the 255 bytes a SOCKS5 proxy takes it in (RFC 1929); none when both
/// fit.
fn too_long_for_socks5(
    username: &str,
    password: Option<&SecretString>,
) -> Option<&'static Setting> {
    const MOST: usize = u8::MAX as usize;
    if username.len() > MOST {
        Some(&PROXY_USERNAME)
    } else if password.is_some_and(|password| password.expose_secret().len() > MOST) {
        Some(&PROXY_PASSWORD)
    } else {
        None
    }
}

/// `setting_invalid` for credentials a SOCKS5 proxy cannot take, set as
/// `key` in `settings`, if `username` or `password` is too long.
fn fits_socks5(
    settings: &Resolved,
    key: Option<&Setting>,
    username: &str,
    password: Option<&SecretString>,
) -> Result<(), CallError> {
    match too_long_for_socks5(username, password) {
        Some(long) => Err(invalid(
            settings,
            key.unwrap_or(long),
            format!(
                "a SOCKS5 proxy takes a username and a password of at most 255 bytes each (RFC 1929), and `{}` is longer",
                long.name
            ),
        )),
        None => Ok(()),
    }
}

/// The proxy kind, country and sticky lifetime `settings` ask a Proxy
/// provider for.
fn wanted(settings: &Resolved) -> Wanted {
    Wanted {
        kind: settings
            .value(&PROXY_KIND)
            .as_str()
            .and_then(ProxyKind::named),
        country: settings
            .value(&PROXY_COUNTRY)
            .as_str()
            .map(CountryCode::parse)
            .transpose()
            .expect("invariant: `proxy.country`'s value was checked against its kind"),
        sticky_lifetime: settings
            .value(&PROXY_STICKY_LIFETIME)
            .as_u64()
            .map(Duration::from_millis),
    }
}

/// The driver of the built-in Proxy provider `name` of `configuration`, and
/// its adapter's reading of its failures for a hosted provider.
///
/// # Errors
///
/// `setting_invalid` for a name no built-in provider has, an account its
/// adapter cannot build on, or `zones` beside a provider other than
/// `bright_data`, which names no zone.
fn adapter(
    settings: &Resolved,
    name: &str,
    configuration: &ProxyProviderConfiguration,
) -> Result<(ProxyDriver, Option<Translation>), CallError> {
    let account = |reason: String| invalid(settings, &PROXY_PROVIDERS, reason);
    if name != bright_data::NAME && !configuration.zones.is_empty() {
        return Err(account(format!(
            "`proxy.providers.{name}.zones` names zones, which only `bright_data` has: leave it out"
        )));
    }
    Ok(match name {
        list::NAME => (ProxyDriver::new(name, list::List::new(configuration)), None),
        bright_data::NAME => (
            ProxyDriver::new(
                name,
                bright_data::BrightData::new(configuration).map_err(account)?,
            ),
            Some(bright_data::failure),
        ),
        oxylabs::NAME => (
            ProxyDriver::new(name, oxylabs::Oxylabs::new(configuration).map_err(account)?),
            Some(oxylabs::failure),
        ),
        apify::NAME => (
            ProxyDriver::new(name, apify::Apify::new(configuration)),
            Some(apify::failure),
        ),
        _ => {
            return Err(invalid(
                settings,
                &PROXY_PROVIDER,
                format!(
                    "no Proxy provider is named `{name}`: the built-in ones are `bright_data`, `oxylabs`, `apify` and `list`"
                ),
            ));
        }
    })
}

/// How the relay reaches the proxy a Proxy provider chose: SOCKS5 or HTTP,
/// with its credentials.
///
/// # Errors
///
/// `setting_invalid` for credentials too long for a SOCKS5 proxy.
fn provided_mode(
    settings: &Resolved,
    chosen: &Chosen,
    password: Option<&SecretString>,
) -> Result<Mode, CallError> {
    Ok(match (&chosen.username, chosen.server.scheme.is_socks()) {
        (Some(username), true) => {
            fits_socks5(settings, Some(&PROXY_PROVIDERS), username, password)?;
            Mode::Socks5 {
                credentials: Some(Credentials {
                    username: username.clone(),
                    password: password.cloned().unwrap_or_else(|| SecretString::from("")),
                }),
            }
        }
        (None, true) => Mode::Socks5 { credentials: None },
        (username, false) => Mode::Http {
            authorization: username.as_deref().map(|username| {
                let password = password.map_or("", |password| password.expose_secret());
                SecretString::from(basic(username, password))
            }),
        },
    })
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
    pub(crate) async fn open(
        settings: &Resolved,
        run: &Run,
        opening: &Opening<'_>,
    ) -> Result<Option<Self>, CallError> {
        let text = |setting: &Setting| settings.get::<Option<&str>>(setting).map(str::to_owned);
        let username = text(&PROXY_USERNAME);
        let reference = settings.get::<Option<SecretReference>>(&PROXY_PASSWORD);
        if let Some(provider) = text(&PROXY_PROVIDER) {
            let also = [&PROXY_SERVER, &PROXY_USERNAME, &PROXY_PASSWORD]
                .into_iter()
                .find(|setting| !settings.value(setting).is_null())
                .map(|setting| setting.name);
            if let Some(also) = also {
                return Err(invalid(
                    settings,
                    &PROXY_PROVIDER,
                    format!(
                        "the Session resolves both `{also}` and `proxy.provider`: a Session's one proxy is either `proxy.server`, with `proxy.username` and `proxy.password`, or a Proxy provider's, whose credentials are `proxy.providers.{provider}.username` and `.password`"
                    ),
                ));
            }
            return Self::provided(settings, run, opening, &provider)
                .await
                .map(Some);
        }
        let Some(server) = settings.value(&PROXY_SERVER).as_str().map(str::to_owned) else {
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
        let judged = EndpointPolicy::of(settings);
        let relayed = username.is_some() && server.scheme.is_socks();
        let addresses =
            proxy_addresses(&server, judged, relayed, || lookup(&server, settings)).await?;
        let password = match (&username, reference) {
            (None, Some(_)) => return Err(needs(PROXY_PASSWORD.name, PROXY_USERNAME.name)),
            (_, None) => None,
            (Some(_), Some(reference)) => {
                let layer = settings
                    .effective(&PROXY_PASSWORD)
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
                fits_socks5(settings, None, username, password.as_ref())?;
                let password = password.clone().unwrap_or_else(|| SecretString::from(""));
                let mode = Mode::Socks5 {
                    credentials: Some(Credentials {
                        username: username.clone(),
                        password,
                    }),
                };
                let relay = Relay::open(run, addresses, mode, None, false)
                    .await
                    .map_err(|error| relay_unavailable(&server, &error))?;
                Some(relay)
            }
            _ => None,
        };
        let account = Account::Proxy {
            server: &server,
            username: username.as_deref(),
        };
        let limits = ProxyLimits::open(settings, run, account)
            .await
            .map_err(|reason| invalid(settings, &POLITENESS_PROXY_CONCURRENCY, reason))?;
        Ok(Some(Self {
            server,
            bypass: text(&PROXY_BYPASS),
            username,
            password,
            relay,
            provided: None,
            limits,
            through_relay: Outcomes::default(),
            judged,
        }))
    }

    /// The proxy the Proxy provider `name` gives the Session `opening`
    /// names, as `settings` ask: its password resolved now, and the relay
    /// that charges its bytes opened now.
    async fn provided(
        settings: &Resolved,
        run: &Run,
        opening: &Opening<'_>,
        name: &str,
    ) -> Result<Self, CallError> {
        let configuration = settings
            .value(&PROXY_PROVIDERS)
            .get(name)
            .map(ProxyProviderConfiguration::read)
            .transpose()
            .expect("invariant: `proxy.providers`' entries were checked against their kind")
            .unwrap_or_default();
        let (driver, translation) = adapter(settings, name, &configuration)?;
        let chosen = driver
            .choose(&wanted(settings), &opening.session.0, opening.ordinal)
            .map_err(|refusal| refused(settings, &refusal))?;
        let currencies: Vec<&str> = opening.currencies.iter().map(String::as_str).collect();
        let price = priced(driver.name(), &currencies, configuration.price.as_ref())
            .map_err(|refusal| refused(settings, &refusal))?;
        let password = match (&chosen.username, &configuration.password) {
            (Some(_), Some(reference)) => {
                let purpose = Purpose {
                    placeholder: PROXY_PROVIDERS.name,
                    domain: Some(&chosen.server.host),
                    domains: &[],
                    layer: settings
                        .effective(&PROXY_PROVIDERS)
                        .map(|effective| effective.layer),
                    run_id: Some(run.id()),
                };
                Some(
                    run.secrets()
                        .resolve(reference, &purpose)
                        .await
                        .map_err(|error| secret_error(&error))?,
                )
            }
            _ => None,
        };
        let judged = EndpointPolicy::of(settings);
        let addresses = proxy_addresses(&chosen.server, judged, true, || {
            lookup(&chosen.server, settings)
        })
        .await?;
        let mode = provided_mode(settings, &chosen, password.as_ref())?;
        let usage = Arc::new(ProxyUsage::new(run.clone(), chosen.kind, price));
        let relay = Relay::open(
            run,
            addresses,
            mode,
            Some(Arc::clone(&usage)),
            translation.is_some(),
        )
        .await
        .map_err(|error| relay_unavailable(&chosen.server, &error))?;
        // A hosted provider's sticky username names the Session, so its
        // limits are its account's: the provider and the configured
        // username, which every Session of the account shares (ADR 0088).
        let account = match translation {
            Some(_) => Account::Provider {
                name,
                username: configuration.username.as_deref(),
            },
            None => Account::Proxy {
                server: &chosen.server,
                username: chosen.username.as_deref(),
            },
        };
        let limits = ProxyLimits::open(settings, run, account)
            .await
            .map_err(|reason| invalid(settings, &POLITENESS_PROXY_CONCURRENCY, reason))?;
        Ok(Self {
            limits,
            server: chosen.server.clone(),
            bypass: settings.value(&PROXY_BYPASS).as_str().map(str::to_owned),
            username: chosen.username.clone(),
            password,
            relay: Some(relay),
            provided: Some(Provided {
                provider: name.to_owned(),
                translation,
                chosen,
                usage,
                opened: run.clock().now(),
                warned: Mutex::new(false),
            }),
            through_relay: Outcomes::default(),
            judged,
        })
    }

    /// Warns `run`, once, when the Session `session` outlived its
    /// provider's sticky lifetime, so its exit may have changed
    /// (`proxy_exit_may_change`).
    pub(crate) async fn warn_if_outlived(&self, run: &Run, session: &SessionId) {
        let Some(provided) = &self.provided else {
            return;
        };
        let lived = run.clock().now().saturating_duration_since(provided.opened);
        let Some(warning) = provided.chosen.exit_warning(&session.0, lived) else {
            return;
        };
        let mut warned = provided.warned.lock().await;
        if std::mem::replace(&mut *warned, true) {
            return;
        }
        drop(warned);
        if let Err(error) = run.warn(warning).await {
            tracing::warn!(%error, "the warning that a Session's proxy exit may change was not kept");
        }
    }

    /// A Permit for one navigation hop through the proxy, once its limits
    /// admit it; none when it has no limits, and goes unlimited.
    pub(crate) async fn permit(&self) -> Option<ProxyPermit> {
        self.limits.as_ref()?.take().await
    }

    /// The `proxyServer` the browser context takes: the relay's address, an
    /// HTTP proxy, when the Session has one, and Chromium's `socks5` for
    /// `socks5h`, whose SOCKS5 proxies resolve every name they are given.
    pub(crate) fn browser_server(&self) -> String {
        if let Some(relay) = &self.relay {
            return format!("http://{}", relay.address());
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
    /// the relay's, or an HTTP or HTTPS proxy's, whose 407 then reaches the
    /// engine.
    pub(crate) fn answers_authentication(&self) -> bool {
        self.relay.is_some() || !self.server.scheme.is_socks()
    }

    /// The credentials the engine answers the proxy's 407 with, if it has
    /// any: the relay's own when the Session has one, which gives the
    /// proxy's itself.
    pub(crate) fn credentials(&self) -> Option<(&str, Option<&SecretString>)> {
        if let Some(relay) = &self.relay {
            let (username, password) = relay.credentials();
            return Some((username, Some(password)));
        }
        self.username
            .as_deref()
            .map(|username| (username, self.password.as_ref()))
    }

    /// `proxy_unavailable` for a request to `url` that met `error`, the
    /// browser's network error or the proxy's own.
    pub(crate) fn unavailable(&self, url: &str, error: &str) -> CallError {
        unavailable(&self.server, Some(url), error)
    }

    /// `proxy_auth_failed` for a request to `url` the proxy answered with
    /// a 407: its fix names the keys the Session's credentials come from, a
    /// Proxy provider's own beside `proxy.provider`, where `proxy.username`
    /// and `proxy.password` are refused (the deep review's 599-6).
    pub(crate) fn auth_failed(&self, url: &str) -> CallError {
        let given = match &self.username {
            Some(username) => format!("refused the credentials of `{username}`"),
            None => "asked for credentials the Session does not have".to_owned(),
        };
        let fix = match &self.provided {
            Some(provided) if provided.provider == bright_data::NAME => format!(
                "set `proxy.providers.{0}.username` and `proxy.providers.{0}.password`, and `proxy.providers.{0}.zones` for a zone its customer named otherwise than its proxy kind",
                provided.provider
            ),
            Some(provided) => format!(
                "set `proxy.providers.{0}.username` and `proxy.providers.{0}.password`",
                provided.provider
            ),
            None => "set `proxy.username` and `proxy.password`".to_owned(),
        };
        CallError::new(
            &PROXY_AUTH_FAILED,
            format!("the proxy {} {given}: {fix}", self.server),
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
    pub(crate) fn failure(&self, url: &str, error: &NetError) -> Option<CallError> {
        if let Some(refusal) = self
            .provided
            .as_ref()
            .and_then(|provided| provided.usage.refusal())
        {
            return Some(refusal.clone());
        }
        if party(Answer::Failure { error }) != Party::Proxy {
            return None;
        }
        if let Some(failure) = self.provider_failure(url) {
            return Some(
                failure
                    .unavailable(&self.provider_name(), &self.server, Some(url))
                    .unwrap_or_else(|| self.auth_failed(url)),
            );
        }
        Some(if self.refused_credentials(url) {
            self.auth_failed(url)
        } else {
            self.unavailable(url, error.name())
        })
    }

    /// The hosted provider's failure in its proxy's answer to the latest
    /// `CONNECT` for `url` it refused, as its adapter reads it (ADR 0127).
    fn provider_failure(&self, url: &str) -> Option<ProviderFailure> {
        let translation = self.provided.as_ref()?.translation?;
        let answer = self.relay.as_ref()?.latest_refusal(url)?;
        Some(translation(&answer))
    }

    /// The name of the Proxy provider that gave the proxy, if one did.
    fn provider_name(&self) -> String {
        self.provided
            .as_ref()
            .map(|provided| provided.provider.clone())
            .unwrap_or_default()
    }

    /// Whether a request to `url` goes around the proxy, as its
    /// `proxy.bypass` says.
    pub(crate) fn bypasses(&self, url: &str) -> bool {
        bypass::bypassed(self.bypass.as_deref(), url)
    }

    /// A response to `url` through the proxy, carrying the `Proxy-Status`
    /// field `proxy_status`: one the proxy forwarded itself only for an
    /// `http:` URL sent to an HTTP or HTTPS proxy and not around it, the
    /// one case where the proxy's own member can be in the field; and,
    /// through a SOCKS5 proxy, one the Session's own relay answered for a
    /// proxy it could not use, which the relay recorded for `url`'s
    /// connection, so a site's own `Proxy-Status` naming the relay is still
    /// the site's, whatever another connection met.
    pub(crate) fn response<'a>(&self, url: &str, proxy_status: Option<&'a str>) -> Answer<'a> {
        let plain = url::Url::parse(url).is_ok_and(|url| url.scheme() == "http");
        let proxied = plain && !self.bypasses(url);
        let socks = self.server.scheme.is_socks();
        let relays = socks
            && proxy_status.is_some_and(party::relays)
            && (self.relay.as_ref().is_some_and(|relay| relay.answered(url))
                || self.through_relay.answered(url));
        Answer::Response {
            proxy_status,
            forwarded: proxied && (!socks || relays),
        }
    }

    /// The Error code a navigation to `url` raises whose response is the
    /// proxy's own, naming `error`: `proxy_auth_failed` when a SOCKS5
    /// proxy refused the Session's credentials on `url`'s latest
    /// connection, `proxy_unavailable` otherwise.
    pub(crate) fn answered(&self, url: &str, error: &str) -> CallError {
        if self.server.scheme.is_socks() && self.refused_credentials(url) {
            self.auth_failed(url)
        } else {
            self.unavailable(url, error)
        }
    }

    /// Whether `answer` to a request to `url`, with `status` for a
    /// response, is the proxy refusing a request its plan does not admit,
    /// which lowers its learned ceiling: never a failure whose connection
    /// reached no proxy, which is a proxy that is down.
    pub(crate) fn refusal(&self, url: &str, answer: Answer<'_>, status: Option<u16>) -> bool {
        let down = matches!(answer, Answer::Failure { .. })
            && (self
                .relay
                .as_ref()
                .is_some_and(|relay| relay.unreachable(url))
                || self.through_relay.unreachable(url));
        !down && refusal(answer, status)
    }

    /// Whether the proxy refused the Session's credentials on the latest
    /// connection for `url` a relay opened.
    pub(crate) fn refused_credentials(&self, url: &str) -> bool {
        self.relay
            .as_ref()
            .is_some_and(|relay| relay.refused_credentials(url))
            || self.through_relay.refused(url)
    }

    /// How the Session's egress relay chains through the proxy, telling it
    /// what each connection met: none for an `https://` proxy, which the
    /// relay does not speak TLS to, so the browser reaches it itself.
    pub(crate) fn chain(self: &Arc<Self>) -> Option<Chain> {
        let authorization = || {
            self.credentials().map(|(username, password)| {
                let password = password.map_or("", |password| password.expose_secret());
                format!(
                    "Basic {}",
                    BASE64.encode(format!("{username}:{password}").as_bytes())
                )
            })
        };
        if let Some(relay) = &self.relay {
            let address = relay.address();
            let upstream = Upstream::Http {
                authorization: authorization(),
            };
            return Some(self.chained(upstream, (address.ip().to_string(), address.port())));
        }
        let upstream = match self.server.scheme {
            ProxyScheme::Https => return None,
            ProxyScheme::Http => Upstream::Http {
                authorization: authorization(),
            },
            ProxyScheme::Socks5 | ProxyScheme::Socks5h => Upstream::Socks5,
        };
        Some(self.chained(upstream, (self.server.host.clone(), self.server.port)))
    }

    /// The chain through `address` speaking `upstream`, its bypass and what
    /// its connections met this proxy's.
    fn chained(self: &Arc<Self>, upstream: Upstream, address: (String, u16)) -> Chain {
        let (bypassing, meeting) = (Arc::clone(self), Arc::clone(self));
        Chain {
            upstream,
            address,
            bypasses: Box::new(move |url| bypassing.bypasses(url)),
            met: Box::new(move |authority, met| meeting.met_through_relay(authority, met)),
            // The loopback relay's own address is no proxy's: the
            // Session's open judged the proxy behind it.
            judged: self.relay.is_none().then_some(self.judged),
        }
    }

    /// Learns what the Session's egress relay met on a connection for
    /// `authority` to the proxy.
    pub(crate) fn met_through_relay(&self, authority: &str, met: Met) {
        self.through_relay.record(authority, met);
    }
}

/// The addresses of `server`'s host: an IP address as it is, a name from
/// the Run's `network.resolve` mapping first, then the system's resolver, as
/// the engine's own requests resolve theirs ([`http_client::lookup`], whose
/// lookup runs off the async workers), so no Session's open reads the
/// system's resolver configuration; none when it has none.
async fn lookup(server: &ProxyServer, settings: &Resolved) -> Option<Vec<SocketAddr>> {
    http_client::lookup(&server.host, server.port, &HostMapping::of(settings))
        .await
        .ok()
}

/// Looks `server`'s host up once, with `lookup`, and judges that answer
/// under `judged` before the Session opens (ADR 0017): the addresses judged,
/// which the loopback relay of a `relayed` proxy dials alone, so an answer
/// that changes after the judgment reaches nothing. A browser that reaches
/// the proxy itself meets no other check of the proxy's address. A host
/// with no address is left to the connection, which fails as the proxy's,
/// unless `egress.no_private_endpoints` holds the proxy to addresses it
/// judged or the relay needs one to dial.
///
/// # Errors
///
/// `egress_refused` naming the proxy, its address and
/// `egress.no_private_endpoints`; `proxy_unavailable` when the host has no
/// address and the switch is on or the proxy is `relayed`.
async fn proxy_addresses<L, F>(
    server: &ProxyServer,
    judged: EndpointPolicy,
    relayed: bool,
    lookup: L,
) -> Result<Vec<SocketAddr>, CallError>
where
    L: FnOnce() -> F,
    F: Future<Output = Option<Vec<SocketAddr>>>,
{
    let Some(addresses) = lookup().await.filter(|addresses| !addresses.is_empty()) else {
        if relayed || judged.no_private_endpoints() {
            return Err(unavailable(server, None, "the proxy's host has no address"));
        }
        return Ok(Vec::new());
    };
    let named = server.to_string();
    Gate::Endpoint(judged)
        .judge_host(&server.host, &named)
        .and_then(|judged| judged.addresses(&addresses))
        .map_err(|refused| refused.error())?;
    Ok(addresses)
}

/// `proxy_relay_unavailable` for the relay to `server`, which could not
/// open for `error`: the engine's own failure, never the proxy's.
fn relay_unavailable(server: &ProxyServer, error: &std::io::Error) -> CallError {
    CallError::new(
        &PROXY_RELAY_UNAVAILABLE,
        format!("the loopback relay to the proxy {server} could not open: {error}"),
        &ProxyRelayUnavailableDetails {
            proxy: server.to_string(),
            error: error.to_string(),
        },
    )
}

/// `proxy_unavailable` for `server`, met by a request to `url`, or by the
/// Session's open when none.
fn unavailable(server: &ProxyServer, url: Option<&str>, error: &str) -> CallError {
    CallError::new(
        &PROXY_UNAVAILABLE,
        format!("the proxy {server} is unavailable: {error}"),
        &ProxyUnavailableDetails {
            proxy: server.to_string(),
            url: url.map(str::to_owned),
            error: Some(error.to_owned()),
        },
    )
}

#[cfg(test)]
mod tests;
