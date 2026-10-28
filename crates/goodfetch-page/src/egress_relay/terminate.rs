//! The egress relay's terminating mode (ADRs 0042 and 0124), for a host a
//! certificate exception lists. The relay reads the browser's ClientHello
//! before it answers, opens its upstream handshake through BoringSSL with
//! the same values (cipher suites, groups and key shares, signature
//! algorithms, ALPN, ALPS, certificate compression, OCSP and SCT requests,
//! `record_size_limit`, delegated credentials, trust anchor IDs, GREASE,
//! ECH GREASE and the extension order), and hands the chain the site presents to the
//! engine's per-host decision and the Run's exception, before any byte of
//! the browser's goes up. Only then does it finish the browser's handshake
//! with a leaf of its in-memory authority, under the protocol the site
//! chose, and copy the decrypted bytes both ways unchanged: the site sees
//! the browser's own HTTP/2 or HTTP/1.1, and the relay links no HTTP stack.
//!
//! A host an exception lists, which no Session cookie may reach (ADR 0042),
//! is held to HTTP/1.1 on both sides, the one protocol the upstream
//! handshake offers it, and each request head is read before it goes up:
//! the first that carries a `Cookie` header ends the connection
//! ([`cookies`]), a WebSocket handshake's as any other's.
//!
//! The relay is a translation point (ADR 0127): how the site's handshake
//! failed becomes the `certificate_invalid` failure [`failure_of`] names.

#![expect(
    clippy::disallowed_types,
    reason = "a handshake's verify callback keeps the chain it saw for the task that waits on the handshake, never across another lock or an `.await` (ADR 0060)"
)]

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use btls::pkey::PKey;
use btls::ssl::{
    self, AlpnError, CertificateCompressionAlgorithm, CertificateCompressor, ExtensionType,
    KeyShare, SslAcceptor, SslConnector, SslContextBuilder, SslMethod, SslOptions, SslSession,
    SslSessionCacheMode, SslVerifyMode, SslVersion,
};
use btls::x509::X509;
use foreign_types::ForeignTypeRef as _;
use goodfetch_engine::tls::{
    CertificateChanged, CertificateExceptions, CertificateInvalid, Examiner, ExceptionUsed, Trust,
};
use goodfetch_protocol::tls::CertificateFailure;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};
use tokio_btls::SslStream;

use super::application_settings::{self, ApplicationSettings};
use super::authority::Authority;
use super::client_hello::{self, ClientHello};
use super::cookies::{self, Guarded, HTTP_1_1};

/// What a Session's relay terminates, and how it decides upstream.
pub(crate) struct Terminating {
    /// The Run's authority, which signs each listed host's leaf.
    pub(crate) authority: Arc<Authority>,
    /// The Run's exceptions: the hosts it lists are terminated, and its
    /// pins decide.
    pub(crate) exceptions: CertificateExceptions,
    /// The authorities the Run trusts by domain: the hosts it lists are
    /// terminated too, since a browser cannot trust an authority for some
    /// hosts alone.
    pub(crate) trust: Trust,
    /// The per-host decision on each chain a site presents, under `trust`.
    pub(crate) examiner: Examiner,
    /// The ALPS payload of the Run's browser build, captured when the
    /// build was pinned; empty for a build with none.
    pub(crate) application_settings: Arc<ApplicationSettings>,
    /// The Session's upstream TLS sessions, which a returning connection
    /// resumes; dropped with the Session's relay.
    pub(crate) sessions: Arc<Sessions>,
}

/// The upstream TLS sessions of one Session's relay, the latest of each
/// host, at most [`KEPT_SESSIONS`].
#[derive(Default)]
pub(crate) struct Sessions(Mutex<VecDeque<(String, SslSession)>>);

/// The most upstream sessions a relay keeps (STYLE.md: every buffer is
/// bounded).
const KEPT_SESSIONS: usize = 64;

impl Sessions {
    fn kept(&self) -> MutexGuard<'_, VecDeque<(String, SslSession)>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The latest session of `host`.
    fn of(&self, host: &str) -> Option<SslSession> {
        self.kept()
            .iter()
            .find(|(kept, _)| kept == host)
            .map(|(_, session)| session.clone())
    }

