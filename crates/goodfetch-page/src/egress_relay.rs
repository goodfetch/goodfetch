//! The egress relay (ADRs 0017, 0042, 0044 and 0089; spec #564): every
//! browser Session the engine launches reaches the network through a
//! loopback HTTP proxy of its own while `security.raw_cdp_egress_proxy` is
//! on, its default. A browser resolves names itself, so the interception's
//! check of a request cannot hold the address its connection then uses;
//! the relay resolves each name itself and judges every answer before it
//! connects to those addresses alone, so an answer that changes in between
//! (DNS rebinding) reaches nothing, and a raw CDP client or a Lent page
//! that switches interception off, a WebSocket and WebRTC over TCP are held
//! at the network layer too, with no relaunch. The domain policy is judged
//! there again, for the connections no interception sees.
//!
//! [`EgressRelay::endpoint`] is `relay_for(session)`: a loopback address
//! and the Session's own credentials, which the engine's interception
//! gives the browser when it asks (Chromium's `Fetch.authRequired`), so no
//! other local process can use the relay: a connection without them gets
//! a `407` and no relayed byte. The relay tunnels (`CONNECT`) and forwards
//! absolute-form HTTP requests ([`serve`]), leaving the bytes, the TLS
//! handshake among them, unchanged. A Session with an upstream proxy is
//! chained through it, its credentials sent by the relay; the address
//! check runs before anything goes to the proxy. Every refusal is recorded
//! in the Run trace as a request that failed, and kept for the call whose
//! navigation it failed.
#![expect(
    clippy::disallowed_types,
    reason = "the egress relay opens its own loopback listener and its connections out, in this module only (ADR 0060)"
)]

pub(crate) mod head;
pub(crate) mod serve;

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use data_encoding::BASE64;
use goodfetch_engine::egress::{Policy, Refused};
use goodfetch_engine::endpoint::{REFUSAL_TARGET, new_token};
use goodfetch_engine::http_client::{self, Resolve};
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved as ResolvedSettings;
use goodfetch_protocol::page::RequestFailed;
use goodfetch_protocol::security::SECURITY_RAW_CDP_EGRESS_PROXY;
use goodfetch_protocol::settings::NETWORK_RESOLVE;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

use self::serve::{Met, Network, Observe, Relaying, Resolved, Unreached, Upstream};

/// The user name of every Session's relay credentials; the password is the
/// Session's own.
const USERNAME: &str = "goodfetch";

/// The most refusals a relay keeps for the calls that wait on them.
const KEPT_REFUSALS: usize = 64;

/// The relay of one Session: its loopback address and credentials, closed
/// with every connection it relays when dropped or when its Run stops.
pub(crate) struct EgressRelay {
    address: SocketAddr,
    password: String,
    record: Arc<Record>,
    _closed_when_dropped: DropGuard,
}

/// `relay_for(session)`: where the Session's browser connects, and the
/// credentials it gives.
#[derive(Clone, Copy)]
pub(crate) struct RelayEndpoint<'a> {
    pub(crate) address: SocketAddr,
    pub(crate) username: &'a str,
    pub(crate) password: &'a str,
}

/// How a Session's relay chains through its upstream proxy.
pub(crate) struct Chain {
    /// The protocol the relay speaks to the proxy.
    pub(crate) upstream: Upstream,
    /// The proxy's host and port, or those of the loopback relay that
    /// authenticates to it.
    pub(crate) address: (String, u16),
    /// Whether a URL goes around the proxy, as `proxy.bypass` says.
    pub(crate) bypasses: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// Tells the proxy what a connection to it met.
    pub(crate) met: Box<dyn Fn(Met) + Send + Sync>,
}

/// What the relay met on its latest connection to a Session's upstream
/// proxy: whether it failed to open, and whether the proxy refused the
/// Session's credentials, which the proxy's failures are read from.
#[derive(Default)]
pub(crate) struct Latest {
    unreachable: AtomicBool,
    refused: AtomicBool,
}

