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

/// The Console's names, and a content origin's beside them.
fn console() -> Authorities {
    Authorities::loopback(SocketAddr::from(([127, 0, 0, 1], 4000)))
}

fn content() -> Authorities {
    Authorities::only("content.example")
}

/// Each listener's `Host` and `Origin` rule: the ops listener takes any
/// address and no `Origin`; a WebSocket handshake or a Console write needs
/// one of its own origins; a Console read takes its own or none; the
/// content origin takes its own, the Console's or none. The `Host` is
/// judged first.
///
/// Covers: story:4-21
#[rstest]
#[case::ops_from_a_pods_address("ops", Some("10.0.0.7:9464"), None, None)]
#[case::ops_from_a_rebinding_name(
    "ops",
    Some("attacker.example"),
    None,
    Some(Refusal::HostForeign)
)]
#[case::ops_from_a_page(
    "ops",
    Some("127.0.0.1:9464"),
    Some("http://127.0.0.1:9464"),
    Some(Refusal::OriginForeign)
)]
#[case::handshake_from_its_own_page(
    "handshake",
    Some("localhost:4000"),
    Some("http://127.0.0.1:4000"),
    None
)]
#[case::handshake_with_no_origin(
    "handshake",
    Some("127.0.0.1:4000"),
    None,
    Some(Refusal::OriginMissing)
)]
#[case::handshake_from_another_page(
    "handshake",
    Some("127.0.0.1:4000"),
    Some("http://attacker.example"),
    Some(Refusal::OriginForeign)
)]
#[case::handshake_naming_an_address_it_is_not_reached_at(
    "handshake",
    Some("127.0.0.2:4000"),
    Some("http://127.0.0.2:4000"),
    Some(Refusal::HostForeign)
)]
#[case::handshake_over_https(
    "handshake",
    Some("127.0.0.1:4000"),
    Some("https://127.0.0.1:4000"),
    Some(Refusal::OriginForeign)
)]
#[case::read_with_no_origin("read", Some("LOCALHOST:4000"), None, None)]
#[case::read_from_another_page(
    "read",
    Some("127.0.0.1:4000"),
    Some("http://attacker.example"),
    Some(Refusal::OriginForeign)
)]
#[case::read_with_no_host("read", None, None, Some(Refusal::HostForeign))]
#[case::content_framed_by_the_console(
    "content",
    Some("content.example"),
    Some("http://localhost:4000"),
    None
)]
#[case::content_naming_the_consoles_host(
    "content",
    Some("127.0.0.1:4000"),
    None,
    Some(Refusal::HostForeign)
)]
#[case::content_from_another_page(
    "content",
    Some("content.example"),
    Some("http://attacker.example"),
    Some(Refusal::OriginForeign)
)]
fn a_request_passes_only_with_a_host_and_origin_its_listener_admits(
    #[case] listener: &str,
    #[case] host: Option<&str>,
    #[case] origin: Option<&str>,
    #[case] refused: Option<Refusal>,
) {
    let (console, content) = (console(), content());
    let (hosts, origins) = match listener {
        "ops" => (Hosts::Address, Origins::Forbidden),
        "handshake" => (Hosts::Named(&console), Origins::Required(&[&console])),
        "read" => (Hosts::Named(&console), Origins::Allowed(&[&console])),
        _ => (
            Hosts::Named(&content),
            Origins::Allowed(&[&content, &console]),
        ),
    };

    assert_eq!(refusal(host, origin, hosts, origins), refused);
}