    /// Keeps `session` as `host`'s latest.
    fn keep(&self, host: &str, session: SslSession) {
        let mut kept = self.kept();
        kept.retain(|(kept, _)| kept != host);
        if kept.len() >= KEPT_SESSIONS {
            kept.pop_front();
        }
        kept.push_back((host.to_owned(), session));
    }
}

impl Terminating {
    /// Whether the relay terminates `host`: an exception or a trusted
    /// authority lists its domain.
    pub(crate) fn terminates(&self, host: &str) -> bool {
        self.exceptions.covers(host) || self.trust.scope_of(host).is_some()
    }
}

/// Why the relay ended a terminated connection before any byte of the
/// browser's went up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// A pinned host presented another key, or its pin cannot be read.
    Changed(CertificateChanged),
    /// The site's certificate or handshake failed, with no exception.
    Invalid(CertificateInvalid),
}

/// How a terminated connection went.
#[derive(Debug)]
pub(crate) enum Terminated {
    /// It passed, and its bytes went both ways: these many from the
    /// browser and to it.
    Relayed { up: u64, down: u64 },
    /// The relay stopped it, and told why first; or a request to a host an
    /// exception lists carried a Session cookie, which never went up.
    Stopped,
    /// It ended otherwise: the browser sent no ClientHello, or a side
    /// closed.
    Failed,
}

/// The translation point of the site's TLS failures (ADR 0127): the
/// failure the per-host decision named, else BoringSSL's reason for the
/// handshake's end, as text.
pub(crate) fn failure_of(
    examined: Option<CertificateFailure>,
    handshake: Option<&ssl::Error>,
) -> CertificateFailure {
    if let Some(failure) = examined {
        return failure;
    }
    let reason = handshake
        .and_then(ssl::Error::ssl_error)
        .and_then(|stack| stack.errors().first())
        .and_then(btls::error::Error::reason)
        .map(|reason| reason.to_ascii_lowercase().replace([' ', '-'], "_"));
    CertificateFailure::Unrecognized(reason.unwrap_or_else(|| "handshake_failed".to_owned()))
}

/// Terminates the browser's TLS connection to `host` over `site`, a
/// connection already open to it, `rest` the browser's bytes read past its
/// head, as [`Terminating`] says; `passed` learns, before any byte of the
/// browser's goes up, that the site's certificate passed, and through which
/// exception, and `stopped` why the relay stopped the connection, before the
/// browser's end of it closes: the browser fails its load only once the
/// Session keeps the refusal, so the call waiting on that load finds it.
///
/// # Cancel safety
///
/// Dropping the future closes both connections; a pin it was writing may
/// lose its Audit log entry.
pub(crate) async fn terminate<B, S>(
    browser: B,
    site: S,
    (rest, host): (Vec<u8>, &str),
    terminating: &Terminating,
    passed: impl FnOnce(Option<&ExceptionUsed>),
    stopped: impl FnOnce(&Stop),
) -> Terminated
where
    B: AsyncRead + AsyncWrite + Unpin + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let examiner = &terminating.examiner;
    let mut browser = Replayed::new(browser);
    browser.replay(rest);
    let Ok((hello, raw)) = client_hello::read(&mut browser).await else {
        return Terminated::Failed;
    };
    browser.replay(raw);
    // A host an exception lists is held to HTTP/1.1, whose request heads
    // the relay reads for Session cookies (ADR 0042).
    let guarded = terminating.exceptions.covers(host);
    let held = guarded.then(|| http_1_1_only(&hello));
    let mirror = Mirror {
        hello: held.as_ref().unwrap_or(&hello),
        application_settings: &terminating.application_settings,
        sessions: Some(&terminating.sessions),
    };
    let mut up = match upstream(site, host, Some(mirror)).await {
        Ok(up) => up,
        Err((chain, error)) => {
            let failure = chain
                .as_deref()
                .and_then(|chain| examiner.examine(host, chain).ok())
                .and_then(|presented| presented.failure);
            let stop = Stop::Invalid(CertificateInvalid {
                host: host.to_owned(),
                failure: failure_of(failure, error.as_ref()),
            });
            stopped(&stop);
            return Terminated::Stopped;
        }
    };
    let used = match decide(host, &up.chain, terminating, examiner).await {
        Ok(used) => used,
        Err(stop) => {
            stopped(&stop);
            return Terminated::Stopped;
        }
    };
    passed(used.as_ref());
    let chosen = up.stream.ssl().selected_alpn_protocol().map(<[u8]>::to_vec);
    let Ok(mut browser) = accept(browser, host, &terminating.authority, chosen).await else {
        return Terminated::Failed;
    };
    if !guarded {
        let (up, down) = both_ways(&mut browser, &mut up.stream).await;
        return Terminated::Relayed { up, down };
    }
    match without_cookies(&mut browser, &mut up.stream).await {
        Some((up, down)) => Terminated::Relayed { up, down },
        None => {
            tracing::warn!(
                host,
                "the egress relay ended a connection whose request carried a Session cookie to a host reached through an invalid-certificate exception (ADR 0042)"
            );
            Terminated::Stopped
        }
    }
}

