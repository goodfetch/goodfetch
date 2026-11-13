//! The HTTP tier's transports through a Run's proxy (#1640): a Run that
//! sets `proxy.server`, `proxy.provider` or `proxy.kind` sends every hop of
//! its own transport, a forced `http` tier's documents, robots.txt, the
//! AI-use signal files and sitemaps, through a proxy, as its browser
//! Sessions do. Each pool Session takes one proxy for its life, opened from
//! the same settings and Proxy-provider driver as a browser Session's on
//! its first hop and held until the Session retires; the Run's own reads,
//! the navigation gate's robots.txt and signal files, take one more. A hop
//! that `proxy.bypass` sends around the proxy goes on the Session's own
//! transport, and each hop through the proxy holds a Permit of the proxy's
//! limits until its body ends, given back as the proxy's refusal when the
//! proxy refused it, which lowers the proxy's ceiling (ADR 0053), as the
//! browser tier does. A hop the proxy answered or failed itself ends with
//! the proxy's error, `proxy_auth_failed` or `proxy_unavailable`, never the
//! proxy's answer as the site's. A proxy that did not open is opened again
//! on the Session's next hop.
#![expect(
    clippy::disallowed_types,
    reason = "the table of Sessions' proxies is read or written in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_page::document_request::DocumentRequest;
use goodfetch_page::{TransportPermit, TransportProxy};
use goodfetch_protocol::page::SessionId;
use tokio::sync::OnceCell;
use url::Url;

use crate::driver::{Body, CallFailure, Hop, Reuse, Sending, Sent, Transport, TransportFailure};
use crate::transports::{LocalSource, Reqwest, TransportConfig};

/// The proxies of a Run's Sessions on its HTTP tier, by Session, each
/// opened on its Session's first hop; a retired Session's leaves with it.
pub(crate) struct SessionProxies {
    run: Run,
    settings: Resolved,
    /// The Run's transport configuration, which each Session's proxied
    /// transport takes with its own local addresses and proxy.
    config: TransportConfig,
    /// The document request each presents: its Sessions' browser's.
    presents: Option<DocumentRequest>,
    /// The Run's own transport, sending directly: a Session's hops around
    /// the proxy go on it, unless the Session holds addresses of its own.
    direct: Arc<dyn Transport>,
    /// How many proxies opened, which orders them for a provider's servers.
    opened: AtomicU64,
    by_session: Mutex<BTreeMap<SessionId, Arc<Proxied>>>,
}

impl SessionProxies {
    /// The proxies of `run`'s Sessions under `settings`, its transports
    /// built from `config`, presenting `presents`, `direct` sending around
    /// the proxy.
    pub(crate) fn new(
        run: &Run,
        settings: &Resolved,
        config: TransportConfig,
        presents: Option<DocumentRequest>,
        direct: Arc<dyn Transport>,
    ) -> Self {
        Self {
            run: run.clone(),
            settings: settings.clone(),
            config,
            presents,
            direct,
            opened: AtomicU64::new(0),
            by_session: Mutex::new(BTreeMap::new()),
        }
    }

    /// The transport of the Run's own reads, robots.txt and the AI-use
    /// signal files of its navigation gate, through a proxy of their own.
    pub(crate) fn of_run(&self) -> Arc<Proxied> {
        let sticky = SessionId(format!("{}.http", self.run.id()));
        Arc::new(self.proxied(sticky, self.config.local.clone(), Arc::clone(&self.direct)))
    }

    /// The transport of the pool Session `session`, holding `local`, the
    /// Run's local addresses for `None`, and sending around the proxy on
    /// `direct`, the Run's own transport for `None`: the one it took on
    /// its first use, kept until it retires.
    pub(crate) fn of(
        &self,
        session: &SessionId,
        local: Option<LocalSource>,
        direct: Option<Arc<dyn Transport>>,
    ) -> Arc<Proxied> {
        let mut by_session = self
            .by_session
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(by_session.entry(session.clone()).or_insert_with(|| {
            // Named apart from the Session's browser's, `{run}.session-{n}`,
            // and from every other Run's.
            let sticky = SessionId(format!("{}.http.{}", self.run.id(), session.0));
            let local = local.or_else(|| self.config.local.clone());
            let direct = direct.unwrap_or_else(|| Arc::clone(&self.direct));
            Arc::new(self.proxied(sticky, local, direct))
        }))
    }

