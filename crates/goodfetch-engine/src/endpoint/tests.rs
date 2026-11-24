//! The endpoint rule's tokens and `Host` and `Origin` checks at its
//! interface: a token is 256 bits of the OS generator, only the same token
//! is the same, the comparison takes as long whatever prefix a guess
//! shares with the token, and a request's `Host` and `Origin` pass only
//! where the listener admits them.

use std::hint::black_box;

use chacha20::ChaCha20Rng;
use chacha20::rand_core::{Rng as _, SeedableRng as _};
use rstest::rstest;

use std::net::SocketAddr;

use super::{
    Authorities, Hosts, Origins, Refusal, names_an_address, new_token, refusal, same_token,
};

/// A token is 64 lowercase hex digits, 256 bits, and no two are alike.
///
/// Covers: story:4-21
#[test]
fn a_token_is_256_bits_of_hex_and_each_one_is_new() {
    let (first, second) = (new_token().unwrap(), new_token().unwrap());

    assert_eq!(
        (
            first.len(),
            first
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            first == second,
        ),
        (64, true, false)
    );
}

/// Only the token itself is the same token: not one differing in its first
/// or last letter, nor its prefix, nor itself and more.
///
/// Covers: story:4-21
#[rstest]
#[case::itself("0123456789abcdef", true)]
#[case::its_first_letter_changed("1123456789abcdef", false)]
#[case::its_last_letter_changed("0123456789abcdee", false)]
#[case::its_prefix("0123456789abcde", false)]
#[case::itself_and_more("0123456789abcdef0", false)]
#[case::nothing("", false)]
fn only_the_same_token_is_the_same(#[case] given: &str, #[case] same: bool) {
    assert_eq!(same_token(given, "0123456789abcdef"), same);
}

/// Comparisons each sample times, so one sample outlasts the clock's
/// resolution.
const BATCH: usize = 64;

/// Samples of each kind of guess.
const SAMPLES: usize = 4_000;

/// Above this Welch's t, the two kinds of guess take measurably different
/// times: dudect's threshold for a leak is 4.5, and this one leaves room
/// for a loaded machine, while an early exit gives a t in the hundreds.
const LEAK: f64 = 10.0;

/// How long `BATCH` comparisons of `guess` with `token` take, in
/// nanoseconds.
#[expect(
    clippy::disallowed_methods,
    reason = "the property is the comparison's own run time, which only the OS clock measures"
)]
fn timed(guess: &str, token: &str) -> f64 {
    let started = std::time::Instant::now();
    for _ in 0..BATCH {
        black_box(same_token(black_box(guess), black_box(token)));
    }
    started.elapsed().as_secs_f64() * 1e9
}

/// Welch's t between two samples.
fn welch(first: &[f64], second: &[f64]) -> f64 {
    let moments = |sample: &[f64]| {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a sample's size is far below 2^52"
        )]
        let size = sample.len() as f64;
        let mean = sample.iter().sum::<f64>() / size;
        let variance = sample
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (size - 1.0);
        (mean, variance / size)
    };
    let ((first_mean, first_error), (second_mean, second_error)) =
        (moments(first), moments(second));
    (first_mean - second_mean) / (first_error + second_error).sqrt()
}

/// A guess that differs from `token` only at byte `at`.
fn differing_at(token: &str, at: usize) -> String {
    let mut bytes = token.as_bytes().to_vec();
    bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
    String::from_utf8(bytes).unwrap()
}

