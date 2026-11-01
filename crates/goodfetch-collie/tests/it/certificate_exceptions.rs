//! The egress relay's terminating mode on Chromium over stdio (spec #564;
//! ADRs 0042 and 0124): a host the Run trusts an authority for, or lists
//! for an invalid-certificate exception, is terminated by the Session's
//! relay, which presents the browser a leaf of the Run's in-memory
//! authority and connects upstream itself under the Run's pins. The
//! Proving ground's `certificates` family serves pitfall H03's
//! certificates; its observations, the Run's Provenance and the files the
//! Run left are the oracles.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the files a Run left on their own thread, never an async worker"
)]

use std::path::Path;

use data_encoding::BASE64;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::egress_relay::{PAST_AUTO_RELOAD, Program, error, provenance, started};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 635;

/// A second seed, whose certificates carry other keys under the same names.
const OTHER_SEED: u64 = 636;

/// Pitfall H03's hosts, each with the failure the relay names.
const H03: [(&str, &str); 5] = [
    ("expired.certificates.test", "expired"),
    ("not-yet-valid.certificates.test", "not_yet_valid"),
    ("unknown-issuer.certificates.test", "unknown_issuer"),
    ("name-mismatch.certificates.test", "name_mismatch"),
    ("self-signed.certificates.test", "self_signed"),
];

/// The `https://` URL of `path` on `host` at the Proving ground.
fn secure(proving_ground: &ProvingGround, host: &str, path: &str) -> String {
    format!("https://{host}:{}{path}", proving_ground.address().port())
}

/// The user configuration trusting the Proving ground's authority for the
/// `certificates` family, as an operator configures it.
fn trusting(env: &TestEnv, proving_ground: &ProvingGround) {
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!(
        "tls.trusted_cas = {{ \"certificates.test\" = [{}] }}\n",
        json!(ca)
    ));
}

/// The requests the Proving ground's sites received, each as its host and
/// the cookies it carried.
fn requests(proving_ground: &ProvingGround) -> Vec<(String, Value)> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request["target"] != json!("/favicon.ico"))
        .map(|request| {
            (
                request["host"]
                    .as_str()
                    .unwrap_or_default()
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                request["cookies"].clone(),
            )
        })
        .collect()
}

/// The status of the response a navigation's answer reached.
fn reached(answer: &Value) -> Value {
    answer["result"]["response"]["status"].clone()
}

/// Pitfall H03 on a browser: each invalid certificate is refused with
/// `certificate_invalid` by a Chromium Session through the relay, the
/// failure named, and each passes under its per-domain exception, as the
/// Run's Provenance records each host the relay terminated with the key it
/// presented (positive control).
///
/// Covers: story:4-40, story:4-35, error:certificate_invalid, setting:tls.accept_invalid_certs, setting:tls.trusted_cas, operation:page.goto
#[test]
fn h03_each_invalid_certificate_is_refused_through_the_relay_and_passes_under_its_exception() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    trusting(&env, &proving_ground);

    let mut refusing = started(&env, &proving_ground, "refusing", &json!({}), &[]);
    let page = refusing.new_page("refusing");
    let refused: Vec<(Value, Value, Value)> = H03
        .iter()
        .map(|(host, _)| {
            let (code, details) = error(&refusing.goto(&page, &secure(&proving_ground, host, "/")));
            (code, details["host"].clone(), details["failure"].clone())
        })
        .collect();
    refusing.finish("refusing");
    let reached_while_refused = requests(&proving_ground).len();

    let exceptions = json!({
        "tls": { "accept_invalid_certs": { "certificates.test": null } }
    });
    let mut excepted = started(&env, &proving_ground, "excepted", &exceptions, &[]);
    let page = excepted.new_page("excepted");
    let passed: Vec<Value> = H03
        .iter()
        .map(|(host, _)| reached(&excepted.goto(&page, &secure(&proving_ground, host, "/"))))
        .collect();
    excepted.finish("excepted");

    assert_eq!(
        refused,
        H03.iter()
            .map(|(host, failure)| (json!("certificate_invalid"), json!(host), json!(failure)))
            .collect::<Vec<_>>()
    );
    assert_eq!(reached_while_refused, 0);
    assert_eq!(passed, vec![json!(200); H03.len()]);
    let relay = provenance(&env, "excepted", "egress_relay")[0]["address"].clone();
    let terminated: Vec<(Value, Value, Value, bool)> =
        provenance(&env, "excepted", "tls_terminated")
            .into_iter()
            .map(|entry| {
                (
                    entry["relay"].clone(),
                    entry["host"].clone(),
                    entry["exception"].clone(),
                    entry["spki_sha256"]
                        .as_str()
                        .is_some_and(|key| key.starts_with("sha256/")),
                )
            })
            .collect();
    assert_eq!(
        terminated,
        H03.iter()
            .map(|(host, _)| (
                relay.clone(),
                json!(host),
                json!("trust_on_first_use"),
                true
            ))
            .collect::<Vec<_>>()
    );
}

