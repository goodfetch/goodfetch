//! The relay's terminating mode in process, on every OS (spec #564's
//! Testing Decisions): the browser, the relay and the site are in-memory
//! streams, the browser and the site BoringSSL ends of their own, the site's
//! certificates made by a test authority. The oracles are what the site
//! received, what the browser was shown and the Run's pins.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use btls::pkey::PKey;
use btls::ssl::{SslAcceptor, SslConnector, SslMethod, SslVerifyMode};
use btls::x509::X509;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::{Policy, Refused};
use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::{ExceptionKind, KeyHash, KnownCertificates};
use goodfetch_engine::secrets::ExceptionHosts;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{CertificateExceptions, ExceptionUsed, PinBook, Presented, Trust};
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use goodfetch_test_support::{TestEnv, runtime};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use rstest::rstest;
use serde_json::json;
use tokio::io::{
    AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream, ReadBuf, duplex,
};

use super::application_settings::{self, ApplicationSettings};
use super::authority::Authority;
use super::client_hello::{self, ClientHello};
use super::serve::{Met, Network, Observe, Relaying, Resolved, Served, Unreached, Upstream, serve};
use super::terminate::{
    Mirror, Stop, Terminated, Terminating, failure_of, mirrors, request_trust_anchors, terminate,
    upstream,
};

/// The person whose pins the tests keep.
const PERSON: &str = "os_user";

/// A site certificate's defect.
#[derive(Clone, Copy, Debug)]
pub(super) enum Made {
    Valid,
    Expired,
    NotYetValid,
    UnknownIssuer,
    NameMismatch,
    SelfSigned,
}

/// The test authority every site certificate but the unknown issuer's and
/// the self-signed one chains to.
pub(super) struct SiteAuthority {
    params: CertificateParams,
    key: KeyPair,
    pub(super) der: Vec<u8>,
}

pub(super) fn authority(name: &str) -> SiteAuthority {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    let mut distinguished = DistinguishedName::new();
    distinguished.push(DnType::CommonName, name);
    params.distinguished_name = distinguished;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let der = params.self_signed(&key).unwrap().der().to_vec();
    SiteAuthority { params, key, der }
}

/// A leaf for `host` with `made`'s defect, and its key in PKCS #8.
pub(super) fn site_certificate(
    host: &str,
    made: Made,
    trusted: &SiteAuthority,
) -> (Vec<u8>, Vec<u8>) {
    let key = KeyPair::generate().unwrap();
    let name = match made {
        Made::NameMismatch => "elsewhere.test".to_owned(),
        _ => host.to_owned(),
    };
    let mut params = CertificateParams::new(vec![name.clone()]).unwrap();
    let mut distinguished = DistinguishedName::new();
    distinguished.push(DnType::CommonName, name);
    params.distinguished_name = distinguished;
    match made {
        Made::Expired => {
            params.not_before = date_time_ymd(2020, 1, 1);
            params.not_after = date_time_ymd(2021, 1, 1);
        }
        Made::NotYetValid => {
            params.not_before = date_time_ymd(4000, 1, 1);
            params.not_after = date_time_ymd(4001, 1, 1);
        }
        _ => {}
    }
    let der = match made {
        Made::SelfSigned => params.self_signed(&key).unwrap().der().to_vec(),
        Made::UnknownIssuer => {
            let other = authority("Unknown");
            let issuer = Issuer::from_params(&other.params, &other.key);
            params.signed_by(&key, &issuer).unwrap().der().to_vec()
        }
        _ => {
            let issuer = Issuer::from_params(&trusted.params, &trusted.key);
            params.signed_by(&key, &issuer).unwrap().der().to_vec()
        }
    };
    (der, key.serialize_der())
}

/// What the site received once its handshake finished.
#[derive(Default)]
struct Received(Mutex<Vec<u8>>);

/// The site: a BoringSSL server presenting `certificate`, choosing `h2`
/// when it is offered, else `http/1.1`, which reads what comes and answers
/// `pong` to its first bytes.
async fn site(stream: DuplexStream, (certificate, key): (Vec<u8>, Vec<u8>), received: &Received) {
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    builder
        .set_certificate(&X509::from_der(&certificate).unwrap())
        .unwrap();
    builder
        .set_private_key(&PKey::private_key_from_pkcs8(&key).unwrap())
        .unwrap();
    builder.set_alpn_select_callback(|_, offered| {
        btls::ssl::select_next_proto(b"\x02h2\x08http/1.1", offered)
            .ok_or(btls::ssl::AlpnError::NOACK)
    });
    let acceptor = builder.build();
    let ssl = btls::ssl::Ssl::new(acceptor.context()).unwrap();
    let mut stream = tokio_btls::SslStream::new(ssl, stream).unwrap();
    if std::pin::Pin::new(&mut stream).accept().await.is_err() {
        return;
    }
    // It answers once, then reads to the end, so it keeps every byte the
    // relay sent even after the relay ended the connection.
    let mut answered = false;
    let mut buffer = [0; 64];
    while let Ok(count) = stream.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        received
            .0
            .lock()
            .unwrap()
            .extend_from_slice(&buffer[..count]);
        if !std::mem::replace(&mut answered, true) {
            let _ = stream.write_all(b"pong").await;
        }
    }
}