/// `hello`, offering HTTP/1.1 alone, as the relay offers a host an
/// exception lists: no other protocol, and no ALPS, which only HTTP/2 uses.
fn http_1_1_only(hello: &ClientHello) -> ClientHello {
    ClientHello {
        alpn: hello
            .alpn
            .iter()
            .filter(|protocol| protocol.as_slice() == HTTP_1_1)
            .cloned()
            .collect(),
        alps: Vec::new(),
        ..hello.clone()
    }
}

/// Copies the decrypted bytes both ways as [`both_ways`] does, each of the
/// browser's request heads read before it goes up ([`cookies`]): the bytes
/// copied from `browser` and to it; none when a request carried a Session
/// cookie, and the connection ends there.
async fn without_cookies<B, S>(browser: &mut B, site: &mut S) -> Option<(u64, u64)>
where
    B: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut browser_reads, mut browser_writes) = tokio::io::split(browser);
    let (mut site_reads, mut site_writes) = tokio::io::split(site);
    let up = cookies::requests_without_cookies(&mut browser_reads, &mut site_writes);
    let down = one_way(&mut site_reads, &mut browser_writes);
    tokio::pin!(up, down);
    let mut copied = None;
    let guarded = loop {
        tokio::select! {
            guarded = &mut up => break guarded,
            down = &mut down, if copied.is_none() => copied = Some(down),
        }
    };
    match guarded {
        Guarded::Withheld => None,
        Guarded::Ended(sent) => Some((
            sent,
            match copied {
                Some(copied) => copied,
                None => down.await,
            },
        )),
    }
}

/// Copies the decrypted bytes both ways until each side ends, by a close
/// or an error, as a browser often closes with no TLS close: the bytes
/// copied from `browser` and to it.
async fn both_ways<B, S>(browser: &mut B, site: &mut S) -> (u64, u64)
where
    B: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut browser_reads, mut browser_writes) = tokio::io::split(browser);
    let (mut site_reads, mut site_writes) = tokio::io::split(site);
    tokio::join!(
        one_way(&mut browser_reads, &mut site_writes),
        one_way(&mut site_reads, &mut browser_writes),
    )
}

/// Copies `from` to `to` until `from` ends or either fails, then closes
/// `to`: the bytes copied.
async fn one_way(from: &mut (impl AsyncRead + Unpin), to: &mut (impl AsyncWrite + Unpin)) -> u64 {
    let mut buffer = vec![0; 16 * 1024];
    let mut copied = 0_u64;
    loop {
        let count = match from.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        if to.write_all(&buffer[..count]).await.is_err() {
            break;
        }
        copied += u64::try_from(count).unwrap_or(u64::MAX);
    }
    let _ = to.shutdown().await;
    copied
}

