//! A Secret provider's endpoint through its interface: the wait a busy
//! answer's `Retry-After` asks for, read as every `Retry-After` the engine
//! meets is (ADR 0118), on the paused `Clock`.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::time::{Duration, UNIX_EPOCH};

use goodfetch_protocol::catalog::Product;
use hyper::Response;
use hyper::body::Bytes;
use rstest::rstest;

use super::SecretProviderEndpoint;
use crate::clock::Clock;
use crate::settings::{Layers, resolve};

/// 2015-10-21 07:28:00 UTC, the moment each answer is read.
const RECEIVED: u64 = 1_445_412_480;

/// An endpoint under the built-in settings, whose clock reads
/// [`RECEIVED`].
fn endpoint() -> SecretProviderEndpoint {
    let settings = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
    SecretProviderEndpoint::of(
        &settings,
        Clock::paused(UNIX_EPOCH + Duration::from_secs(RECEIVED)),
    )
}

/// An answer holding `headers`.
fn answer(headers: &[(&str, &str)]) -> Response<Bytes> {
    headers
        .iter()
        .fold(Response::builder().status(503), |answer, (name, value)| {
            answer.header(*name, *value)
        })
        .body(Bytes::new())
        .unwrap()
}

/// A busy answer's `Retry-After` is the wait it asks for, in seconds or as
/// an HTTP date measured against the answer's own `Date`; text that is
/// neither form, or none, asks none, and the driver's backoff waits instead.
#[rstest]
#[case::seconds(&[("retry-after", "120")], Some(Duration::from_secs(120)))]
#[case::seconds_with_spaces(&[("retry-after", " 3 ")], Some(Duration::from_secs(3)))]
#[case::no_wait(&[("retry-after", "0")], Some(Duration::ZERO))]
#[case::none(&[], None)]
#[case::an_http_date_after_the_answers_date(
    &[("retry-after", "Wed, 21 Oct 2015 07:30:00 GMT"), ("date", "Wed, 21 Oct 2015 07:28:00 GMT")],
    Some(Duration::from_secs(120))
)]
#[case::an_http_date_with_no_date(
    &[("retry-after", "Wed, 21 Oct 2015 07:29:00 GMT")],
    Some(Duration::from_secs(60))
)]
#[case::a_negative_number(&[("retry-after", "-1")], None)]
fn a_busy_answers_retry_after_is_the_wait_it_asks_for(
    #[case] headers: &[(&str, &str)],
    #[case] wait: Option<Duration>,
) {
    let asked = goodfetch_test_support::runtime()
        .block_on(async { endpoint().retry_after(&answer(headers)) });

    assert_eq!(asked, wait);
}