/// What the browser saw: the chain it was shown, the protocol chosen and
/// the site's answer.
#[derive(Debug, Default)]
struct Shown {
    chain: Vec<Vec<u8>>,
    protocol: Option<Vec<u8>>,
    answer: Vec<u8>,
}

/// The request a browser sends a host an exception lists, which carries no
/// cookie.
const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: listed.test\r\n\r\n";

/// The browser: a BoringSSL client offering `h2` and `http/1.1` with
/// GREASE, which takes any certificate, sends `ping` and reads the answer.
async fn browser(stream: DuplexStream, host: &str) -> Shown {
    browser_sending(stream, host, b"ping").await
}

/// The browser, sending `sent` once its handshake finished.
async fn browser_sending(stream: DuplexStream, host: &str, sent: &[u8]) -> Shown {
    let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let kept = Arc::clone(&seen);
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_grease_enabled(true);
    builder.set_alpn_protos(b"\x02h2\x08http/1.1").unwrap();
    builder.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
        *kept.lock().unwrap() = ssl
            .peer_cert_chain()
            .map(|chain| {
                chain
                    .iter()
                    .map(|certificate| certificate.to_der().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        Ok(())
    });
    let ssl = builder.build().configure().unwrap().into_ssl(host).unwrap();
    let mut stream = tokio_btls::SslStream::new(ssl, stream).unwrap();
    let mut shown = Shown::default();
    if std::pin::Pin::new(&mut stream).connect().await.is_err() {
        return shown;
    }
    shown.chain = seen.lock().unwrap().clone();
    shown.protocol = stream.ssl().selected_alpn_protocol().map(<[u8]>::to_vec);
    stream.write_all(sent).await.unwrap();
    let mut answer = [0; 4];
    if stream.read_exact(&mut answer).await.is_ok() {
        shown.answer = answer.to_vec();
    }
    drop(stream);
    shown
}

/// A Run's stores: its pins and its Audit log.
pub(super) struct Stores {
    store: KnownCertificates,
    pub(super) book: PinBook,
    home: Home,
    _env: TestEnv,
}

pub(super) fn stores() -> Stores {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let store = KnownCertificates::of(
        &home,
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    );
    let book = PinBook {
        store: store.clone(),
        person: PERSON.to_owned(),
        run_id: None,
        audit: Arc::new(AuditLog::of(
            &home,
            None,
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        )),
        clock: Clock::system(),
    };
    Stores {
        store,
        book,
        home,
        _env: env,
    }
}

/// The relay terminating `listed.test` and its subdomains for the
/// authority `trusted` the Run trusts for them, with no exception: their
/// connections keep the browser's own protocol.
fn trusting(stores: &Stores, trusted: &SiteAuthority) -> Terminating {
    Terminating {
        exceptions: CertificateExceptions::listed(
            &json!({ "elsewhere.test": null }),
            stores.book.clone(),
            ExceptionHosts::default(),
        )
        .unwrap(),
        trust: trust(trusted),
        ..terminating(stores, trusted)
    }
}

/// The relay terminating `listed.test` and its subdomains, trusting
/// `trusted` for them beside Mozilla's roots.
fn terminating(stores: &Stores, trusted: &SiteAuthority) -> Terminating {
    Terminating {
        authority: Arc::new(Authority::new(vec!["listed.test".to_owned()]).unwrap()),
        exceptions: CertificateExceptions::listed(
            &json!({ "listed.test": null }),
            stores.book.clone(),
            ExceptionHosts::default(),
        )
        .unwrap(),
        examiner: trust(trusted).examiner(Clock::system()),
        trust: Trust::default_roots(),
        application_settings: Arc::default(),
        sessions: Arc::default(),
    }
}

/// Mozilla's roots and `trusted`, for `listed.test`.
fn trust(trusted: &SiteAuthority) -> Trust {
    Trust::default_roots()
        .with_trusted_cas("listed.test", vec![trusted.der.clone().into()])
        .unwrap()
}

/// One browser connection to `host` through the terminating mode, its
/// site presenting `made`'s certificate, the browser sending [`REQUEST`]:
/// how the relay ended it, what the browser was shown and what the site
/// received.
fn through(
    host: &str,
    made: Made,
    stores: &Stores,
    trusted: &SiteAuthority,
) -> (Through, Shown, Vec<u8>, Arc<Authority>) {
    through_sending(host, made, (stores, trusted), REQUEST)
}

/// [`through`], the browser sending `sent`.
fn through_sending(
    host: &str,
    made: Made,
    (stores, trusted): (&Stores, &SiteAuthority),
    sent: &[u8],
) -> (Through, Shown, Vec<u8>, Arc<Authority>) {
    let terminating = terminating(stores, trusted);
    let certificate = site_certificate(host, made, trusted);
    let presented = KeyHash::of_public_key(
        &X509::from_der(&certificate.0)
            .unwrap()
            .public_key()
            .unwrap()
            .public_key_to_der()
            .unwrap(),
    );
    let received = Received::default();
    let used = Mutex::new(Passed::Not);
    let told = Mutex::new(None);
    let (terminated, shown) = runtime().block_on(async {
        let (browser_end, relay_browser) = duplex(1 << 16);
        let (relay_site, site_end) = duplex(1 << 16);
        let passed = |exception: Option<&ExceptionUsed>| {
            *used.lock().unwrap() = exception.cloned().map_or(Passed::Verified, Passed::Through);
        };
        let (terminated, shown, ()) = tokio::join!(
            terminate(
                relay_browser,
                relay_site,
                (Vec::new(), host),
                &terminating,
                passed,
                |stop: &Stop| *told.lock().unwrap() = Some(stop.clone())
            ),
            browser_sending(browser_end, host, sent),
            site(site_end, certificate, &received),
        );
        (terminated, shown)
    });
    let received = received.0.lock().unwrap().clone();
    let passed = used.into_inner().unwrap();
    let stopped = told.into_inner().unwrap();
    (
        Through {
            terminated,
            passed,
            stopped,
            presented,
        },
        shown,
        received,
        Arc::clone(&terminating.authority),
    )
}

/// The authority's own certificate, in DER: the second of every chain it
/// signs.
fn authority_certificate(authority: &Authority) -> Vec<u8> {
    let [_, certificate] = authority.leaf("listed.test").unwrap().chain.clone();
    certificate
}

/// How the relay ended a connection, and the exception it said the
/// connection passed through, once it said it passed.
#[derive(Debug)]
struct Through {
    terminated: Terminated,
    passed: Passed,
    /// Why the relay said it stopped it, if it did.
    stopped: Option<Stop>,
    /// The key of the certificate the site presented.
    presented: KeyHash,
}

/// What the relay said of a connection's certificate.
#[derive(Debug)]
enum Passed {
    /// Nothing: it did not pass.
    Not,
    /// It passed with no exception.
    Verified,
    /// It passed through this exception.
    Through(ExceptionUsed),
}

/// Covers: story:4-40, setting:tls.accept_invalid_certs
#[test]
fn a_listed_hosts_invalid_certificate_passes_under_its_exception_and_the_bytes_go_unchanged() {
    let stores = stores();
    let trusted = authority("Site authority");

    let (through, shown, received, authority) =
        through("expired.listed.test", Made::Expired, &stores, &trusted);

    let Through {
        terminated: Terminated::Relayed { .. },
        passed: Passed::Through(used),
        stopped: None,
        ..
    } = through
    else {
        panic!("the connection was not relayed through the exception: {through:?}");
    };
    assert_eq!(
        (
            used.kind,
            used.first_pin,
            shown.chain.get(1).map(Vec::as_slice),
            shown.protocol.as_deref(),
            shown.answer.as_slice(),
            received.as_slice(),
        ),
        (
            ExceptionKind::TrustOnFirstUse,
            true,
            Some(authority_certificate(&authority).as_slice()),
            Some(b"http/1.1".as_slice()),
            b"pong".as_slice(),
            REQUEST,
        )
    );
}

/// No Session cookie reaches a host an exception lists, on a request the
/// interception never pauses either, such as a WebSocket handshake (deep
/// review finding 564-1): the relay holds the host to HTTP/1.1 and reads
/// each request head before it goes up, so the connection ends at the
/// first that carries a `Cookie` header, before any byte of it reaches the
/// host, while the requests before it, a body naming a cookie in its bytes
/// and a WebSocket's frames after its upgrade go up unchanged.
///
/// Covers: story:4-38
#[rstest]
#[case::a_request_with_no_cookie(REQUEST, REQUEST, true)]
#[case::a_request_carrying_a_cookie(
    b"GET /ws HTTP/1.1\r\nHost: listed.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nCookie: session=c00k1e\r\n\r\n",
    b"",
    false
)]
#[case::a_cookie_on_a_later_request(
    b"GET / HTTP/1.1\r\nHost: listed.test\r\n\r\nGET /two HTTP/1.1\r\nHost: listed.test\r\ncookie: session=c00k1e\r\n\r\n",
    REQUEST,
    false
)]
#[case::a_body_that_names_a_cookie(
    b"POST / HTTP/1.1\r\nHost: listed.test\r\nContent-Length: 19\r\n\r\nCookie: not-a-head\nGET / HTTP/1.1\r\nHost: listed.test\r\n\r\n",
    b"POST / HTTP/1.1\r\nHost: listed.test\r\nContent-Length: 19\r\n\r\nCookie: not-a-head\nGET / HTTP/1.1\r\nHost: listed.test\r\n\r\n",
    true
)]
#[case::a_chunked_body_then_a_cookie(
    b"POST / HTTP/1.1\r\nHost: listed.test\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\nGET / HTTP/1.1\r\nHost: listed.test\r\nCookie: a=b\r\n\r\n",
    b"POST / HTTP/1.1\r\nHost: listed.test\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
    false
)]
#[case::a_web_socket_after_its_upgrade(
    b"GET /ws HTTP/1.1\r\nHost: listed.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\x81\x05Cookie",
    b"GET /ws HTTP/1.1\r\nHost: listed.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\x81\x05Cookie",
    true
)]
fn no_request_carrying_a_cookie_reaches_a_host_an_exception_lists(
    #[case] sent: &[u8],
    #[case] expected: &[u8],
    #[case] relayed: bool,
) {
    let stores = stores();
    let trusted = authority("Site authority");

    let (through, shown, received, _) = through_sending(
        "expired.listed.test",
        Made::Expired,
        (&stores, &trusted),
        sent,
    );

    assert_eq!(
        (
            shown.protocol.as_deref(),
            String::from_utf8_lossy(&received).into_owned(),
            matches!(through.terminated, Terminated::Relayed { .. }),
        ),
        (
            Some(b"http/1.1".as_slice()),
            String::from_utf8_lossy(expected).into_owned(),
            relayed
        ),
        "{through:?}"
    );
}

