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
//! handshake among them, unchanged; it notes the browser's ClientHello as
//! it passes, which its own look at a certificate the browser refused
//! sends again, so the site sees the browser's fingerprint alone. A
//! Session with an upstream proxy is chained through it, its credentials
//! sent by the relay; the address check runs before anything goes to the
//! proxy. Every refusal is recorded in the Run trace as a request that
//! failed, and kept for the call whose navigation it failed.
//!
//! A Session with local addresses (ADR 0044) goes through its relay
//! whatever `security.raw_cdp_egress_proxy` says: each connection the
//! relay opens, to a site or to the upstream proxy, leaves from the
//! Session's address of its family, and each name it resolves is asked
//! through the engine's own resolver from that address, so nothing the
//! browser sends reveals the machine's other addresses.
//!
//! A host the Run lists for an invalid-certificate exception (ADR 0042)
//! is terminated instead ([`terminate`]): the relay presents the browser a
//! leaf of the Run's in-memory [`authority`], which Chromium trusts by its
//! key's hash, and connects upstream through BoringSSL with the browser's
//! own ClientHello, under the Run's pins. Each host it terminated is
//! recorded once in the Run's Provenance with the Session's relay, the
//! exception it passed through and the key the host presented; a pin that
//! changed stops the host with `certificate_changed` before any byte of the
//! browser's goes up.
#![expect(
    clippy::disallowed_types,
    reason = "the egress relay opens its own loopback listener and its connections out, in this module only (ADR 0060)"
)]

pub(crate) mod application_settings;
pub(crate) mod authority;
pub(crate) mod capture;
pub(crate) mod client_hello;
pub(crate) mod head;
pub(crate) mod serve;
pub(crate) mod terminate;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use data_encoding::BASE64;
use goodfetch_engine::CallError;
use goodfetch_engine::dns::Dns;
use goodfetch_engine::egress::{self, EndpointPolicy, Policy, Refused};
use goodfetch_engine::endpoint::{REFUSAL_TARGET, new_token};
use goodfetch_engine::http_client::{self, LocalBinding, Resolve};
use goodfetch_engine::local_addresses::SessionAddresses;
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved as ResolvedSettings;
use goodfetch_engine::tls::{
    CertificateExceptions, CertificateInvalid, Examiner, ExceptionKind, ExceptionUsed, Trust,
};
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::page::RequestFailed;
use goodfetch_protocol::security::SECURITY_RAW_CDP_EGRESS_PROXY;
use goodfetch_protocol::settings::NETWORK_RESOLVE;
use serde_json::json;
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, DropGuard};

use self::application_settings::ApplicationSettings;
use self::authority::Authority;
use self::client_hello::ClientHello;
use self::serve::{Met, Network, Observe, ProxyRefused, Relaying, Resolved, Unreached, Upstream};
use self::terminate::{Stop, Terminating};

/// The user name of every Session's relay credentials; the password is the
/// Session's own.
const USERNAME: &str = "goodfetch";

/// The most refusals a relay keeps for the calls that wait on them.
const KEPT_REFUSALS: usize = 64;

/// The most ClientHello messages a relay keeps, the latest of each
/// authority, for its own look at a certificate the browser refused.
const KEPT_CLIENT_HELLOS: usize = 64;

/// The relay of one Session: its loopback address and credentials, closed
/// with every connection it relays when dropped or when its Run stops.
pub(crate) struct EgressRelay {
    address: SocketAddr,
    password: String,
    authorization: String,
    record: Arc<Record>,
    /// What it terminates, when the Run lists a certificate exception.
    terminating: Option<Arc<Terminating>>,
    /// The authorities the Run trusts by domain.
    trust: Trust,
    /// The ALPS payload of the Run's browser build.
    application_settings: Arc<ApplicationSettings>,
    /// The per-host decision on a certificate the browser refused, made
    /// on its first use.
    examiner: OnceLock<Examiner>,
    /// The local addresses its connections leave from, when the Session
    /// has them.
    local: Option<SessionAddresses>,
    _closed_when_dropped: DropGuard,
}

/// A Session's local addresses at its relay (ADR 0044): the address of
/// each family its connections bind, and the engine's resolution, whose
/// queries the relay sends from them.
pub(crate) struct Local {
    pub(crate) addresses: SessionAddresses,
    pub(crate) dns: Dns,
}