impl Latest {
    /// Keeps `met`, a connection's: one that reached no proxy leaves the
    /// credentials' answer as the latest that did.
    pub(crate) fn record(&self, met: Met) {
        self.unreachable
            .store(met == Met::Unreachable, Ordering::Relaxed);
        if met != Met::Unreachable {
            self.refused
                .store(met == Met::CredentialsRefused, Ordering::Relaxed);
        }
    }

    /// Whether the latest connection reached no proxy.
    pub(crate) fn unreachable(&self) -> bool {
        self.unreachable.load(Ordering::Relaxed)
    }

    /// Whether the proxy refused the credentials on the latest connection
    /// that reached it.
    pub(crate) fn refused(&self) -> bool {
        self.refused.load(Ordering::Relaxed)
    }
}

/// Whether `settings`, a Session's, send its browser through the relay:
/// `security.raw_cdp_egress_proxy` on, its default.
pub(crate) fn relayed(settings: &ResolvedSettings) -> bool {
    settings
        .value(SECURITY_RAW_CDP_EGRESS_PROXY.name)
        .as_bool()
        .unwrap_or(true)
}

impl EgressRelay {
    /// Opens the relay of a Session of `run`, chained through `chain` when
    /// the Session has an upstream proxy, binding port 0 of loopback.
    ///
    /// # Errors
    ///
    /// The bind's failure, or the OS generator's.
    pub(crate) async fn open(run: &Run, chain: Option<Chain>) -> io::Result<Self> {
        type Bypasses = Box<dyn Fn(&str) -> bool + Send + Sync>;
        let (upstream, address, bypasses, met): (_, _, Bypasses, _) = match chain {
            None => (Upstream::Direct, None, Box::new(|_: &str| false), None),
            Some(chain) => (
                chain.upstream,
                Some(chain.address),
                chain.bypasses,
                Some(chain.met),
            ),
        };
        let password = new_token().map_err(io::Error::other)?;
        let authorization = format!(
            "Basic {}",
            BASE64.encode(format!("{USERNAME}:{password}").as_bytes())
        );
        let settings = run
            .settings()
            .map(goodfetch_engine::settings::RunSettings::resolved);
        let policy = match settings {
            Some(settings) => Policy::of(settings).map_err(|invalid| {
                io::Error::other(format!("`{}` holds `{}`", invalid.setting, invalid.entry))
            })?,
            None => {
                Policy::new(&["*"], &[] as &[&str]).map_err(|_| io::Error::other("no policy"))?
            }
        };
        let mapping: BTreeMap<String, Vec<IpAddr>> = settings
            .map(|settings| settings.hosts(NETWORK_RESOLVE.name))
            .unwrap_or_default()
            .into_iter()
            .map(|(host, addresses)| (host.to_ascii_lowercase(), addresses))
            .collect();
        let relaying = Relaying {
            policy,
            authorization,
            upstream,
            bypasses,
        };
        let network = Tcp {
            mapping,
            upstream: address,
        };
        let record = Arc::new(Record {
            run: run.clone(),
            log: Arc::new(RequestLog::new(
                run.owner().child(),
                run.folder(),
                run.secrets().mask().clone(),
            )),
            met,
            refusals: Mutex::new(VecDeque::new()),
            unreached: Mutex::new(BTreeMap::new()),
            sequence: AtomicU64::new(0),
        });
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        run.spawn(accept(
            listener,
            Arc::new((relaying, network)),
            Arc::clone(&record),
            run.clone(),
            stop.clone(),
        ));
        Ok(Self {
            address,
            password,
            record,
            _closed_when_dropped: stop.drop_guard(),
        })
    }