/// Covers: story:4-40
#[test]
fn a_listed_host_whose_certificate_verifies_passes_with_no_exception() {
    let stores = stores();
    let trusted = authority("Site authority");

    let (through, shown, received, _) =
        through("valid.listed.test", Made::Valid, &stores, &trusted);

    assert!(
        matches!(
            through,
            Through {
                terminated: Terminated::Relayed { .. },
                passed: Passed::Verified,
                stopped: None,
                ..
            }
        ),
        "{through:?}"
    );
    assert_eq!(
        (shown.answer, received),
        (b"pong".to_vec(), REQUEST.to_vec())
    );
}

/// A04-18: a pinned host presenting another key is stopped, and nothing of
/// the browser's reaches it.
/// Covers: error:certificate_changed
#[test]
fn a_pinned_host_that_presents_another_key_is_stopped_and_receives_nothing() {
    let stores = stores();
    let trusted = authority("Site authority");
    let other = KeyHash::of_public_key(b"another key");
    goodfetch_test_support::runtime()
        .block_on(stores.store.pin(
            PERSON,
            "expired.listed.test",
            &other,
            None,
            jiff::Timestamp::UNIX_EPOCH,
        ))
        .unwrap();

    let (through, shown, received, _) =
        through("expired.listed.test", Made::Expired, &stores, &trusted);

    let (Terminated::Stopped, Some(Stop::Changed(changed))) =
        (&through.terminated, through.stopped)
    else {
        panic!("the connection was not stopped as a changed key");
    };
    assert_eq!(
        (
            changed.pinned,
            matches!(through.passed, Passed::Not),
            shown.answer.len(),
            received.len()
        ),
        (Some(other), true, 0, 0)
    );
}

