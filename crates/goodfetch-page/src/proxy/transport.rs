//! A Session's proxy as a fetch tier's own HTTP transport takes it
//! (#1640): opened as a browser Session's is, from the same settings and
//! the same Proxy-provider driver, held for the Session's life, and reached
//! as an HTTP proxy, through the loopback relay a SOCKS5 proxy takes since
//! the transport speaks no SOCKS5. Each hop through it holds a Permit of
//! the proxy's limits, given back as the proxy's refusal when the proxy
//! refused the hop, which lowers its learned ceiling (ADR 0053). A hop the
//! proxy answered or failed itself raises the proxy's error, as a browser
//! Session's navigation does, never the site's answer (the proxy-or-site
//! rule).

use goodfetch_cdp::NetError;
use goodfetch_engine::CallError;
use goodfetch_engine::run::state_unavailable;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::page::proxy::PROXY_PROVIDER;
use goodfetch_protocol::usage::PROXY_BYTES;
use secrecy::SecretString;

use super::limits::ProxyPermit;
use super::party::{Answer, Party, party, proxy_error, relays};
use super::relay::{Mode, Relay};
use super::{Opening, SessionProxy, lookup, proxy_addresses, relay_unavailable};

/// The proxy one Session's HTTP transport sends through, as it opened.
pub struct TransportProxy {
    proxy: SessionProxy,
    /// The loopback relay in front of a SOCKS5 proxy the Session has no
    /// credentials for, which the transport reaches as an HTTP proxy.
    socks: Option<Relay>,
    /// The Session it opened for, which a provider's sticky username names.
    session: SessionId,
}

/// Where a transport sends its hops through a proxy: the proxy's URL, an
/// HTTP or HTTPS proxy's, and the credentials it answers with.
#[derive(Clone, Debug)]
pub struct ProxyRoute {
    /// The proxy's URL, the loopback relay's when the Session has one.
    pub url: String,
    /// The username and password the proxy asks for, if any.
    pub credentials: Option<(String, SecretString)>,
}

/// A Permit of the proxy's limits one hop through it holds: dropped, it
/// goes back as a request that ended.
#[derive(Debug)]
pub struct TransportPermit(ProxyPermit);

impl TransportPermit {
    /// Gives the Permit back now for a hop the proxy itself refused,
    /// lowering the proxy's ceiling.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the Permit to the Run's end, which
    /// frees it.
    pub async fn refused(self) {
        self.0.refused().await;
    }
}

impl TransportProxy {
    /// The proxy `settings`, a Run's, name for its Session `session`, the
    /// `ordinal`-th of the Run's to open one, from 1, if they name one.
    ///
    /// # Errors
    ///
    /// The errors of a browser Session's proxy as it opens
    /// (`setting_invalid`, the Secret provider's codes, `proxy_unavailable`
    /// and `proxy_relay_unavailable`), and `run_state_unavailable` when the
    /// Run's Limits on `proxy_bytes` cannot be read for a Proxy provider.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any relay it opened.
    pub async fn open(
        run: &Run,
        settings: &Resolved,
        session: SessionId,
        ordinal: u64,
    ) -> Result<Option<Self>, CallError> {
        // A Proxy provider's price must be in the currency of each currency
        // Limit on `proxy_bytes` that holds the Run (ADR 0101), as a
        // browser Session's.
        let currencies = if settings.get::<Option<&str>>(&PROXY_PROVIDER).is_none() {
            Vec::new()
        } else {
            run.usage()
                .await
                .map_err(|error| {
                    state_unavailable(run.id().clone(), run.folder(), &error.to_string())
                })?
                .limits
                .into_iter()
                .filter(|limit| limit.usage_kind == PROXY_BYTES)
                .filter_map(|limit| limit.currency)
                .collect()
        };
        let opening = Opening {
            session: &session,
            ordinal,
            currencies: &currencies,
        };
        let Some(proxy) = SessionProxy::open(settings, run, &opening).await? else {
            return Ok(None);
        };
        let socks = if proxy.relay.is_none() && proxy.server.scheme.is_socks() {
            let addresses = proxy_addresses(&proxy.server, proxy.judged, true, || {
                lookup(&proxy.server, settings)
            })
            .await?;
            let mode = Mode::Socks5 { credentials: None };
            Some(
                Relay::open(run, addresses, mode, None, false)
                    .await
                    .map_err(|error| relay_unavailable(&proxy.server, &error))?,
            )
        } else {
            None
        };
        Ok(Some(Self {
            proxy,
            socks,
            session,
        }))
    }