/// The Run's decision on `chain`, `host`'s: the exception it passes
/// through, if any.
async fn decide(
    host: &str,
    chain: &[Vec<u8>],
    terminating: &Terminating,
    examiner: &Examiner,
) -> Result<Option<ExceptionUsed>, Stop> {
    let presented = examiner.examine(host, chain).map_err(|_| {
        Stop::Invalid(CertificateInvalid {
            host: host.to_owned(),
            failure: CertificateFailure::Unrecognized("unreadable_certificate".to_owned()),
        })
    })?;
    if terminating.exceptions.covers(host) {
        return terminating
            .exceptions
            .admit(host, presented)
            .await
            .map_err(Stop::Changed);
    }
    match presented.failure {
        None => Ok(None),
        Some(failure) => Err(Stop::Invalid(CertificateInvalid {
            host: host.to_owned(),
            failure,
        })),
    }
}

/// An upstream connection whose handshake finished: the chain its site
/// presented, leaf first.
pub(crate) struct Upstream<S> {
    pub(crate) stream: SslStream<S>,
    pub(crate) chain: Vec<Vec<u8>>,
}

/// Why an upstream handshake did not finish: the chain the site presented,
/// if it got that far, and BoringSSL's error.
pub(crate) type Unfinished = (Option<Vec<Vec<u8>>>, Option<ssl::Error>);

/// What an upstream handshake mirrors: the browser's ClientHello, the ALPS
/// payload of its build, and the Session's upstream sessions, which it
/// resumes and keeps, when it has them.
#[derive(Clone, Copy)]
pub(crate) struct Mirror<'a> {
    pub(crate) hello: &'a ClientHello,
    pub(crate) application_settings: &'a ApplicationSettings,
    pub(crate) sessions: Option<&'a Arc<Sessions>>,
}

/// Opens the upstream handshake to `host` over `site`, with the values of
/// `mirror`, the browser's, or BoringSSL's own without one. Every chain
/// passes BoringSSL: the per-host decision reads it once the handshake is
/// done, before anything is sent. A resumed session's chain is the one its
/// first handshake kept, which the decision reads again.
///
/// # Errors
///
/// [`Unfinished`] when the handshake fails.
pub(crate) async fn upstream<S>(
    site: S,
    host: &str,
    mirror: Option<Mirror<'_>>,
) -> Result<Upstream<S>, Unfinished>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let seen: Arc<Mutex<Option<Vec<Vec<u8>>>>> = Arc::default();
    let hello = mirror.map(|mirror| mirror.hello);
    let sessions = mirror.and_then(|mirror| mirror.sessions);
    let ssl = connector(
        hello,
        Arc::clone(&seen),
        sessions.map(|sessions| (sessions, host)),
    )
    .and_then(|connector| {
        let mut configuration = connector.configure()?;
        configuration.set_verify_hostname(false);
        configuration.set_use_server_name_indication(host.parse::<std::net::IpAddr>().is_err());
        if let Some(Mirror {
            hello,
            application_settings,
            ..
        }) = mirror
        {
            configuration.set_enable_ech_grease(hello.ech_grease());
            let shares: Vec<KeyShare> = hello
                .key_shares
                .iter()
                .filter_map(|group| key_share(*group))
                .collect();
            if !shares.is_empty() {
                configuration.set_client_key_shares(&shares)?;
            }
            for protocol in &hello.alps {
                let payload = std::str::from_utf8(protocol)
                    .ok()
                    .and_then(|protocol| application_settings.get(protocol))
                    .map_or(&[][..], Vec::as_slice);
                application_settings::offer(&mut configuration, protocol, payload)?;
            }
            if !hello.alps.is_empty() {
                configuration.set_alps_use_new_codepoint(hello.alps_new_codepoint);
            }
        }
        let mut ssl = configuration.into_ssl(host)?;
        if let Some(session) = sessions.and_then(|sessions| sessions.of(host)) {
            resume(&mut ssl, &session)?;
        }
        Ok(ssl)
    })
    .map_err(|error| (None, Some(ssl::Error::from(error))))?;
    let taken = || seen.lock().unwrap_or_else(PoisonError::into_inner).take();
    let mut stream = SslStream::new(ssl, site).map_err(|error| (None, Some(error.into())))?;
    match Pin::new(&mut stream).connect().await {
        Ok(()) => match taken().or_else(|| resumed_chain(stream.ssl())) {
            Some(chain) => Ok(Upstream { stream, chain }),
            None => Err((None, None)),
        },
        Err(error) => Err((taken(), Some(error))),
    }
}