/// Story 4-43: a pin row damaged on disk, its key's bytes cut short, stops
/// the host as a changed key whose pin cannot be read, never let through
/// as unpinned, and nothing of the browser's reaches it.
/// Covers: story:4-43, error:certificate_changed, format:known-certificates
#[test]
fn a_damaged_pin_row_stops_the_host_as_a_changed_key() {
    let stores = stores();
    let trusted = authority("Site authority");
    goodfetch_test_support::runtime()
        .block_on(stores.store.pin(
            PERSON,
            "expired.listed.test",
            &KeyHash::of_public_key(b"the pinned key"),
            None,
            jiff::Timestamp::UNIX_EPOCH,
        ))
        .unwrap();
    rusqlite::Connection::open(stores.home.known_certificates())
        .unwrap()
        .execute("UPDATE pins SET spki_sha256 = x'0102'", [])
        .unwrap();

    let (through, shown, received, _) =
        through("expired.listed.test", Made::Expired, &stores, &trusted);

    let (Terminated::Stopped, Some(Stop::Changed(changed))) =
        (&through.terminated, through.stopped)
    else {
        panic!("the connection was not stopped as a changed key");
    };
    assert_eq!(
        (
            changed.host.as_str(),
            changed.pinned,
            changed.presented,
            matches!(through.passed, Passed::Not),
            shown.answer.len(),
            received.len()
        ),
        ("expired.listed.test", None, through.presented, true, 0, 0)
    );
    // The wire form names no pin it could not read.
    assert_eq!(
        serde_json::to_value(changed.details()).unwrap(),
        json!({
            "host": "expired.listed.test",
            "presented_spki_sha256": through.presented.to_string(),
            "fix": "A person accepts the new key of expired.listed.test with `certs accept expired.listed.test` in a terminal, or removes its pin with `certs forget expired.listed.test`; no agent, Recipe or `--json` command can.",
        })
    );
}