/// A page whose navigation the relay refused stays on its error page: the
/// browser does not load it again on its own, so no request the program
/// did not make reaches the relay, and the page's next navigation raises
/// its own host's failure, never the earlier host's (fix-queue 83, where
/// Chromium's reload of the error page, a second after it, left the earlier
/// host's refusal for the next call). Positive control: the first
/// navigation raises its host's failure.
///
/// Covers: error:certificate_invalid, operation:page.goto
#[test]
fn a_page_left_on_its_error_page_is_not_reloaded_and_the_next_navigation_raises_its_own_host() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    trusting(&env, &proving_ground);
    let mut program = started(&env, &proving_ground, "left", &json!({}), &[]);
    let page = program.new_page("left");

    let (first, first_details) = error(&program.goto(
        &page,
        &secure(&proving_ground, "expired.certificates.test", "/"),
    ));
    // Nothing to wait on: the page stays as it is, which only time shows.
    std::thread::sleep(PAST_AUTO_RELOAD);
    let (next, next_details) = error(&program.goto(
        &page,
        &secure(&proving_ground, "not-yet-valid.certificates.test", "/"),
    ));
    program.finish("left");

    assert_eq!(
        [
            (first, first_details["host"].clone()),
            (next, next_details["host"].clone())
        ],
        [
            (
                json!("certificate_invalid"),
                json!("expired.certificates.test")
            ),
            (
                json!("certificate_invalid"),
                json!("not-yet-valid.certificates.test")
            ),
        ]
    );
}

/// A04-18: a pinned host whose key changed stops with
/// `certificate_changed`, naming both keys, for every later navigation of
/// the Run, and the site receives nothing of the browser's. Positive
/// control: the same host passed with the key it was pinned to.
///
/// Covers: story:4-36, error:certificate_changed, format:known-certificates
#[test]
fn a_pinned_host_whose_key_changed_stops_the_session_and_receives_nothing() {
    let env = TestEnv::new();
    let host = "expired.certificates.test";
    let exceptions = json!({ "tls": { "accept_invalid_certs": { (host): null } } });
    let pinning = ProvingGround::start(&env, SEED).unwrap();
    let mut first = started(&env, &pinning, "pinning", &exceptions, &[]);
    let page = first.new_page("pinning");
    let pinned = reached(&first.goto(&page, &secure(&pinning, host, "/")));
    first.finish("pinning");
    drop(pinning);

    let changed = ProvingGround::start(&env, OTHER_SEED).unwrap();
    let mut second = started(&env, &changed, "changed", &exceptions, &[]);
    let page = second.new_page("changed");
    let stopped = error(&second.goto(&page, &secure(&changed, host, "/")));
    let again = error(&second.goto(&page, &secure(&changed, host, "/again")));
    second.finish("changed");

    assert_eq!(pinned, json!(200));
    let (code, details) = stopped;
    assert_eq!(
        (
            code,
            &details["host"],
            details["pinned_spki_sha256"] != details["presented_spki_sha256"]
        ),
        (json!("certificate_changed"), &json!(host), true),
        "{details}"
    );
    assert_eq!(again.0, json!("certificate_changed"));
    assert_eq!(requests(&changed), vec![]);
}

/// No Session cookie goes to an exception host: a cookie on the whole
/// `certificates.test` domain goes to a host of the family the Run lists
/// no exception for (positive control), and a navigation to the host it
/// does list fails with `secret_input_refused` before any request reaches
/// it.
///
/// Covers: story:4-38, error:secret_input_refused
#[test]
fn no_session_cookie_is_sent_to_a_host_reached_through_an_exception() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let host = "expired.certificates.test";
    let exceptions = json!({ "tls": { "accept_invalid_certs": { (host): null } } });
    let mut program = started(&env, &proving_ground, "cookies", &exceptions, &[]);
    let page = program.new_page("cookies");
    let plain =
        |host: &str, path: &str| format!("http://{host}:{}{path}", proving_ground.address().port());

    let cookie_set = reached(&program.goto(
        &page,
        &plain("name-mismatch.certificates.test", "/set-cookie"),
    ));
    let sent = reached(&program.goto(&page, &plain("self-signed.certificates.test", "/")));
    let (code, details) = error(&program.goto(&page, &secure(&proving_ground, host, "/")));
    program.finish("cookies");

    assert_eq!((cookie_set, sent), (json!(200), json!(200)));
    let received = requests(&proving_ground);
    assert!(
        received.iter().any(
            |(received, cookies)| received == "self-signed.certificates.test"
                && cookies.to_string().contains("session")
        ),
        "{received:?}"
    );
    assert_eq!(
        (code, &details["reason"], &details["domain"]),
        (
            json!("secret_input_refused"),
            &json!("invalid_certificate_exception"),
            &json!(host)
        ),
        "{details}"
    );
    assert!(
        received.iter().all(|(received, _)| received != host),
        "{received:?}"
    );
}