/// The chain a resumed session's first handshake kept, leaf first; `None`
/// for a connection that resumed nothing.
fn resumed_chain(ssl: &ssl::SslRef) -> Option<Vec<Vec<u8>>> {
    if !ssl.session_reused() {
        return None;
    }
    let chain = ssl.peer_cert_chain()?;
    Some(
        chain
            .iter()
            .filter_map(|certificate| certificate.to_der().ok())
            .collect(),
    )
}

/// Offers to resume `session` on `ssl`, which `btls` allows only for a
/// session of the same context: BoringSSL itself ties a client session to
/// no context, and does not offer one that `ssl`'s settings cannot resume.
#[expect(
    unsafe_code,
    reason = "btls's `set_session` asks for the session's own context, while each upstream connection has a context of its own built from the browser's ClientHello, so the relay calls BoringSSL's `SSL_set_session` through btls-sys (ADR 0124)"
)]
fn resume(ssl: &mut ssl::SslRef, session: &SslSession) -> Result<(), btls::error::ErrorStack> {
    // SAFETY: `ssl.as_ptr()` is the live `SSL` that `ssl` borrows, whose
    // handshake has not begun, as `SSL_set_session` requires; BoringSSL
    // takes its own reference to the live `SSL_SESSION` that `session`
    // owns. The session was made by this same BoringSSL, whose contexts
    // share one X.509 method, and BoringSSL checks the session's version
    // and cipher suite against `ssl`'s settings before it offers it.
    let set = unsafe { btls_sys::SSL_set_session(ssl.as_ptr(), session.as_ptr()) };
    if set == 1 {
        Ok(())
    } else {
        Err(btls::error::ErrorStack::get())
    }
}

/// The connector of one upstream handshake: the values of `hello`, a
/// verify callback that keeps the chain in `seen` and lets it through, and,
/// with `keeping`, the sessions the site gives kept as the host's.
fn connector(
    hello: Option<&ClientHello>,
    seen: Arc<Mutex<Option<Vec<Vec<u8>>>>>,
    keeping: Option<(&Arc<Sessions>, &str)>,
) -> Result<SslConnector, btls::error::ErrorStack> {
    let mut builder = SslConnector::bare_builder(SslMethod::tls())?;
    builder.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
        let chain = ssl
            .peer_cert_chain()
            .map(|chain| {
                chain
                    .iter()
                    .filter_map(|certificate| certificate.to_der().ok())
                    .collect()
            })
            .unwrap_or_default();
        *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(chain);
        Ok(())
    });
    if let Some((sessions, host)) = keeping {
        let sessions = Arc::clone(sessions);
        let host = host.to_owned();
        builder.set_session_cache_mode(SslSessionCacheMode::CLIENT);
        builder.set_new_session_callback(move |_, session| sessions.keep(&host, session));
    }
    let Some(hello) = hello else {
        builder.set_alpn_protos(b"\x08http/1.1")?;
        return Ok(builder.build());
    };
    let versions = || {
        hello
            .versions
            .iter()
            .filter_map(|version| version_of(*version))
    };
    builder.set_min_proto_version(versions().min_by_key(|version| order(*version)))?;
    builder.set_max_proto_version(versions().max_by_key(|version| order(*version)))?;
    builder.set_grease_enabled(hello.grease);
    builder.set_preserve_tls13_cipher_list(true);
    let ciphers = names(&hello.cipher_suites, cipher_name);
    if !ciphers.is_empty() {
        builder.set_cipher_list(&ciphers)?;
    }
    let groups = names(&hello.groups, group_name);
    if !groups.is_empty() {
        builder.set_curves_list(&groups)?;
    }
    let algorithms = names(&hello.signature_algorithms, signature_name);
    if !algorithms.is_empty() {
        builder.set_sigalgs_list(&algorithms)?;
    }
    let delegated = names(&hello.delegated_credentials, signature_name);
    if !delegated.is_empty() {
        builder.set_delegated_credentials(&delegated)?;
    }
    if !hello.alpn.is_empty() {
        let mut wire = Vec::new();
        for protocol in &hello.alpn {
            wire.push(u8::try_from(protocol.len()).unwrap_or(u8::MAX));
            wire.extend_from_slice(protocol);
        }
        builder.set_alpn_protos(&wire)?;
    }
    if hello.status_request() {
        builder.enable_ocsp_stapling();
    }
    if hello.signed_certificate_timestamps() {
        builder.enable_signed_cert_timestamps();
    }
    if !hello.session_ticket() {
        builder.set_options(SslOptions::NO_TICKET);
    }
    if let Some(limit) = hello.record_size_limit {
        builder.set_record_size_limit(limit);
    }
    if let Some(ids) = &hello.trust_anchors {
        request_trust_anchors(&mut builder, ids);
    }
    for algorithm in &hello.certificate_compression {
        match *algorithm {
            1 => builder.add_certificate_compression_algorithm(Zlib)?,
            2 => builder.add_certificate_compression_algorithm(Brotli)?,
            _ => {}
        }
    }
    let order: Vec<ExtensionType> = hello
        .extensions
        .iter()
        .copied()
        .filter(|kind| BORINGSSL_EXTENSIONS.contains(kind))
        .map(ExtensionType::from)
        .collect();
    if builder.set_extension_permutation(&order).is_err() {
        tracing::debug!("BoringSSL refused the browser's extension order; its own is sent");
    }
    Ok(builder.build())
}