/// ADR 0127's table for the relay: how each certificate fails the per-host
/// decision, read from the chain the site's handshake presented, and a
/// handshake BoringSSL ended for another reason, as its reason in text.
/// Covers: error:certificate_invalid
#[rstest]
#[case::expired(Made::Expired, KnownCertificateFailure::Expired)]
#[case::not_yet_valid(Made::NotYetValid, KnownCertificateFailure::NotYetValid)]
#[case::unknown_issuer(Made::UnknownIssuer, KnownCertificateFailure::UnknownIssuer)]
#[case::name_mismatch(Made::NameMismatch, KnownCertificateFailure::NameMismatch)]
#[case::self_signed(Made::SelfSigned, KnownCertificateFailure::SelfSigned)]
fn each_known_failure_of_a_sites_certificate_is_named_as_the_wire_names_it(
    #[case] made: Made,
    #[case] expected: KnownCertificateFailure,
) {
    let trusted = authority("Site authority");
    let host = "site.listed.test";
    let examiner = Trust::default_roots()
        .with_trusted_cas("listed.test", vec![trusted.der.clone().into()])
        .unwrap()
        .examiner(Clock::system());
    let certificate = site_certificate(host, made, &trusted);
    let received = Received::default();

    let chain = runtime().block_on(async {
        let (relay_site, site_end) = duplex(1 << 16);
        let (opened, ()) = tokio::join!(
            async {
                let opened = upstream(relay_site, host, None).await;
                opened
                    .map(|upstream| upstream.chain)
                    .map_err(|(chain, _)| chain)
            },
            site(site_end, certificate, &received),
        );
        opened
    });

    let chain = chain.unwrap_or_else(|chain| chain.unwrap());
    let Presented { failure, .. } = examiner.examine(host, &chain).unwrap();
    assert_eq!(
        failure_of(failure, None),
        CertificateFailure::Known(expected)
    );
}

/// Covers: error:certificate_invalid
#[test]
fn a_handshake_that_ends_for_another_reason_is_named_by_boringssls_reason() {
    let opened = runtime().block_on(async {
        let (relay_site, mut site_end) = duplex(1 << 16);
        let (opened, ()) = tokio::join!(upstream(relay_site, "site.listed.test", None), async {
            let mut hello = [0; 5];
            let _ = site_end.read_exact(&mut hello).await;
            // A TLS 1.2 alert record: fatal, protocol_version.
            let _ = site_end.write_all(&[21, 3, 3, 0, 2, 2, 70]).await;
        });
        opened
    });

    let Err((None, error)) = opened else {
        panic!("the handshake did not fail before the certificate");
    };
    let failure = failure_of(None, error.as_ref());
    assert!(
        matches!(&failure, CertificateFailure::Unrecognized(text) if text.contains("protocol_version")),
        "{failure:?}"
    );
}

/// The relay's upstream ClientHello carries the browser's own values, in
/// the browser's order, the trust anchor IDs Chromium asks for among them.
/// Covers: story:4-40
#[test]
fn the_upstream_client_hello_mirrors_the_browsers() {
    let (browser_hello, relay_hello) = runtime().block_on(async {
        let (browser_end, mut relay_reads) = duplex(1 << 16);
        let (relay_site, mut site_end) = duplex(1 << 16);
        let ((), browser_hello, relay_hello) = tokio::join!(
            async {
                let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
                builder.set_grease_enabled(true);
                builder.set_permute_extensions(true);
                builder.set_alpn_protos(b"\x02h2\x08http/1.1").unwrap();
                builder.set_curves_list("X25519:P-256:P-384").unwrap();
                builder
                    .set_sigalgs_list("ecdsa_secp256r1_sha256:rsa_pss_rsae_sha256:rsa_pkcs1_sha256")
                    .unwrap();
                builder.enable_ocsp_stapling();
                builder.enable_signed_cert_timestamps();
                request_trust_anchors(&mut builder, b"\x03\x2a\x03\x04\x02\x2a\x05");
                builder.set_verify(SslVerifyMode::NONE);
                let ssl = builder
                    .build()
                    .configure()
                    .unwrap()
                    .into_ssl("site.listed.test")
                    .unwrap();
                let mut stream = tokio_btls::SslStream::new(ssl, browser_end).unwrap();
                let _ = std::pin::Pin::new(&mut stream).connect().await;
            },
            async {
                let read = client_hello::read(&mut relay_reads).await.unwrap().0;
                drop(relay_reads);
                let mirror = Mirror {
                    hello: &read,
                    application_settings: &ApplicationSettings::new(),
                    sessions: None,
                };
                let _ = upstream(relay_site, "site.listed.test", Some(mirror)).await;
                read
            },
            async {
                let read = client_hello::read(&mut site_end).await.unwrap().0;
                drop(site_end);
                read
            },
        );
        (browser_hello, relay_hello)
    });

    assert_eq!(
        (
            &relay_hello.cipher_suites,
            &relay_hello.groups,
            &relay_hello.key_shares,
            &relay_hello.signature_algorithms,
            &relay_hello.alpn,
            &relay_hello.versions,
            &relay_hello.extensions,
            relay_hello.grease,
            &relay_hello.server_name,
            &relay_hello.trust_anchors,
        ),
        (
            &browser_hello.cipher_suites,
            &browser_hello.groups,
            &browser_hello.key_shares,
            &browser_hello.signature_algorithms,
            &browser_hello.alpn,
            &browser_hello.versions,
            &browser_hello.extensions,
            browser_hello.grease,
            &browser_hello.server_name,
            &browser_hello.trust_anchors,
        )
    );
    assert_eq!(
        browser_hello.trust_anchors.as_deref(),
        Some(&b"\x03\x2a\x03\x04\x02\x2a\x05"[..])
    );
}