/// The function `page.evaluate` runs to open a WebSocket to `url` and
/// answer how it ended: `open`, or `error` once the handshake failed.
fn web_socket(url: &str) -> String {
    format!(
        "() => new Promise((done) => {{ const socket = new WebSocket({}); socket.onopen = () => {{ socket.close(); done('open'); }}; socket.onerror = () => done('error'); }})",
        json!(url)
    )
}

/// No Session cookie goes to an exception host on a request the
/// interception never pauses either (deep review finding 564-1): a page's
/// WebSocket handshake to the host reaches it while the Session holds no
/// cookie for it (positive control), and once the Session holds one, the
/// relay, which reads every request on the terminated connection, closes
/// the handshake before it reaches the host.
///
/// Covers: story:4-38
#[test]
fn a_web_socket_handshake_carries_no_session_cookie_to_a_host_reached_through_an_exception() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let host = "expired.certificates.test";
    let exceptions = json!({ "tls": { "accept_invalid_certs": { (host): null } } });
    let mut program = started(&env, &proving_ground, "web-socket", &exceptions, &[]);
    let page = program.new_page("web-socket");
    let setter = program.new_page("web-socket");
    let socket = format!("wss://{host}:{}/ws", proving_ground.address().port());
    let open = |program: &mut Program<'_>| {
        program.call(
            "page.evaluate",
            &json!({ "page_id": page, "function": web_socket(&socket) }),
        )
    };

    let opened = reached(&program.goto(&page, &secure(&proving_ground, host, "/")));
    let before = open(&mut program);
    let cookie_set = reached(&program.goto(
        &setter,
        &format!(
            "http://name-mismatch.certificates.test:{}/set-cookie",
            proving_ground.address().port()
        ),
    ));
    let after = open(&mut program);
    program.finish("web-socket");

    assert_eq!(
        (opened, cookie_set),
        (json!(200), json!(200)),
        "{before} {after}"
    );
    let handshakes: Vec<Value> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| {
            request["kind"] == json!("websocket")
                && request["host"]
                    .as_str()
                    .is_some_and(|received| received.starts_with(host))
        })
        .map(|request| request["cookies"].clone())
        .collect();
    assert_eq!(handshakes, vec![json!([])], "{before} {after}");
}

/// The exception host's own host-only cookie is a Session cookie too: the
/// engine cannot tell one set through the exception from one the Session
/// held before (an imported Session, a sign-in made while the certificate
/// was valid, a `__Host-` cookie), so a cookie set by the host over plain
/// `http:` keeps its `https:` navigation from leaving, as the engine's own
/// client sends no cookie under an exception (ADR 0042). No request
/// reaches the host over TLS.
///
/// Covers: story:4-38, error:secret_input_refused
#[test]
fn the_exception_hosts_own_host_only_cookie_is_withheld_too() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let host = "expired.certificates.test";
    let exceptions = json!({ "tls": { "accept_invalid_certs": { (host): null } } });
    let mut program = started(&env, &proving_ground, "host-only", &exceptions, &[]);
    let page = program.new_page("host-only");
    let plain = format!("http://{host}:{}/", proving_ground.address().port());

    let opened = reached(&program.goto(&page, &plain));
    let set = program.call(
        "page.evaluate",
        &json!({ "page_id": page, "function": "() => { document.cookie = 'login=h0st; Path=/'; return document.cookie; }" }),
    );
    let (code, details) = error(&program.goto(&page, &secure(&proving_ground, host, "/")));
    program.finish("host-only");

    assert_eq!(
        (opened, &set["result"]["value"]),
        (json!(200), &json!("login=h0st")),
        "{set}"
    );
    assert_eq!(
        (code, &details["reason"], &details["placeholder"]),
        (
            json!("secret_input_refused"),
            &json!("invalid_certificate_exception"),
            &json!("login")
        ),
        "{details}"
    );
    let over_tls: Vec<Value> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|received| received.starts_with(host))
                && !request["tls"].is_null()
        })
        .map(|request| request["target"].clone())
        .collect();
    assert_eq!(over_tls, Vec::<Value>::new());
}