/// What a Session's relay takes of its Run's TLS settings (ADR 0042): the
/// Run's authority, when it lists an exception, its exceptions and the
/// authorities its `tls.trusted_cas` trusts.
pub(crate) struct RelayTls {
    pub(crate) authority: Option<Arc<Authority>>,
    pub(crate) exceptions: CertificateExceptions,
    pub(crate) trust: Trust,
    /// The ALPS payload of the Run's browser build, captured when it was
    /// pinned, which the relay's own handshakes send.
    pub(crate) application_settings: Arc<ApplicationSettings>,
}

/// A connection the relay refused or stopped before any byte of the
/// browser's left for its site, kept for the call whose load it failed.
#[derive(Clone, Debug)]
pub(crate) enum Refusal {
    /// The egress policy refused it.
    Egress(Refused),
    /// The terminating mode stopped it on its certificate.
    Certificate(Stop),
}

impl Refusal {
    /// The failure the call that waited for the load answers with.
    pub(crate) fn error(&self) -> CallError {
        match self {
            Self::Egress(refused) => refused.error(),
            Self::Certificate(Stop::Changed(changed)) => changed.error(),
            Self::Certificate(Stop::Invalid(invalid)) => invalid.error(),
        }
    }

    /// The failure as a request's event carries it.
    fn failure(&self) -> ErrorData {
        match self {
            Self::Egress(refused) => refused.failure(),
            Self::Certificate(_) => {
                let error = self.error();
                ErrorData {
                    code: error.code().to_owned(),
                    retryable: error.retryable(),
                    details: error.details().cloned().unwrap_or_default(),
                }
            }
        }
    }
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
    /// How the proxy's own addresses are judged before each connection to
    /// it, as an Operator-configured endpoint's (ADR 0017); none for the
    /// Session's loopback relay, which the Session's open judged the proxy
    /// behind.
    pub(crate) judged: Option<EndpointPolicy>,
}

/// What the relay met on its latest connection to a Session's upstream
/// proxy: whether it failed to open, and whether the proxy refused the
/// Session's credentials, which the proxy's failures are read from.
#[derive(Default)]
pub(crate) struct Latest {
    unreachable: AtomicBool,
    refused: AtomicBool,
    /// Whether the relay answered the latest connection itself, with its
    /// own `502` for a SOCKS5 proxy it could not use.
    answered: AtomicBool,
}