/// The relay sends a browser's ClientHello again for its own look at a
/// certificate only when its BoringSSL carries every value of it: an
/// ML-DSA signature algorithm, as Chrome for Testing offers, or a group,
/// cipher suite, extension or certificate compression it does not send
/// would show the site another fingerprint.
/// Covers: story:4-40
#[test]
fn a_client_hello_is_mirrored_whole_only_when_boringssl_sends_each_of_its_values() {
    let browser = ClientHello {
        cipher_suites: vec![0x1301, 0x1302, 0x1303, 0xc02b, 0xc02f],
        extensions: vec![0, 5, 10, 13, 16, 18, 27, 43, 45, 51, 0xca34, 0xfe0d, 17_613],
        groups: vec![0x11ec, 0x001d, 0x0017],
        key_shares: vec![0x11ec, 0x001d],
        signature_algorithms: vec![0x0403, 0x0804, 0x0401],
        versions: vec![0x0304, 0x0303],
        certificate_compression: vec![2],
        ..ClientHello::default()
    };
    let with = |change: fn(&mut ClientHello)| {
        let mut hello = browser.clone();
        change(&mut hello);
        mirrors(&hello)
    };

    assert_eq!(
        [
            mirrors(&browser),
            with(|hello| hello.signature_algorithms.insert(0, 0x0904)),
            with(|hello| hello.groups.push(0x0101)),
            with(|hello| hello.cipher_suites.push(0x1304)),
            with(|hello| hello.extensions.push(49)),
            with(|hello| hello.certificate_compression.push(3)),
        ],
        [true, false, false, false, false, false]
    );
}

/// Story 4-40: the authority signs for any host, but a verifier that reads
/// its name constraints takes its leaf only for a listed domain.
/// Covers: story:4-40
#[test]
fn the_authoritys_leaf_for_a_host_outside_its_domains_is_rejected_by_a_verifier() {
    use rustls::client::WebPkiServerVerifier;
    use rustls::client::danger::ServerCertVerifier as _;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    let authority = Authority::new(vec!["listed.test".to_owned(), "192.0.2.7".to_owned()]).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(authority_certificate(&authority)))
        .unwrap();
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    let accepted = |host: &str| {
        let leaf = authority.leaf(host).unwrap();
        verifier
            .verify_server_cert(
                &CertificateDer::from(leaf.chain[0].clone()),
                &[],
                &ServerName::try_from(host.to_owned()).unwrap(),
                &[],
                UnixTime::since_unix_epoch(
                    Clock::system()
                        .wall()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap(),
                ),
            )
            .is_ok()
    };

    assert_eq!(
        [
            accepted("listed.test"),
            accepted("www.listed.test"),
            accepted("192.0.2.7"),
            accepted("example.com"),
            accepted("notlisted.test"),
            accepted("192.0.2.8"),
        ],
        [true, true, true, false, false, false]
    );
}

/// The relay's end of a browser connection, which says when the browser
/// can see it end: once it is shut down or dropped.
struct BrowserEnd {
    stream: DuplexStream,
    ended: Arc<AtomicBool>,
}

impl Drop for BrowserEnd {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::SeqCst);
    }
}

impl AsyncRead for BrowserEnd {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for BrowserEnd {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.ended.store(true, Ordering::SeqCst);
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

/// The network beyond the relay: every name at a public address, and its
/// one connection the site's end.
struct OneSite(Mutex<Option<DuplexStream>>);

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the test network answers at once, through the async interface the relay calls"
)]
impl Network for OneSite {
    type Stream = DuplexStream;