    /// Where the transport sends its hops: the loopback relay, with its
    /// own credentials, when the Session has one; the proxy itself, with
    /// the Session's, otherwise.
    #[must_use]
    pub fn route(&self) -> ProxyRoute {
        if let Some(relay) = self.proxy.relay.as_ref().or(self.socks.as_ref()) {
            let (username, password) = relay.credentials();
            return ProxyRoute {
                url: format!("http://{}", relay.address()),
                credentials: Some((username.to_owned(), password.clone())),
            };
        }
        ProxyRoute {
            url: self.proxy.server.to_string(),
            credentials: self.proxy.username.clone().map(|username| {
                let password = self
                    .proxy
                    .password
                    .clone()
                    .unwrap_or_else(|| SecretString::from(""));
                (username, password)
            }),
        }
    }

    /// Whether a hop to `url` goes around the proxy, as `proxy.bypass`
    /// says.
    #[must_use]
    pub fn bypasses(&self, url: &str) -> bool {
        self.proxy.bypasses(url)
    }

    /// A Permit for one hop through the proxy, once its limits admit it
    /// (ADRs 0053 and 0088); none when the proxy has no limits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the limits as they were.
    pub async fn permit(&self) -> Option<TransportPermit> {
        self.proxy.permit().await.map(TransportPermit)
    }

    /// Whether the response to `url` with `status` and the `Proxy-Status`
    /// field `proxy_status` is the proxy refusing a request its plan does
    /// not admit: its own 429 or 502 (the proxy-or-site rule), never the
    /// site's nor the relay's own.
    #[must_use]
    pub fn refused_response(&self, url: &str, status: u16, proxy_status: Option<&str>) -> bool {
        let answer = self.response(url, proxy_status);
        self.proxy.refusal(url, answer, Some(status))
    }

    /// The error a response to `url` with `status` and the `Proxy-Status`
    /// field `proxy_status` raises when the proxy gave it, never the site
    /// (the proxy-or-site rule): `proxy_auth_failed` for its `407` to a
    /// request it forwarded, which a browser takes as the proxy's too;
    /// `proxy_unavailable` naming the error its field names otherwise.
    #[must_use]
    pub fn answered(
        &self,
        url: &str,
        status: u16,
        proxy_status: Option<&str>,
    ) -> Option<CallError> {
        let answer = self.response(url, proxy_status);
        if status == 407
            && matches!(
                answer,
                Answer::Response {
                    forwarded: true,
                    ..
                }
            )
        {
            return Some(self.proxy.auth_failed(url));
        }
        if party(answer) != Party::Proxy {
            return None;
        }
        let error = proxy_status.and_then(proxy_error).unwrap_or_default();
        Some(self.proxy.answered(url, &error))
    }

    /// The error a hop to `url` that failed with the network error
    /// `network_error`, as Chrome names it, raises when the proxy failed it
    /// itself: `proxy_auth_failed` or `proxy_unavailable`, as a browser
    /// Session's.
    #[must_use]
    pub fn failure(&self, url: &str, network_error: &str) -> Option<CallError> {
        self.proxy.failure(url, &NetError::of(network_error))
    }

    /// A response to `url` through the proxy, as a browser Session reads
    /// one; the `502` of the transport's own SOCKS5 relay for a proxy it
    /// could not use is the proxy's, as the Session's relay's is.
    fn response<'a>(&self, url: &str, proxy_status: Option<&'a str>) -> Answer<'a> {
        let relayed = proxy_status.is_some_and(relays)
            && self.socks.as_ref().is_some_and(|relay| relay.answered(url));
        if relayed {
            return Answer::Response {
                proxy_status,
                forwarded: true,
            };
        }
        self.proxy.response(url, proxy_status)
    }

    /// Whether the hop to `url` that failed with the network error
    /// `network_error`, as Chrome names it, is the proxy refusing its
    /// tunnel, never a proxy that could not be reached.
    #[must_use]
    pub fn refused_failure(&self, url: &str, network_error: &str) -> bool {
        let error = NetError::of(network_error);
        let down = self
            .socks
            .as_ref()
            .is_some_and(|relay| relay.unreachable(url));
        !down
            && self
                .proxy
                .refusal(url, Answer::Failure { error: &error }, None)
    }

    /// Warns `run`, once, when the Session outlived its provider's sticky
    /// lifetime, so its exit may have changed (`proxy_exit_may_change`).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the warning unwritten.
    pub async fn warn_if_outlived(&self, run: &Run) {
        self.proxy.warn_if_outlived(run, &self.session).await;
    }
}

impl std::fmt::Debug for TransportProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransportProxy")
            .field("proxy", &self.proxy)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}