    /// `relay_for(session)`: its loopback address and the Session's
    /// credentials.
    pub(crate) fn endpoint(&self) -> RelayEndpoint<'_> {
        RelayEndpoint {
            address: self.address,
            username: USERNAME,
            password: &self.password,
        }
    }

    /// The `proxyServer` the Session's browser context takes.
    pub(crate) fn browser_server(&self) -> String {
        format!("http://{}", self.endpoint().address)
    }

    /// Where the relay's refusals stand now, for [`Self::take_refusal`].
    pub(crate) fn mark(&self) -> u64 {
        self.record.sequence.load(Ordering::Acquire)
    }

    /// Takes the refusal that failed a load of `url`, once: with `mark`,
    /// where the refusals stood as a navigation to `url` began, the latest
    /// since then for `url`'s host and port, else the latest since then, as
    /// the navigation's later hops go elsewhere; without one, the latest
    /// for `url`'s host and port. Each refusal fails one load.
    pub(crate) fn take_refusal(&self, url: &str, mark: Option<u64>) -> Option<Refused> {
        let authority = authority_of(url);
        let mut refusals = lock(&self.record.refusals);
        let matching =
            |entry: &(u64, String, Refused)| authority.as_deref() == Some(entry.1.as_str());
        let at = match mark {
            Some(mark) => refusals
                .iter()
                .rposition(|entry| entry.0 > mark && matching(entry))
                .or_else(|| refusals.iter().rposition(|entry| entry.0 > mark)),
            None => refusals.iter().rposition(matching),
        }?;
        let (_, _, refused) = refusals.remove(at)?;
        Some(Refused {
            url: url.to_owned(),
            ..refused
        })
    }
}

impl EgressRelay {
    /// The browser's own name of the failure of a request to `url` that
    /// failed with `error`, when the relay could not reach its site
    /// itself: the browser sees only the relay's answer, such as
    /// `net::ERR_TUNNEL_CONNECTION_FAILED` or `net::ERR_EMPTY_RESPONSE`,
    /// where it would name the site's failure, such as
    /// `net::ERR_NAME_NOT_RESOLVED`. With `mark`, where the relay stood as
    /// a navigation to `url` began, the latest such failure since then
    /// names a later hop's when `url`'s site was reached.
    pub(crate) fn net_error(
        &self,
        url: &str,
        error: &str,
        mark: Option<u64>,
    ) -> Option<&'static str> {
        let relayed = matches!(
            error.strip_prefix("net::").unwrap_or(error),
            "ERR_TUNNEL_CONNECTION_FAILED" | "ERR_EMPTY_RESPONSE" | "ERR_FAILED"
        );
        if !relayed {
            return None;
        }
        let sites = lock(&self.record.unreached);
        let own = authority_of(url).and_then(|authority| sites.get(&authority).copied());
        let since = || {
            let mark = mark?;
            sites
                .values()
                .filter(|(sequence, _)| *sequence > mark)
                .max_by_key(|(sequence, _)| *sequence)
                .copied()
        };
        own.or_else(since)
            .map(|(_, unreached)| unreached.net_error())
    }
}

