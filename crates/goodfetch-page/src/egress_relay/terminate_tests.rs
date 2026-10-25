//! The relay's terminating mode in process, on every OS (spec #564's
//! Testing Decisions): the browser, the relay and the site are in-memory
//! streams, the browser and the site BoringSSL ends of their own, the site's
//! certificates made by a test authority. The oracles are what the site
//! received, what the browser was shown and the Run's pins.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::{Arc, Mutex};

use btls::pkey::PKey;
use btls::ssl::{SslAcceptor, SslConnector, SslMethod, SslVerifyMode};
use btls::x509::X509;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::{KeyHash, KnownCertificates};
use goodfetch_engine::secrets::ExceptionHosts;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{
    CertificateExceptions, ExceptionKind, ExceptionUsed, PinBook, Presented, Trust,
};
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use goodfetch_test_support::{TestEnv, runtime};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use rstest::rstest;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, duplex};

use super::authority::Authority;
use super::client_hello::{self, ClientHello};
use super::terminate::{
    Stop, Terminated, Terminating, failure_of, mirrors, request_trust_anchors, terminate, upstream,
};

/// The person whose pins the tests keep.
const PERSON: &str = "os_user";

/// A site certificate's defect.
#[derive(Clone, Copy, Debug)]
enum Made {
    Valid,
    Expired,
    NotYetValid,
    UnknownIssuer,
    NameMismatch,
    SelfSigned,
}

/// The test authority every site certificate but the unknown issuer's and
/// the self-signed one chains to.
struct SiteAuthority {
    params: CertificateParams,
    key: KeyPair,
    der: Vec<u8>,
}

fn authority(name: &str) -> SiteAuthority {
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
fn site_certificate(host: &str, made: Made, trusted: &SiteAuthority) -> (Vec<u8>, Vec<u8>) {
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

/// The site: a BoringSSL server presenting `certificate`, choosing `h2`,
/// which reads what comes and answers `pong` to it.
async fn site(stream: DuplexStream, (certificate, key): (Vec<u8>, Vec<u8>), received: &Received) {
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
    let acceptor = builder.build();
    let ssl = btls::ssl::Ssl::new(acceptor.context()).unwrap();
    let mut stream = tokio_btls::SslStream::new(ssl, stream).unwrap();
    if std::pin::Pin::new(&mut stream).accept().await.is_err() {
        return;
    }
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
        if stream.write_all(b"pong").await.is_err() {
            break;
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

/// The browser: a BoringSSL client offering `h2` and `http/1.1` with
/// GREASE, which takes any certificate, sends `ping` and reads the answer.
async fn browser(stream: DuplexStream, host: &str) -> Shown {
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
    stream.write_all(b"ping").await.unwrap();
    let mut answer = [0; 4];
    if stream.read_exact(&mut answer).await.is_ok() {
        shown.answer = answer.to_vec();
    }
    drop(stream);
    shown
}

/// A Run's stores: its pins and its Audit log.
struct Stores {
    store: KnownCertificates,
    book: PinBook,
    _env: TestEnv,
}

fn stores() -> Stores {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let store = KnownCertificates::of(&home, None);
    let book = PinBook {
        store: store.clone(),
        person: PERSON.to_owned(),
        run_id: None,
        audit: Arc::new(AuditLog::of(&home, None)),
        owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        clock: Clock::system(),
    };
    Stores {
        store,
        book,
        _env: env,
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
        examiner: trust(trusted).examiner(),
        trust: Trust::default_roots(),
    }
}

/// Mozilla's roots and `trusted`, for `listed.test`.
fn trust(trusted: &SiteAuthority) -> Trust {
    Trust::default_roots()
        .with_trusted_cas("listed.test", vec![trusted.der.clone().into()])
        .unwrap()
}

/// One browser connection to `host` through the terminating mode, its
/// site presenting `made`'s certificate: how the relay ended it, what the
/// browser was shown and what the site received.
fn through(
    host: &str,
    made: Made,
    stores: &Stores,
    trusted: &SiteAuthority,
) -> (Through, Shown, Vec<u8>, Arc<Authority>) {
    let terminating = terminating(stores, trusted);
    let certificate = site_certificate(host, made, trusted);
    let received = Received::default();
    let used = Mutex::new(Passed::Not);
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
                passed
            ),
            browser(browser_end, host),
            site(site_end, certificate, &received),
        );
        (terminated, shown)
    });
    let received = received.0.lock().unwrap().clone();
    let passed = used.into_inner().unwrap();
    (
        Through { terminated, passed },
        shown,
        received,
        Arc::clone(&terminating.authority),
    )
}

/// How the relay ended a connection, and the exception it said the
/// connection passed through, once it said it passed.
#[derive(Debug)]
struct Through {
    terminated: Terminated,
    passed: Passed,
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
            Some(authority.certificate()),
            Some(b"h2".as_slice()),
            b"pong".as_slice(),
            b"ping".as_slice(),
        )
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
                passed: Passed::Verified
            }
        ),
        "{through:?}"
    );
    assert_eq!(
        (shown.answer, received),
        (b"pong".to_vec(), b"ping".to_vec())
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
    stores
        .store
        .pin(
            PERSON,
            "expired.listed.test",
            &other,
            None,
            jiff::Timestamp::UNIX_EPOCH,
        )
        .unwrap();

    let (through, shown, received, _) =
        through("expired.listed.test", Made::Expired, &stores, &trusted);

    let Terminated::Stopped(Stop::Changed(changed)) = through.terminated else {
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
        .examiner();
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
                let _ = upstream(relay_site, "site.listed.test", Some(&read)).await;
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
        .add(CertificateDer::from(authority.certificate().to_vec()))
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
                UnixTime::now(),
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