    async fn resolve(&self, _: &str, port: u16) -> Option<Resolved> {
        Some(Resolved {
            addresses: vec![SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(192, 31, 196, 1)),
                port,
            )],
        })
    }

    async fn connect(&self, _: SocketAddr) -> io::Result<DuplexStream> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionRefused))
    }

    async fn connect_upstream(&self) -> io::Result<DuplexStream> {
        Err(io::Error::from(io::ErrorKind::ConnectionRefused))
    }
}

/// What the relay told of its stopped connections: each one's URL, and
/// whether the browser could see that connection end by then.
struct Stopping {
    ended: Arc<AtomicBool>,
    stopped: Mutex<Vec<(String, bool)>>,
}

impl Observe for Stopping {
    fn unreached(&self, _: &str, _: Unreached) {}
    fn reached(&self, _: &str) {}
    fn refused(&self, _: &Refused, _: &str) {}
    fn upstream(&self, _: &str, _: Met) {}
    fn unauthenticated(&self, _: bool) {}
    fn terminated(&self, _: &str, _: Option<&ExceptionUsed>) {}
    fn client_hello(&self, _: &str, _: ClientHello) {}

    fn stopped(&self, url: &str, _: &str, _: &Stop) {
        self.stopped
            .lock()
            .unwrap()
            .push((url.to_owned(), self.ended.load(Ordering::SeqCst)));
    }
}

/// A stopped connection is told to the Session before the browser can see
/// it end: the browser fails its load only then, and the navigation that
/// waits on it finds its own refusal kept, never an earlier one of another
/// host's (fix-queue 83).
/// Covers: error:certificate_invalid
#[test]
fn a_stopped_connection_is_told_before_the_browser_sees_it_end() {
    let stores = stores();
    let trusted = authority("Site authority");
    let host = "expired.listed.test";
    let terminating = Terminating {
        authority: Arc::new(Authority::new(vec!["listed.test".to_owned()]).unwrap()),
        exceptions: CertificateExceptions::listed(
            &json!({ "elsewhere.test": null }),
            stores.book.clone(),
            ExceptionHosts::default(),
        )
        .unwrap(),
        examiner: trust(&trusted).examiner(Clock::system()),
        trust: trust(&trusted),
        application_settings: Arc::default(),
        sessions: Arc::default(),
    };
    let relaying = Relaying {
        policy: Policy::new(&["*"], &[] as &[&str]).unwrap(),
        authorization: "Basic c2Vzc2lvbg==".to_owned(),
        upstream: Upstream::Direct,
        bypasses: Box::new(|_| false),
        terminating: Some(Arc::new(terminating)),
        clock: Clock::system(),
        connect_deadline: std::time::Duration::from_secs(10),
    };
    let ended = Arc::new(AtomicBool::new(false));
    let told = Stopping {
        ended: Arc::clone(&ended),
        stopped: Mutex::default(),
    };
    let (relay_site, site_end) = duplex(1 << 16);
    let network = OneSite(Mutex::new(Some(relay_site)));
    let received = Received::default();

    let served = runtime().block_on(async {
        let (mut browser_end, relay_browser) = duplex(1 << 16);
        let relay_browser = BrowserEnd {
            stream: relay_browser,
            ended: Arc::clone(&ended),
        };
        let browsing = async {
            let authority = format!("{host}:443");
            browser_end
                .write_all(
                    format!(
                        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic c2Vzc2lvbg==\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(browser_end.read_u8().await.unwrap());
            }
            browser(browser_end, host).await
        };
        let (served, shown, ()) = tokio::join!(
            serve(relay_browser, &relaying, &network, &told),
            browsing,
            site(site_end, site_certificate(host, Made::Expired, &trusted), &received),
        );
        (served, shown.answer)
    });

    assert_eq!(
        (served, told.stopped.into_inner().unwrap()),
        (
            (Served::Refused, Vec::new()),
            vec![(format!("https://{host}:443/"), false)]
        )
    );
}

/// What a site that keeps its sessions saw of one connection: whether it
/// resumed an earlier session, and the ALPS payload the relay sent it.
#[derive(Debug, Default)]
struct Kept {
    resumed: bool,
    application_settings: Option<Vec<u8>>,
}

/// A site that keeps its sessions across connections, as `acceptor` does,
/// accepting ALPS for `h2`: it answers `pong` once and tells what it saw.
async fn keeping_site(stream: DuplexStream, acceptor: &SslAcceptor) -> Kept {
    let mut ssl = btls::ssl::Ssl::new(acceptor.context()).unwrap();
    ssl.set_alps_use_new_codepoint(true);
    application_settings::offer(&mut ssl, b"h2", b"").unwrap();
    let mut stream = tokio_btls::SslStream::new(ssl, stream).unwrap();
    if std::pin::Pin::new(&mut stream).accept().await.is_err() {
        return Kept::default();
    }
    let kept = Kept {
        resumed: stream.ssl().session_reused(),
        application_settings: application_settings::of_peer(stream.ssl()),
    };
    let mut buffer = [0; 4];
    if stream.read_exact(&mut buffer).await.is_ok() {
        let _ = stream.write_all(b"pong").await;
    }
    let _ = stream.shutdown().await;
    kept
}

/// The acceptor of a site whose certificate for `host` `trusted` signed,
/// choosing `h2`, which issues session tickets.
fn keeping_acceptor(host: &str, trusted: &SiteAuthority) -> SslAcceptor {
    let (certificate, key) = site_certificate(host, Made::Valid, trusted);
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    builder
        .set_certificate(&X509::from_der(&certificate).unwrap())
        .unwrap();
    builder
        .set_private_key(&PKey::private_key_from_pkcs8(&key).unwrap())
        .unwrap();
    builder.set_alpn_select_callback(|_, offered| {
        btls::ssl::select_next_proto(b"\x02h2", offered).ok_or(btls::ssl::AlpnError::NOACK)
    });
    builder.build()
}

/// A browser offering ALPS for `h2`, as Chromium does, with a payload of
/// its own the relay never sees before its upstream handshake.
async fn alps_browser(stream: DuplexStream, host: &str) -> Vec<u8> {
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_alpn_protos(b"\x02h2\x08http/1.1").unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    let mut configuration = builder.build().configure().unwrap();
    configuration.set_alps_use_new_codepoint(true);
    application_settings::offer(&mut configuration, b"h2", b"browser").unwrap();
    let ssl = configuration.into_ssl(host).unwrap();
    let mut stream = tokio_btls::SslStream::new(ssl, stream).unwrap();
    if std::pin::Pin::new(&mut stream).connect().await.is_err() {
        return Vec::new();
    }
    stream.write_all(b"ping").await.unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer).await;
    answer
}