/// Whether the relay's upstream ClientHello carries every value of
/// `hello`, the browser's, so the site sees the browser's own fingerprint:
/// each cipher suite, group, key share, signature algorithm, certificate
/// compression algorithm, version and extension is one this BoringSSL
/// sends. A browser that offers more, such as ML-DSA signature algorithms,
/// is mirrored only in part.
pub(crate) fn mirrors(hello: &ClientHello) -> bool {
    hello
        .cipher_suites
        .iter()
        .all(|suite| cipher_name(*suite).is_some())
        && hello
            .groups
            .iter()
            .all(|group| group_name(*group).is_some())
        && hello
            .key_shares
            .iter()
            .all(|group| key_share(*group).is_some())
        && hello
            .signature_algorithms
            .iter()
            .chain(&hello.delegated_credentials)
            .all(|scheme| signature_name(*scheme).is_some())
        && hello
            .certificate_compression
            .iter()
            .all(|algorithm| matches!(algorithm, 1 | 2))
        && hello
            .versions
            .iter()
            .all(|version| version_of(*version).is_some())
        && hello
            .extensions
            .iter()
            .all(|kind| BORINGSSL_EXTENSIONS.contains(kind))
}

/// Asks for the trust anchors `ids` names, each ID behind its one-byte
/// length as a ClientHello's `trust_anchors` extension lists them, which
/// `btls` has no method for; a list BoringSSL refuses is left out.
#[expect(
    unsafe_code,
    reason = "btls has no method for `SSL_CTX_set1_requested_trust_anchors`, so the relay calls BoringSSL's own through btls-sys (ADR 0124)"
)]
pub(crate) fn request_trust_anchors(builder: &mut SslContextBuilder, ids: &[u8]) {
    // SAFETY: `builder.as_ptr()` is the live `SSL_CTX` that `builder` owns
    // for the whole call, and BoringSSL copies the `ids.len()` bytes `ids`
    // points to before it returns, keeping no pointer to them.
    let set = unsafe {
        btls_sys::SSL_CTX_set1_requested_trust_anchors(builder.as_ptr(), ids.as_ptr(), ids.len())
    };
    if set != 1 {
        tracing::debug!("BoringSSL refused the browser's trust anchor IDs; none are asked for");
    }
}

/// The extensions BoringSSL can send, which an extension order may name.
const BORINGSSL_EXTENSIONS: [u16; 26] = [
    0, 5, 10, 11, 13, 16, 18, 21, 23, 27, 28, 34, 35, 41, 43, 45, 50, 51, 57, 0x3374, 0x4469,
    17_513, 17_613, 0xca34, 0xfe0d, 0xff01,
];

