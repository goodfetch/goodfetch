//! The fingerprint an HTTP-only `stealth` Session presents (spec #579;
//! ADRs 0020 and 0035): `wreq` under its default HTTP profile, and the
//! Chrome for Testing build Collie pins, each fetch one page of the
//! Proving ground over TLS, and its observations (#159) show the same TLS
//! ClientHello and HTTP/2 settings, but for what BoringSSL in `btls` 0.5.6
//! cannot send.
//!
//! **What it cannot send:** Chrome 156 offers the TLS Trust Anchor
//! Identifiers extension (`0xca34`), one more extension its field-trial
//! configuration turns on (`0x12e0`), and the ML-DSA signature algorithms
//! (`0x0904` to `0x0906`); `btls` 0.5.6, the newest, has no setting for
//! any of them. They are named here exactly, so the test fails when the
//! browser or `btls` moves, and area 8's captured profiles (#1175) start
//! from them.

use std::collections::BTreeSet;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_limits::Limits;
use goodfetch_fetch_tier::{FetchBounds, Hop, HttpProfile, Wreq, fetch};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{
    ProvingGround, ReceivedRequests, TestEnv, browser_cache, chrome_for_testing_dumping_dom,
};
use serde_json::Value;
use url::Url;

use crate::{config, running_clock_runtime};

/// Any seed: the access family is served on every one.
const SEED: u64 = 772;

/// The page both clients fetch.
const HOST: &str = "1.access.test";

/// The extensions Chrome for Testing 156 sends that `btls` 0.5.6 cannot:
/// TLS Trust Anchor Identifiers, and the one its field trials add.
const UNSENDABLE_EXTENSIONS: [u64; 2] = [0xca34, 0x12e0];

/// The signature algorithms Chrome for Testing 156 offers that `btls`
/// 0.5.6 cannot: ML-DSA-44, -65 and -87.
const UNSENDABLE_SIGNATURE_ALGORITHMS: [u64; 3] = [0x0904, 0x0905, 0x0906];

/// Loads `url` once in the pinned Chrome for Testing, headless, its host
/// mapped to the Proving ground, and waits for it to exit.
fn chrome_loads(env: &TestEnv, proving_ground: &ProvingGround, url: &str) {
    let status = chrome_for_testing_dumping_dom(
        env,
        &browser_cache(env).unwrap(),
        (HOST, proving_ground.address().ip()),
        &[],
        url,
    )
    .unwrap()
    .close_stdin_and_wait()
    .unwrap();
    assert!(status.success(), "Chrome for Testing exited with {status}");
}

/// The first request the Proving ground received for `target` on [`HOST`].
fn request_for(observations: &Value, target: &str) -> Value {
    let request = ReceivedRequests::of(observations)
        .of_the_case()
        .find(|request| request.host.starts_with(HOST) && request.target == target)
        .unwrap_or_else(|| panic!("no request for {target}: {observations}"))
        .clone();
    serde_json::to_value(request).unwrap()
}

/// The numbers of a list of the observations.
fn numbers(list: &Value) -> Vec<u64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|number| number.as_u64().unwrap())
        .collect()
}

/// The JA4 fingerprint's first two parts: the protocol, version, server
/// name, counts and ALPN, then the ciphers' hash.
fn ja4_head(request: &Value) -> String {
    let ja4 = request["tls"]["ja4"].as_str().unwrap();
    ja4.rsplit_once('_').unwrap().0.to_owned()
}

/// Covers: story:5-122
#[test]
fn a_stealth_session_s_http_presents_its_browser_s_tls_and_http2_fingerprint() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();
    chrome_loads(&env, &proving_ground, &format!("https://{HOST}:{port}/"));
    let transport = Wreq::new(
        config(&proving_ground, None, &[HOST]),
        HttpProfile::default(),
    )
    .unwrap();
    let hop = Hop::get(Url::parse(&format!("https://{HOST}:{port}/?client=wreq")).unwrap());
    let deadlines = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    };
    running_clock_runtime()
        .block_on(fetch(&transport, &hop, &deadlines, &Clock::system()))
        .unwrap();

    let observations = proving_ground.observations().unwrap();
    let browser = request_for(&observations, "/");
    let stealth = request_for(&observations, "/?client=wreq");

    // HTTP/2, and its settings, window and pseudo-header order.
    assert_eq!(stealth["protocol"], browser["protocol"]);
    assert_eq!(stealth["http2"]["settings"], browser["http2"]["settings"]);
    assert_eq!(
        stealth["http2"]["window_update"],
        browser["http2"]["window_update"]
    );
    assert_eq!(
        stealth["pseudo_header_order"],
        browser["pseudo_header_order"]
    );
    // TLS: the version, ALPN and ciphers in order, so JA4's head.
    assert_eq!(stealth["tls"]["version"], browser["tls"]["version"]);
    assert_eq!(stealth["tls"]["alpn"], browser["tls"]["alpn"]);
    assert_eq!(stealth["tls"]["ciphers"], browser["tls"]["ciphers"]);
    assert_eq!(
        ja4_head(&stealth).split_once('_').unwrap().1,
        ja4_head(&browser).split_once('_').unwrap().1
    );
    // TLS: every extension the browser sends, but those `btls` cannot.
    let browser_extensions: BTreeSet<u64> =
        numbers(&browser["tls"]["extensions"]).into_iter().collect();
    let unsendable = BTreeSet::from(UNSENDABLE_EXTENSIONS);
    assert!(
        browser_extensions.is_superset(&unsendable),
        "the browser no longer sends {:?}: {browser_extensions:?}",
        unsendable
            .difference(&browser_extensions)
            .collect::<Vec<_>>()
    );
    let stealth_extensions: BTreeSet<u64> =
        numbers(&stealth["tls"]["extensions"]).into_iter().collect();
    assert_eq!(
        stealth_extensions,
        &browser_extensions - &unsendable,
        "browser: {browser}\nstealth: {stealth}"
    );
    // TLS: the browser's signature algorithms in order, but ML-DSA.
    let browser_algorithms = numbers(&browser["tls"]["signature_algorithms"]);
    assert!(
        UNSENDABLE_SIGNATURE_ALGORITHMS
            .iter()
            .all(|algorithm| browser_algorithms.contains(algorithm)),
        "the browser no longer offers ML-DSA: {browser_algorithms:?}"
    );
    assert_eq!(
        numbers(&stealth["tls"]["signature_algorithms"]),
        browser_algorithms
            .into_iter()
            .filter(|algorithm| !UNSENDABLE_SIGNATURE_ALGORITHMS.contains(algorithm))
            .collect::<Vec<_>>()
    );
}