impl Latest {
    /// Keeps `met`, a connection's: one that reached no proxy leaves the
    /// credentials' answer as the latest that did.
    pub(crate) fn record(&self, met: Met) {
        self.unreachable
            .store(met == Met::Unreachable, Ordering::Relaxed);
        self.answered.store(met != Met::Reached, Ordering::Relaxed);
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

    /// Whether the relay answered the latest connection itself: a proxy
    /// it could not reach, or one that refused what it asked.
    pub(crate) fn answered(&self) -> bool {
        self.answered.load(Ordering::Relaxed)
    }
}

/// The egress policy of a Session whose Run resolved `settings`, every
/// host allowed for a Run with none.
///
/// # Errors
///
/// An entry of a domain or private-range setting that is none.
fn session_policy(settings: Option<&ResolvedSettings>) -> io::Result<Policy> {
    match settings {
        Some(settings) => Policy::of(settings).map_err(|invalid| {
            io::Error::other(format!("`{}` holds `{}`", invalid.setting, invalid.entry))
        }),
        None => Policy::new(&["*"], &[] as &[&str]).map_err(|_| io::Error::other("no policy")),
    }
}

/// The `network.resolve` mapping of a Session whose Run resolved
/// `settings`, by lowercase host.
fn mapping_of(settings: Option<&ResolvedSettings>) -> BTreeMap<String, Vec<IpAddr>> {
    settings
        .map(|settings| settings.hosts(NETWORK_RESOLVE.name))
        .unwrap_or_default()
        .into_iter()
        .map(|(host, addresses)| (host.to_ascii_lowercase(), addresses))
        .collect()
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
    /// the Session has an upstream proxy, its connections leaving from
    /// `local`'s addresses when it has them, binding port 0 of loopback.
    ///
    /// # Errors
    ///
    /// The bind's failure, or the OS generator's.
    pub(crate) async fn open(
        run: &Run,
        chain: Option<Chain>,
        tls: RelayTls,
        local: Option<Local>,
    ) -> io::Result<Self> {
        type Bypasses = Box<dyn Fn(&str) -> bool + Send + Sync>;
        let (upstream, address, bypasses, met, judged): (_, _, Bypasses, _, _) = match chain {
            None => (
                Upstream::Direct,
                None,
                Box::new(|_: &str| false),
                None,
                None,
            ),
            Some(chain) => (
                chain.upstream,
                Some(chain.address),
                chain.bypasses,
                Some(chain.met),
                chain.judged,
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
        let policy = session_policy(settings)?;
        let mapping = mapping_of(settings);
        let RelayTls {
            authority,
            exceptions,
            trust,
            application_settings,
        } = tls;
        let terminating = authority.map(|authority| {
            Arc::new(Terminating {
                authority,
                exceptions,
                examiner: trust.examiner(),
                trust: trust.clone(),
                application_settings: Arc::clone(&application_settings),
                sessions: Arc::default(),
            })
        });
        let relaying = Relaying {
            policy,
            authorization: authorization.clone(),
            upstream,
            bypasses,
            terminating: terminating.clone(),
        };
        let addresses = local.as_ref().map(|local| local.addresses);
        let network = Tcp {
            mapping,
            upstream: address,
            judged,
            local: local.map(|local| LocalBinding::new(local.addresses, &local.dns)),
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let record = Arc::new(Record {
            address,
            terminated: Mutex::new(BTreeSet::new()),
            run: run.clone(),
            log: Arc::new(RequestLog::new(
                run.owner().child(),
                run.folder(),
                run.secrets().mask().clone(),
            )),
            met,
            refusals: Mutex::new(VecDeque::new()),
            unreached: Mutex::new(BTreeMap::new()),
            client_hellos: Mutex::new(VecDeque::new()),
            sequence: AtomicU64::new(0),
        });
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
            authorization,
            record,
            terminating,
            trust,
            application_settings,
            examiner: OnceLock::new(),
            local: addresses,
            _closed_when_dropped: stop.drop_guard(),
        })
    }

    /// The local addresses its connections leave from, when the Session
    /// has them.
    pub(crate) fn local_addresses(&self) -> Option<SessionAddresses> {
        self.local
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
    pub(crate) fn take_refusal(&self, url: &str, mark: Option<u64>) -> Option<Refusal> {
        let authority = authority_of(url);
        let mut refusals = lock(&self.record.refusals);
        let matching =
            |entry: &(u64, String, Refusal)| authority.as_deref() == Some(entry.1.as_str());
        let at = match mark {
            Some(mark) => refusals
                .iter()
                .rposition(|entry| entry.0 > mark && matching(entry))
                .or_else(|| refusals.iter().rposition(|entry| entry.0 > mark)),
            None => refusals.iter().rposition(matching),
        }?;
        let (_, _, refusal) = refusals.remove(at)?;
        Some(match refusal {
            Refusal::Egress(refused) => Refusal::Egress(Refused {
                url: url.to_owned(),
                ..refused
            }),
            certificate @ Refusal::Certificate(_) => certificate,
        })
    }

    /// Whether the relay terminates `host`'s connections: its domain is
    /// listed for an invalid-certificate exception.
    pub(crate) fn terminates(&self, host: &str) -> bool {
        self.terminating
            .as_deref()
            .is_some_and(|terminating| terminating.terminates(host))
    }

    /// How the certificate of `url`'s host fails the engine's per-host
    /// decision, for a load the browser refused on it: the relay opens a
    /// handshake of its own to the host, through itself, so the same
    /// egress checks and upstream proxy hold, with the ClientHello the
    /// browser last sent that host through the relay, so the site sees the
    /// browser's fingerprint alone (ADR 0042), and sends nothing past it.
    /// None when the certificate passes, the relay saw no ClientHello of
    /// the browser's for the host or cannot send all of it again
    /// ([`terminate::mirrors`]), or no handshake opens: the browser's own
    /// name of the failure stands then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes its connection.
    pub(crate) async fn examine(&self, url: &str) -> Option<CertificateInvalid> {
        let url = url::Url::parse(url).ok()?;
        let host = match url.host()? {
            url::Host::Ipv6(address) => address.to_string(),
            host => host.to_string(),
        };
        let authority = head::authority(&host, url.port_or_known_default()?);
        let hello = self
            .record
            .client_hello_of(&authority)
            .filter(terminate::mirrors)?;
        let mut stream = TcpStream::connect(self.address).await.ok()?;
        let request = format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: {}\r\n\r\n",
            self.authorization
        );
        stream.write_all(request.as_bytes()).await.ok()?;
        let answer = head::answer(&mut stream).await.ok()?;
        if !(200..300).contains(&answer.status) {
            return None;
        }
        let examiner = self.examiner.get_or_init(|| self.trust.examiner());
        let mirror = terminate::Mirror {
            hello: &hello,
            application_settings: &self.application_settings,
            sessions: None,
        };
        let chain = match terminate::upstream(stream, &host, Some(mirror)).await {
            Ok(upstream) => Some(upstream.chain),
            Err((chain, _)) => chain,
        }?;
        let failure = examiner.examine(&host, &chain).ok()?.failure?;
        Some(CertificateInvalid { host, failure })
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
    /// The relay's own address, which names its Session in Provenance.
    address: SocketAddr,
    /// The hosts it terminated, each recorded once.
    terminated: Mutex<BTreeSet<String>>,
    run: Run,
    log: Arc<RequestLog>,
    /// Tells the Session's upstream proxy what a connection to it met.
    met: Option<Box<dyn Fn(Met) + Send + Sync>>,
    /// Its latest refusals, each with its number and authority.
    refusals: Mutex<VecDeque<(u64, String, Refusal)>>,
    /// The sites it could not reach itself since it last reached them,
    /// each with the number of the latest refusal then, by authority.
    unreached: Mutex<BTreeMap<String, (u64, Unreached)>>,
    /// The number of the latest refusal.
    sequence: AtomicU64,
    /// The latest ClientHello the browser sent on a tunnel the relay
    /// passed as it is, of each authority, the latest last.
    client_hellos: Mutex<VecDeque<(String, ClientHello)>>,
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
        tracing::debug!(%refused, "the egress relay refused a connection");
        self.keep(&refused.url, method, Refusal::Egress(refused.clone()));
    }

    fn terminated(&self, host: &str, used: Option<&ExceptionUsed>) {
        if !lock(&self.terminated).insert(host.to_owned()) {
            return;
        }
        let exception = used.map(|used| match used.kind {
            ExceptionKind::TrustOnFirstUse => "trust_on_first_use",
            ExceptionKind::Any => "any",
            _ => "other",
        });
        let entry = json!({
            "action": TLS_TERMINATED,
            "relay": self.address.to_string(),
            "host": host,
            "exception": exception,
            "spki_sha256": used.map(|used| used.certificate.to_string()),
        });
        let run = self.run.clone();
        self.run.spawn(async move {
            if let Err(error) = goodfetch_engine::run_trace::provenance(
                run.owner(),
                run.folder(),
                run.secrets().mask(),
                &entry,
            )
            .await
            {
                tracing::warn!(%error, "the Run's Provenance did not take a host the egress relay terminated");
            }
        });
    }

    fn client_hello(&self, authority: &str, hello: ClientHello) {
        let mut hellos = lock(&self.client_hellos);
        hellos.retain(|(kept, _)| kept != authority);
        hellos.push_back((authority.to_owned(), hello));
        if hellos.len() > KEPT_CLIENT_HELLOS {
            hellos.pop_front();
        }
    }

    fn stopped(&self, url: &str, method: &str, stop: &Stop) {
        tracing::debug!(?stop, "the egress relay stopped a terminated connection");
        self.keep(url, method, Refusal::Certificate(stop.clone()));
    }

    fn upstream(&self, met: Met) {
        if let Some(tell) = &self.met {
            tell(met);
        }
    }

    fn unauthenticated(&self, given: bool) {
        // Every client's first request, the browser's included, carries no
        // credentials until the relay asks for them: only wrong ones are a
        // refusal, recorded in the Audit log.
        if !given {
            tracing::debug!(
                target: REFUSAL_TARGET,
                listener = "egress_relay",
                "a connection to a Session's egress relay gave no credentials and was asked for them"
            );
            return;
        }
        let refusals = self.run.refusals();
        self.run.spawn(async move {
            refusals.refused("egress_relay", "token_wrong").await;
        });
    }
}

/// What a Session's Provenance names a host its relay terminated by.
const TLS_TERMINATED: &str = "tls_terminated";

impl Record {
    /// The latest ClientHello the browser sent on a tunnel to `authority`
    /// that the relay passed as it is.
    fn client_hello_of(&self, authority: &str) -> Option<ClientHello> {
        lock(&self.client_hellos)
            .iter()
            .rev()
            .find(|(kept, _)| kept == authority)
            .map(|(_, hello)| hello.clone())
    }

    /// Keeps `refusal` of a load of `url` that `method` asked for, for the
    /// call it fails, and records it in the Run trace.
    fn keep(&self, url: &str, method: &str, refusal: Refusal) {
        let authority = authority_of(url).unwrap_or_default();
        let failure = refusal.failure();
        {
            let mut refusals = lock(&self.refusals);
            let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
            refusals.push_back((sequence, authority, refusal));
            if refusals.len() > KEPT_REFUSALS {
                refusals.pop_front();
            }
        }
        self.run.record_outcome(&failure.code);
        let log = Arc::clone(&self.log);
        let failed = RequestFailed {
            url: url.to_owned(),
            method: method.to_owned(),
            failure,
        };
        self.run.spawn(async move {
            if let Err(error) = log.request_failed(&failed).await {
                tracing::warn!(%error, "the Run trace did not take a connection the egress relay refused");
            }
        });
    }
}

/// The relay's network: TCP connections from this host, names resolved
/// through the Run's `network.resolve` mapping, then the system resolver;
/// with the Session's local addresses, each connection bound to its address
/// of the family and each name resolved by the engine's own resolver from
/// them (ADR 0044).
struct Tcp {
    mapping: BTreeMap<String, Vec<IpAddr>>,
    /// The upstream proxy's host and port, when there is one.
    upstream: Option<(String, u16)>,
    /// How the upstream proxy's own addresses are judged when the relay
    /// connects to the proxy itself.
    judged: Option<EndpointPolicy>,
    /// The Session's local addresses, when it has them.
    local: Option<LocalBinding>,
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
        let addresses = match &self.local {
            Some(local) => local.lookup(host, port, &Resolve::new()).await,
            None => http_client::lookup(host, port, &Resolve::new()).await,
        }
        .ok()?;
        Some(Resolved { addresses })
    }

    async fn connect(&self, address: SocketAddr) -> io::Result<TcpStream> {
        match &self.local {
            Some(local) => local.connect(address).await,
            None => TcpStream::connect(address).await,
        }
    }

    async fn connect_upstream(&self) -> io::Result<TcpStream> {
        let Some((host, port)) = &self.upstream else {
            return Err(io::ErrorKind::NotConnected.into());
        };
        // The proxy's own addresses are judged as an Operator-configured
        // endpoint's before any connection, so an answer that changed since
        // the Session's open reaches nothing (ADR 0017).
        let resolved = self
            .resolve(host, *port)
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        if let Some(judged) = self.judged {
            let url = if host.contains(':') {
                format!("http://[{host}]:{port}")
            } else {
                format!("http://{host}:{port}")
            };
            egress::judge_endpoint_addresses(&url, &resolved.addresses, judged).map_err(
                |refused| io::Error::new(io::ErrorKind::PermissionDenied, ProxyRefused(refused)),
            )?;
        }
        let mut last = io::Error::from(io::ErrorKind::NotFound);
        for address in resolved.addresses {
            match self.connect(address).await {
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

/// The fuzz target of the relay's request heads (ADR 0067): any bytes, read
/// as a browser's head and forwarded, never panic.
pub fn fuzz_request_head(data: &[u8]) {
    if let Ok(Some((head, _))) = head::parse(data) {
        let _ = head.target.url();
        let _ = head.target.authority();
        let _ = head.forwarded(true, Some("Basic Z29vZGZldGNoOng="));
    }
}

/// The fuzz target of the ClientHello the relay mirrors (ADR 0067): any
/// bytes, read as the records of a browser's ClientHello, never panic.
pub fn fuzz_client_hello(data: &[u8]) {
    let _ = client_hello::parse(data);
}

#[cfg(test)]
mod terminate_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod request_head_fuzz {
    goodfetch_test_support::fuzz_replay!(
        crate::egress_relay::fuzz_request_head,
        "fuzz/relay_request_head"
    );
}

#[cfg(test)]
mod client_hello_fuzz {
    goodfetch_test_support::fuzz_replay!(
        crate::egress_relay::fuzz_client_hello,
        "fuzz/relay_client_hello"
    );
}