/// Each of `values` that `name` knows, joined as BoringSSL's lists are.
fn names(values: &[u16], name: fn(u16) -> Option<&'static str>) -> String {
    values
        .iter()
        .filter_map(|value| name(*value))
        .collect::<Vec<_>>()
        .join(":")
}

fn version_of(version: u16) -> Option<SslVersion> {
    match version {
        0x0304 => Some(SslVersion::TLS1_3),
        0x0303 => Some(SslVersion::TLS1_2),
        0x0302 => Some(SslVersion::TLS1_1),
        0x0301 => Some(SslVersion::TLS1),
        _ => None,
    }
}

fn order(version: SslVersion) -> u8 {
    match version {
        SslVersion::TLS1_3 => 4,
        SslVersion::TLS1_2 => 3,
        SslVersion::TLS1_1 => 2,
        _ => 1,
    }
}

/// BoringSSL's name of a cipher suite (IANA's TLS Cipher Suites).
fn cipher_name(suite: u16) -> Option<&'static str> {
    Some(match suite {
        0x1301 => "TLS_AES_128_GCM_SHA256",
        0x1302 => "TLS_AES_256_GCM_SHA384",
        0x1303 => "TLS_CHACHA20_POLY1305_SHA256",
        0xc02b => "ECDHE-ECDSA-AES128-GCM-SHA256",
        0xc02f => "ECDHE-RSA-AES128-GCM-SHA256",
        0xc02c => "ECDHE-ECDSA-AES256-GCM-SHA384",
        0xc030 => "ECDHE-RSA-AES256-GCM-SHA384",
        0xcca9 => "ECDHE-ECDSA-CHACHA20-POLY1305",
        0xcca8 => "ECDHE-RSA-CHACHA20-POLY1305",
        0xc009 => "ECDHE-ECDSA-AES128-SHA",
        0xc00a => "ECDHE-ECDSA-AES256-SHA",
        0xc013 => "ECDHE-RSA-AES128-SHA",
        0xc014 => "ECDHE-RSA-AES256-SHA",
        0xc023 => "ECDHE-ECDSA-AES128-SHA256",
        0xc024 => "ECDHE-ECDSA-AES256-SHA384",
        0xc027 => "ECDHE-RSA-AES128-SHA256",
        0xc028 => "ECDHE-RSA-AES256-SHA384",
        0x009e => "DHE-RSA-AES128-GCM-SHA256",
        0x009f => "DHE-RSA-AES256-GCM-SHA384",
        0x0033 => "DHE-RSA-AES128-SHA",
        0x0039 => "DHE-RSA-AES256-SHA",
        0x0067 => "DHE-RSA-AES128-SHA256",
        0x006b => "DHE-RSA-AES256-SHA256",
        0x003c => "AES128-SHA256",
        0x003d => "AES256-SHA256",
        0x009c => "AES128-GCM-SHA256",
        0x009d => "AES256-GCM-SHA384",
        0x002f => "AES128-SHA",
        0x0035 => "AES256-SHA",
        0x000a => "DES-CBC3-SHA",
        _ => return None,
    })
}

/// BoringSSL's name of a group (IANA's TLS Supported Groups).
fn group_name(group: u16) -> Option<&'static str> {
    Some(match group {
        0x11ec => "X25519MLKEM768",
        0x6399 => "X25519Kyber768Draft00",
        0x001d => "X25519",
        0x0017 => "P-256",
        0x0018 => "P-384",
        0x0019 => "P-521",
        _ => return None,
    })
}

/// BoringSSL's key share of a group.
fn key_share(group: u16) -> Option<KeyShare> {
    Some(match group {
        0x11ec => KeyShare::X25519_MLKEM768,
        0x6399 => KeyShare::X25519_KYBER768_DRAFT00,
        0x001d => KeyShare::X25519,
        0x0017 => KeyShare::P256,
        0x0018 => KeyShare::P384,
        0x0019 => KeyShare::P521,
        _ => return None,
    })
}