/// A host the relay only tunnels, its certificate refused by Chromium
/// itself, is `certificate_invalid` too, and the site sees one TLS
/// fingerprint, the browser's (ADR 0042): the relay's own look at the
/// certificate, which would tell a self-signed certificate from an unknown
/// issuer, sends the browser's ClientHello again only when its BoringSSL
/// carries all of it. Chrome for Testing offers ML-DSA signature
/// algorithms it does not, so the relay opens no handshake of its own and
/// Chromium's name stands: both are `unknown_issuer`. Nothing reaches the
/// site.
///
/// Covers: error:certificate_invalid, operation:page.goto
#[test]
fn a_certificate_chromium_refuses_on_a_tunnelled_host_is_invalid_and_the_site_sees_one_fingerprint()
{
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut program = started(&env, &proving_ground, "tunnelled", &json!({}), &[]);
    let page = program.new_page("tunnelled");

    let refused: Vec<(Value, Value, Value)> = [
        "self-signed.certificates.test",
        "unknown-issuer.certificates.test",
    ]
    .iter()
    .map(|host| {
        let (code, details) = error(&program.goto(&page, &secure(&proving_ground, host, "/")));
        (code, details["host"].clone(), details["failure"].clone())
    })
    .collect();
    program.finish("tunnelled");

    assert_eq!(
        refused,
        vec![
            (
                json!("certificate_invalid"),
                json!("self-signed.certificates.test"),
                json!("unknown_issuer")
            ),
            (
                json!("certificate_invalid"),
                json!("unknown-issuer.certificates.test"),
                json!("unknown_issuer")
            ),
        ]
    );
    assert_eq!(requests(&proving_ground), vec![]);
    assert_eq!(
        provenance(&env, "tunnelled", "tls_terminated"),
        Vec::<Value>::new()
    );
    let handshakes: Vec<(Value, Value)> = proving_ground.observations().unwrap()["tls_handshakes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|handshake| {
            handshake["sni"]
                .as_str()
                .is_some_and(|host| host.ends_with(".certificates.test"))
        })
        .map(|handshake| (handshake["sni"].clone(), handshake["ja4_base"].clone()))
        .collect();
    assert!(
        handshakes.len() >= 2 && handshakes.iter().all(|(_, ja4)| ja4 == &handshakes[0].1),
        "one fingerprint, the browser's: {handshakes:?}"
    );
}

/// The DER prefix of a P-256 `SubjectPublicKeyInfo`, which the relay's
/// authority's key has.
const P256_SPKI: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The DER prefix of a P-256 private key in PKCS #8, as the authority's
/// key would be kept.
const P256_PKCS8: [u8; 26] = [
    0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
    0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x6d,
];

/// Each file under `folder`, with its bytes.
fn files(folder: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(path),
                Ok(kind) if kind.is_file() => {
                    if let Ok(bytes) = std::fs::read(&path) {
                        found.push((path.display().to_string(), bytes));
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// The files of `found` that hold the public key whose SHA-256 is
/// `spki_sha256` in base64, as a certificate or a key holds it, or any
/// P-256 private key at all.
fn holding(found: &[(String, Vec<u8>)], spki_sha256: &str) -> Vec<String> {
    found
        .iter()
        .filter(|(_, bytes)| {
            let key = bytes
                .windows(91)
                .filter(|window| window.starts_with(&P256_SPKI))
                .any(|window| BASE64.encode(&Sha256::digest(window)) == spki_sha256);
            let private = bytes
                .windows(P256_PKCS8.len())
                .any(|window| window == P256_PKCS8);
            key || private
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// Story 4-40: the relay's authority lives in memory only. After a Run
/// that terminated a host, no file the Goodfetch home holds (the Run's
/// folder with its Browser data folder and trace, the stores and the
/// configuration) carries the authority's key, its certificate or any
/// P-256 private key. Positive control: the search finds the key in a file
/// that holds it.
///
/// Covers: story:4-40
#[test]
fn no_authority_key_or_certificate_is_written_to_disk() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let host = "self-signed.certificates.test";
    let exceptions = json!({ "tls": { "accept_invalid_certs": { (host): null } } });
    let mut program = started(&env, &proving_ground, "in-memory", &exceptions, &[]);
    let page = program.new_page("in-memory");
    let passed = reached(&program.goto(&page, &secure(&proving_ground, host, "/")));
    program.finish("in-memory");

    let authority = provenance(&env, "in-memory", "relay_authority");
    let spki_sha256 = authority[0]["spki_sha256"].as_str().unwrap().to_owned();
    let found = files(env.home());
    let control = {
        let mut spki = P256_SPKI.to_vec();
        spki.extend_from_slice(&[0x04; 65]);
        (
            "control".to_owned(),
            [b"before".as_slice(), &spki, b"after"].concat(),
        )
    };
    let control_hash = BASE64.encode(&Sha256::digest(&control.1[6..97]));

    assert_eq!(passed, json!(200));
    assert_eq!(
        (authority.len(), &authority[0]["domains"]),
        (1, &json!([host]))
    );
    assert_ne!(found.len(), 0);
    assert_eq!(holding(&found, &spki_sha256), Vec::<String>::new());
    assert_eq!(
        holding(&[control], &control_hash),
        vec!["control".to_owned()]
    );
}