/// A guess sharing no prefix with the token and one sharing all of it but
/// its last letter take as long to refuse: the two kinds, drawn in an order
/// a seed sets so a load on the machine slows both alike, show no
/// difference Welch's t finds, once each kind's slowest tenth, a preempted
/// sample's, is left out.
///
/// Covers: story:4-21
#[test]
fn a_guess_takes_as_long_to_refuse_whatever_prefix_it_shares_with_the_token() {
    let token = new_token().unwrap();
    let guesses = [
        differing_at(&token, 0),
        differing_at(&token, token.len() - 1),
    ];
    let mut order = ChaCha20Rng::seed_from_u64(0x0e4d_9017);
    let mut times: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    for guess in &guesses {
        timed(guess, &token);
    }
    while times.iter().any(|kind| kind.len() < SAMPLES) {
        let kind = usize::from(order.next_u32() % 2 == 1);
        if times[kind].len() < SAMPLES {
            times[kind].push(timed(&guesses[kind], &token));
        }
    }
    for kind in &mut times {
        kind.sort_by(f64::total_cmp);
        kind.truncate(SAMPLES * 9 / 10);
    }

    let t = welch(&times[0], &times[1]);

    assert!(
        t.abs() < LEAK,
        "Welch's t {t} between the two kinds of guess"
    );
}

/// A `Host` naming an IP address or `localhost` passes, with or without a
/// port; a name a DNS-rebinding page points at the listener, or none, does
/// not.
///
/// Covers: story:4-21
#[rstest]
#[case::loopback_with_a_port(Some("127.0.0.1:8080"), true)]
#[case::an_ipv6_address(Some("[::1]:8080"), true)]
#[case::localhost(Some("LocalHost"), true)]
#[case::a_pods_address(Some("10.0.0.7:9464"), true)]
#[case::a_rebinding_name(Some("attacker.example:8080"), false)]
#[case::localhost_as_a_subdomain(Some("localhost.attacker.example"), false)]
#[case::none(None, false)]
fn a_host_naming_an_address_passes(#[case] host: Option<&str>, #[case] passes: bool) {
    assert_eq!(names_an_address(host), passes);
}

/// An Exposed Daemon's names.
fn exposed() -> Authorities {
    Authorities::loopback(SocketAddr::from(([127, 0, 0, 1], 4000)))
}

/// An Exposed Daemon's names once it terminated TLS on a Connection to
/// `192.0.2.1:port` that named `daemon.example`.
fn exposed_over_tls(port: u16) -> Authorities {
    Authorities::loopback(SocketAddr::from(([0, 0, 0, 0], port))).over_tls(
        Some("daemon.example"),
        Some(SocketAddr::from(([192, 0, 2, 1], port))),
    )
}

/// A listener whose `Host` and `Origin` rule a table's row names.
#[derive(Clone, Copy)]
enum Listener {
    Ops,
    Handshake,
    /// A handshake over TLS terminated at this port.
    TlsHandshake(u16),
    /// A handshake a trusted proxy forwarded with these `X-Forwarded-Host`
    /// and `X-Forwarded-Proto`, TLS terminated in front of the Daemon.
    Forwarded(Option<&'static str>, Option<&'static str>),
}