/// `url`'s host and port, as the relay keys its refusals.
fn authority_of(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = match url.host()? {
        url::Host::Ipv6(address) => address.to_string(),
        host => host.to_string(),
    };
    Some(head::authority(&host, url.port_or_known_default()?))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a relay records of its connections.
struct Record {
    run: Run,
    log: Arc<RequestLog>,
    /// Tells the Session's upstream proxy what a connection to it met.
    met: Option<Box<dyn Fn(Met) + Send + Sync>>,
    /// Its latest refusals, each with its number and authority.
    refusals: Mutex<VecDeque<(u64, String, Refused)>>,
    /// The sites it could not reach itself since it last reached them,
    /// each with the number of the latest refusal then, by authority.
    unreached: Mutex<BTreeMap<String, (u64, Unreached)>>,
    /// The number of the latest refusal.
    sequence: AtomicU64,
}

impl Observe for Record {
    fn unreached(&self, authority: &str, unreached: Unreached) {
        let mut sites = lock(&self.unreached);
        if sites.len() >= KEPT_REFUSALS && !sites.contains_key(authority) {
            sites.pop_first();
        }
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        sites.insert(authority.to_owned(), (sequence, unreached));
    }

    fn reached(&self, authority: &str) {
        lock(&self.unreached).remove(authority);
    }

    fn refused(&self, refused: &Refused, method: &str) {
        let authority = authority_of(&refused.url).unwrap_or_default();
        {
            let mut refusals = lock(&self.refusals);
            let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
            refusals.push_back((sequence, authority, refused.clone()));
            if refusals.len() > KEPT_REFUSALS {
                refusals.pop_front();
            }
        }
        tracing::debug!(%refused, "the egress relay refused a connection");
        let failure = refused.failure();
        self.run.record_outcome(&failure.code);
        let log = Arc::clone(&self.log);
        let failed = RequestFailed {
            url: refused.url.clone(),
            method: method.to_owned(),
            failure,
        };
        self.run.spawn(async move {
            if let Err(error) = log.request_failed(&failed).await {
                tracing::warn!(%error, "the Run trace did not take a connection the egress relay refused");
            }
        });
    }

    fn upstream(&self, met: Met) {
        if let Some(tell) = &self.met {
            tell(met);
        }
    }

    fn unauthenticated(&self) {
        tracing::warn!(
            target: REFUSAL_TARGET,
            listener = "egress_relay",
            reason = "token_wrong",
            "a connection to a Session's egress relay gave no credentials, or wrong ones, and was refused"
        );
    }
}

/// The relay's network: TCP connections from this host, names resolved
/// through the Run's `network.resolve` mapping, then the system resolver.
struct Tcp {
    mapping: BTreeMap<String, Vec<IpAddr>>,
    /// The upstream proxy's host and port, when there is one.
    upstream: Option<(String, u16)>,
}

impl Network for Tcp {
    type Stream = TcpStream;

    async fn resolve(&self, host: &str, port: u16) -> Option<Resolved> {
        if let Some(mapped) = self.mapping.get(&host.to_ascii_lowercase()) {
            return Some(Resolved {
                addresses: mapped
                    .iter()
                    .map(|address| SocketAddr::new(*address, port))
                    .collect(),
            });
        }
        let addresses = http_client::lookup(host, port, &Resolve::new())
            .await
            .ok()?;
        Some(Resolved { addresses })
    }

    async fn connect(&self, address: SocketAddr) -> io::Result<TcpStream> {
        TcpStream::connect(address).await
    }

    async fn connect_upstream(&self) -> io::Result<TcpStream> {
        let Some((host, port)) = &self.upstream else {
            return Err(io::ErrorKind::NotConnected.into());
        };
        // The upstream proxy is an Operator-configured endpoint: its own
        // address is not judged (ADR 0017).
        let resolved = self
            .resolve(host, *port)
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let mut last = io::Error::from(io::ErrorKind::NotFound);
        for address in resolved.addresses {
            match TcpStream::connect(address).await {
                Ok(stream) => return Ok(stream),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}

/// Accepts the browser's connections until `stop` or the Run's end, each
/// served on a task of its own.
async fn accept(
    listener: TcpListener,
    relay: Arc<(Relaying, Tcp)>,
    record: Arc<Record>,
    run: Run,
    stop: CancellationToken,
) {
    let ending = run.cancellation().clone();
    let clock = run.clock();
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => return,
            () = ending.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let (browser, _) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::debug!(%error, "the egress relay could not accept a connection");
                tokio::select! {
                    () = stop.cancelled() => return,
                    () = ending.cancelled() => return,
                    () = clock.sleep(std::time::Duration::from_millis(50)) => {}
                }
                continue;
            }
        };
        let (relay, record, stop, ending) = (
            Arc::clone(&relay),
            Arc::clone(&record),
            stop.clone(),
            ending.clone(),
        );
        run.spawn(async move {
            let (relaying, network) = &*relay;
            tokio::select! {
                _ = serve::serve(browser, relaying, network, &*record) => {}
                () = stop.cancelled() => {}
                () = ending.cancelled() => {}
            }
        });
    }
}

#[cfg(test)]
mod tests;