/// One connection from a browser offering ALPS to `host` through
/// `terminating`, the site keeping its sessions as `acceptor` does.
fn kept_through(host: &str, terminating: &Terminating, acceptor: &SslAcceptor) -> (Kept, Vec<u8>) {
    runtime().block_on(async {
        let (browser_end, relay_browser) = duplex(1 << 16);
        let (relay_site, site_end) = duplex(1 << 16);
        let (_, answer, kept) = tokio::join!(
            terminate(
                relay_browser,
                relay_site,
                (Vec::new(), host),
                terminating,
                |_: Option<&ExceptionUsed>| {},
                |_: &Stop| {}
            ),
            alps_browser(browser_end, host),
            keeping_site(site_end, acceptor),
        );
        (kept, answer)
    })
}

/// An ALPS payload captured from a pinned build: HTTP/2 `SETTINGS` values,
/// which a build may send there (Chrome for Testing 156's is empty).
const CAPTURED: &[u8] = b"\x00\x01\x00\x01\x00\x00\x00\x02\x00\x00\x00\x00";

/// The relay sends a site the ALPS payload captured from the pinned build
/// when it was pinned, never the empty one `btls` sends, and never the
/// browser's own of the moment, which reaches the relay only after its
/// upstream handshake (ADR 0042's conformance comparison includes it).
/// Covers: story:4-39
#[test]
fn the_relay_sends_the_site_the_alps_payload_captured_from_the_pinned_build() {
    let stores = stores();
    let trusted = authority("Site authority");
    let host = "site.listed.test";
    let mut terminating = trusting(&stores, &trusted);
    terminating.application_settings =
        Arc::new([("h2".to_owned(), CAPTURED.to_vec())].into_iter().collect());

    let (kept, answer) = kept_through(host, &terminating, &keeping_acceptor(host, &trusted));

    assert_eq!(
        (kept.application_settings.as_deref(), answer.as_slice()),
        (Some(CAPTURED), b"pong".as_slice())
    );
}

/// The relay keeps a Session's upstream TLS sessions, as a returning
/// browser resumes: the Session's second connection to a site resumes the
/// first's session, while a new Session's first connection does not, its
/// tickets dropped with the Session that got them.
/// Covers: story:4-39
#[test]
fn a_returning_connection_resumes_the_sessions_upstream_tls_session_and_a_new_session_does_not() {
    let stores = stores();
    let trusted = authority("Site authority");
    let host = "site.listed.test";
    let acceptor = keeping_acceptor(host, &trusted);
    let session = trusting(&stores, &trusted);
    let next_session = trusting(&stores, &trusted);

    let resumed: Vec<(bool, Vec<u8>)> = [&session, &session, &next_session]
        .into_iter()
        .map(|terminating| {
            let (kept, answer) = kept_through(host, terminating, &acceptor);
            (kept.resumed, answer)
        })
        .collect();

    assert_eq!(
        resumed,
        vec![
            (false, b"pong".to_vec()),
            (true, b"pong".to_vec()),
            (false, b"pong".to_vec())
        ]
    );
}