/// Each listener's `Host` and `Origin` rule: the ops listener takes any
/// address and no `Origin`; a WebSocket handshake needs one of its own
/// origins. The `Host` is judged first.
///
/// Covers: story:4-21
#[rstest]
#[case::ops_from_a_pods_address(Listener::Ops, Some("10.0.0.7:9464"), None, None)]
#[case::ops_from_a_rebinding_name(
    Listener::Ops,
    Some("attacker.example"),
    None,
    Some(Refusal::HostForeign)
)]
#[case::ops_from_a_page(
    Listener::Ops,
    Some("127.0.0.1:9464"),
    Some("http://127.0.0.1:9464"),
    Some(Refusal::OriginForeign)
)]
#[case::handshake_from_its_own_page(
    Listener::Handshake,
    Some("localhost:4000"),
    Some("http://127.0.0.1:4000"),
    None
)]
#[case::handshake_with_no_origin(
    Listener::Handshake,
    Some("127.0.0.1:4000"),
    None,
    Some(Refusal::OriginMissing)
)]
#[case::handshake_from_another_page(
    Listener::Handshake,
    Some("127.0.0.1:4000"),
    Some("http://attacker.example"),
    Some(Refusal::OriginForeign)
)]
#[case::handshake_naming_an_address_it_is_not_reached_at(
    Listener::Handshake,
    Some("127.0.0.2:4000"),
    Some("http://127.0.0.2:4000"),
    Some(Refusal::HostForeign)
)]
#[case::handshake_over_https(
    Listener::Handshake,
    Some("127.0.0.1:4000"),
    Some("https://127.0.0.1:4000"),
    Some(Refusal::OriginForeign)
)]
#[case::tls_handshake_naming_its_server_name(
    Listener::TlsHandshake(4000),
    Some("daemon.example:4000"),
    Some("https://daemon.example:4000"),
    None
)]
#[case::tls_handshake_naming_the_address_it_reached(
    Listener::TlsHandshake(4000),
    Some("192.0.2.1:4000"),
    Some("https://192.0.2.1:4000"),
    None
)]
#[case::tls_handshake_at_the_default_port(
    Listener::TlsHandshake(443),
    Some("daemon.example"),
    Some("https://daemon.example"),
    None
)]
#[case::tls_handshake_from_a_page_over_http(
    Listener::TlsHandshake(4000),
    Some("daemon.example:4000"),
    Some("http://daemon.example:4000"),
    Some(Refusal::OriginForeign)
)]
#[case::tls_handshake_naming_another_name(
    Listener::TlsHandshake(4000),
    Some("attacker.example:4000"),
    Some("https://attacker.example:4000"),
    Some(Refusal::HostForeign)
)]
#[case::forwarded_over_https_naming_its_public_name(
    Listener::Forwarded(Some("daemon.example"), Some("https")),
    Some("daemon.example"),
    Some("https://daemon.example"),
    None
)]
#[case::forwarded_with_the_proxy_host_of_its_own(
    Listener::Forwarded(Some("daemon.example"), Some("https")),
    Some("127.0.0.1:4000"),
    Some("https://daemon.example"),
    None
)]
#[case::forwarded_reading_each_headers_first_value(
    Listener::Forwarded(Some("daemon.example, inner.example"), Some("HTTPS, http")),
    Some("daemon.example"),
    Some("https://daemon.example"),
    None
)]
#[case::forwarded_naming_a_later_value(
    Listener::Forwarded(Some("daemon.example, inner.example"), Some("https, http")),
    Some("inner.example"),
    Some("https://inner.example"),
    Some(Refusal::HostForeign)
)]
#[case::forwarded_over_http(
    Listener::Forwarded(Some("daemon.example"), Some("http")),
    Some("daemon.example"),
    Some("http://daemon.example"),
    Some(Refusal::HostForeign)
)]
#[case::forwarded_with_no_proto(
    Listener::Forwarded(Some("daemon.example"), None),
    Some("daemon.example"),
    Some("http://daemon.example"),
    Some(Refusal::HostForeign)
)]
#[case::forwarded_from_another_page(
    Listener::Forwarded(Some("daemon.example"), Some("https")),
    Some("daemon.example"),
    Some("https://attacker.example"),
    Some(Refusal::OriginForeign)
)]
#[case::forwarded_from_a_page_over_http(
    Listener::Forwarded(Some("daemon.example"), Some("https")),
    Some("daemon.example"),
    Some("http://daemon.example"),
    Some(Refusal::OriginForeign)
)]
fn a_request_passes_only_with_a_host_and_origin_its_listener_admits(
    #[case] listener: Listener,
    #[case] host: Option<&str>,
    #[case] origin: Option<&str>,
    #[case] refused: Option<Refusal>,
) {
    let exposed = match listener {
        Listener::TlsHandshake(port) => exposed_over_tls(port),
        Listener::Forwarded(forwarded_host, proto) => exposed().forwarded(forwarded_host, proto),
        Listener::Ops | Listener::Handshake => exposed(),
    };
    let (hosts, origins) = match listener {
        Listener::Ops => (Hosts::Address, Origins::Forbidden),
        Listener::Handshake | Listener::TlsHandshake(_) | Listener::Forwarded(..) => {
            (Hosts::Named(&exposed), Origins::Required(&[&exposed]))
        }
    };

    assert_eq!(refusal(host, origin, hosts, origins), refused);
}