    /// The transport of the retired Session `session`, taken off the
    /// table, if it took one.
    pub(crate) fn forget(&self, session: &SessionId) -> Option<Arc<Proxied>> {
        self.by_session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session)
    }

    fn proxied(
        &self,
        sticky: SessionId,
        local: Option<LocalSource>,
        direct: Arc<dyn Transport>,
    ) -> Proxied {
        Proxied {
            run: self.run.clone(),
            settings: self.settings.clone(),
            sticky,
            ordinal: self.opened.fetch_add(1, Ordering::Relaxed) + 1,
            config: TransportConfig {
                local,
                proxy: None,
                ..self.config.clone()
            },
            presents: self.presents.clone(),
            direct,
            opened: OnceCell::new(),
        }
    }
}

/// One Session's transport through its proxy, opened on its first hop.
pub(crate) struct Proxied {
    run: Run,
    settings: Resolved,
    /// The name its proxy's sticky username takes.
    sticky: SessionId,
    /// Its position among the Run's proxies, from 1.
    ordinal: u64,
    /// Its transport's configuration, without the proxy.
    config: TransportConfig,
    presents: Option<DocumentRequest>,
    /// Its transport around the proxy, for the hops `proxy.bypass` names.
    direct: Arc<dyn Transport>,
    /// Its proxy and the transport through it, once opened; none when the
    /// settings name no proxy after all.
    opened: OnceCell<Option<Opened>>,
}

/// A Session's proxy and the transport that sends through it.
struct Opened {
    proxy: TransportProxy,
    transport: Reqwest,
}

impl Proxied {
    /// Opens the Session's proxy and the transport through it.
    async fn open(&self) -> Result<Option<Opened>, TransportFailure> {
        let opened =
            TransportProxy::open(&self.run, &self.settings, self.sticky.clone(), self.ordinal)
                .await
                .map_err(|error| TransportFailure::Proxy(CallFailure(error)))?;
        let Some(proxy) = opened else {
            return Ok(None);
        };
        let transport = Reqwest::new(TransportConfig {
            proxy: Some(proxy.route()),
            ..self.config.clone()
        })
        .map_err(|error| {
            tracing::warn!(%error, "a Session's transport through its proxy could not be built");
            TransportFailure::Other
        })?
        .presenting(self.presents.clone());
        Ok(Some(Opened { proxy, transport }))
    }

    /// Warns the Run, once, when the Session outlived its provider's sticky
    /// lifetime, so its exit may have changed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the warning unwritten.
    pub(crate) async fn warn_if_outlived(&self) {
        if let Some(Some(opened)) = self.opened.get() {
            opened.proxy.warn_if_outlived(&self.run).await;
        }
    }
}

impl Transport for Proxied {
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            let opened = self.opened.get_or_try_init(|| self.open()).await?;
            let url = hop.url.as_str();
            let Some(opened) = opened.as_ref().filter(|opened| !opened.proxy.bypasses(url)) else {
                return self.direct.send(hop, reuse).await;
            };
            let permit = opened.proxy.permit().await;
            match opened.transport.send(hop, reuse).await {
                Ok(sent) => {
                    let proxy_status = sent
                        .headers
                        .iter()
                        .find(|(name, _)| name == "proxy-status")
                        .map(|(_, value)| value.as_str());
                    if let Some(error) = opened.proxy.answered(url, sent.status, proxy_status) {
                        if let Some(permit) = permit
                            && opened
                                .proxy
                                .refused_response(url, sent.status, proxy_status)
                        {
                            permit.refused().await;
                        }
                        return Err(TransportFailure::Proxy(CallFailure(error)));
                    }
                    let body = Box::new(Holding {
                        body: sent.body,
                        permit,
                    });
                    Ok(Sent { body, ..sent })
                }
                Err(failure) => {
                    let network_error = failure.network_error();
                    if let Some(permit) = permit
                        && opened.proxy.refused_failure(url, network_error)
                    {
                        permit.refused().await;
                    }
                    Err(opened
                        .proxy
                        .failure(url, network_error)
                        .map_or(failure, |error| TransportFailure::Proxy(CallFailure(error))))
                }
            }
        })
    }

    fn presents(&self, url: &Url) -> Vec<(String, String)> {
        // Its transport through the proxy presents what the direct one
        // does: its Sessions' browser's document request.
        self.direct.presents(url)
    }
}

/// A body through the proxy, holding its hop's Permit until it ends.
struct Holding {
    body: Box<dyn Body>,
    permit: Option<TransportPermit>,
}

impl Body for Holding {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            let chunk = self.body.chunk().await;
            if !matches!(chunk, Ok(Some(_))) {
                self.permit = None;
            }
            chunk
        })
    }
}