/// BoringSSL's name of a signature algorithm (IANA's TLS `SignatureScheme`).
fn signature_name(scheme: u16) -> Option<&'static str> {
    Some(match scheme {
        0x0403 => "ecdsa_secp256r1_sha256",
        0x0503 => "ecdsa_secp384r1_sha384",
        0x0603 => "ecdsa_secp521r1_sha512",
        0x0804 => "rsa_pss_rsae_sha256",
        0x0805 => "rsa_pss_rsae_sha384",
        0x0806 => "rsa_pss_rsae_sha512",
        0x0401 => "rsa_pkcs1_sha256",
        0x0501 => "rsa_pkcs1_sha384",
        0x0601 => "rsa_pkcs1_sha512",
        0x0201 => "rsa_pkcs1_sha1",
        0x0203 => "ecdsa_sha1",
        0x0807 => "ed25519",
        _ => return None,
    })
}

/// Brotli-compressed certificates, decoded (RFC 8879).
struct Brotli;

impl CertificateCompressor for Brotli {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::BROTLI;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> io::Result<()>
    where
        W: io::Write,
    {
        brotli_decompressor::BrotliDecompress(&mut &*input, output)
    }
}

/// Zlib-compressed certificates, decoded (RFC 8879).
struct Zlib;

impl CertificateCompressor for Zlib {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> io::Result<()>
    where
        W: io::Write,
    {
        io::copy(&mut flate2::read::ZlibDecoder::new(input), output).map(|_| ())
    }
}

/// Finishes the browser's handshake with `host`'s leaf, choosing `chosen`,
/// the site's protocol, when the browser offered it.
async fn accept<B>(
    browser: B,
    host: &str,
    authority: &Authority,
    chosen: Option<Vec<u8>>,
) -> Result<SslStream<B>, ()>
where
    B: AsyncRead + AsyncWrite + Unpin,
{
    let leaf = authority.leaf(host).map_err(|error| {
        tracing::warn!(%error, "the relay's authority signed no leaf");
    })?;
    let acceptor = (|| {
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())?;
        let certificate = X509::from_der(&leaf.chain[0])?;
        builder.set_certificate(&certificate)?;
        builder.add_extra_chain_cert(X509::from_der(&leaf.chain[1])?)?;
        let key = PKey::private_key_from_pkcs8(&leaf.key)?;
        builder.set_private_key(&key)?;
        builder.set_alpn_select_callback(move |_, offered| {
            chosen
                .as_deref()
                .and_then(|chosen| alpn_in(offered, chosen))
                .ok_or(AlpnError::NOACK)
        });
        Ok::<_, btls::error::ErrorStack>(builder.build())
    })()
    .map_err(|_| ())?;
    let ssl = ssl::Ssl::new(acceptor.context()).map_err(|_| ())?;
    let mut stream = SslStream::new(ssl, browser).map_err(|_| ())?;
    Pin::new(&mut stream).accept().await.map_err(|_| ())?;
    Ok(stream)
}

/// `chosen` as it stands in `offered`, an ALPN list, so the answer borrows
/// the browser's own bytes.
fn alpn_in<'a>(offered: &'a [u8], chosen: &[u8]) -> Option<&'a [u8]> {
    let mut rest = offered;
    while let Some((&length, tail)) = rest.split_first() {
        let length = usize::from(length);
        let name = tail.get(..length)?;
        if name == chosen {
            return Some(name);
        }
        rest = &tail[length..];
    }
    None
}

/// The browser's connection, with the bytes the relay read off it to be
/// read again first.
struct Replayed<B> {
    first: Vec<u8>,
    at: usize,
    inner: B,
}

impl<B> Replayed<B> {
    fn new(inner: B) -> Self {
        Self {
            first: Vec::new(),
            at: 0,
            inner,
        }
    }

    /// Reads `bytes` again before what is left to read.
    fn replay(&mut self, mut bytes: Vec<u8>) {
        bytes.extend_from_slice(&self.first[self.at..]);
        self.first = bytes;
        self.at = 0;
    }
}

impl<B: AsyncRead + Unpin> AsyncRead for Replayed<B> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.at < this.first.len() {
            let left = &this.first[this.at..];
            let count = left.len().min(buffer.remaining());
            buffer.put_slice(&left[..count]);
            this.at += count;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(context, buffer)
    }
}

impl<B: AsyncWrite + Unpin> AsyncWrite for Replayed<B> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}
