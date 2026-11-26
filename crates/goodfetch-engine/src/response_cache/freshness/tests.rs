//! RFC 9111's rules as the Response cache applies them, on requests and
//! responses built here: what is stored in the public partition, freshness
//! as a pure function of the moment the Clock gives, the conditions a
//! stale response is revalidated with, and the headers a 304 updates.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::time::{Duration, SystemTime};

use bytes::Bytes;
use proptest::prelude::*;
use rstest::rstest;

use super::{
    CacheRequest, Freshness, ResponseHead, Stored, freshness, not_modified, replayable,
    served_headers, storable,
};
use crate::clock::Clock;

/// ADR 0059's test `Clock` start.
fn start() -> SystemTime {
    goodfetch_protocol::point_in_time::from_rfc3339("2026-01-05T09:00:00Z").unwrap()
}

fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn get(headers: &[(&str, &str)]) -> CacheRequest {
    CacheRequest::new(
        "GET",
        &"https://cache.test/page".parse().unwrap(),
        &pairs(headers),
        Bytes::new(),
    )
}

fn post(headers: &[(&str, &str)]) -> CacheRequest {
    CacheRequest {
        method: "POST".to_owned(),
        body: Bytes::from_static(br#"{"q":"shoes"}"#),
        ..get(headers)
    }
}

fn head(status: u16, headers: &[(&str, &str)]) -> ResponseHead {
    ResponseHead {
        status,
        headers: pairs(headers),
    }
}

/// `response`, stored for `request` at `fetched_at`.
fn stored(request: &CacheRequest, response: &ResponseHead, fetched_at: SystemTime) -> Stored {
    let variant = storable(request, response).expect("a storable response");
    Stored {
        method: request.method.clone(),
        url: request.url.clone(),
        request_headers: variant.request_headers,
        head: response.clone(),
        fetched_at,
    }
}

/// A response sent with a cookie or a credential, or that sets a cookie or
/// is `private`, is the Session partition's (#795); `no-store`, `Vary: *`,
/// a POST, a partial response and an unparsable header are never stored.
#[rstest]
#[case::cookie(get(&[("cookie", "session=1")]), head(200, &[("cache-control", "max-age=60")]))]
#[case::authorization(get(&[("authorization", "Basic YTpi")]), head(200, &[("cache-control", "max-age=60")]))]
#[case::set_cookie(get(&[]), head(200, &[("cache-control", "max-age=60"), ("set-cookie", "a=1")]))]
#[case::private(get(&[]), head(200, &[("cache-control", "private, max-age=60")]))]
#[case::no_store(get(&[]), head(200, &[("cache-control", "no-store")]))]
#[case::vary_star(get(&[]), head(200, &[("cache-control", "max-age=60"), ("vary", "Accept, *")]))]
#[case::post(post(&[]), head(200, &[("cache-control", "max-age=60")]))]
#[case::partial(get(&[]), head(206, &[("cache-control", "max-age=60")]))]
#[case::unparsable(get(&[]), head(200, &[("cache-control", "max-age=60"), ("x-bad", "a\nb")]))]
fn what_the_public_partition_never_stores(
    #[case] request: CacheRequest,
    #[case] response: ResponseHead,
) {
    assert_eq!(storable(&request, &response), None);
}

/// `replay` stores what `revalidate` never would, a POST, `no-store`, an
/// error status, keyed by the same `Vary`-named headers.
/// Covers: story:6-36
#[rstest]
#[case::post(post(&[("accept", "application/json")]), head(200, &[("vary", "Accept")]))]
#[case::no_store(get(&[("accept", "application/json")]), head(200, &[("cache-control", "no-store"), ("vary", "Accept")]))]
#[case::server_error(get(&[("accept", "application/json")]), head(503, &[("vary", "Accept")]))]
fn replay_stores_any_method_status_and_freshness(
    #[case] request: CacheRequest,
    #[case] response: ResponseHead,
) {
    assert_eq!(storable(&request, &response), None);
    let variant = replayable(&request, &response).expect("replayable");
    assert_eq!(variant, super::variant(&request, &response).unwrap());
    assert_eq!(
        variant.request_headers,
        pairs(&[("accept", "application/json")])
    );
}

/// Under `replay` too, a request with a cookie or a credential, a response
/// that sets a cookie or is `private`, are the Session partition's (#795),
/// and `Vary: *` keys nothing.
#[rstest]
#[case::cookie(post(&[("cookie", "session=1")]), head(200, &[]))]
#[case::proxy_authorization(post(&[("proxy-authorization", "Basic YTpi")]), head(200, &[]))]
#[case::set_cookie(post(&[]), head(200, &[("set-cookie", "a=1")]))]
#[case::private(post(&[]), head(200, &[("cache-control", "private")]))]
#[case::vary_star(post(&[]), head(200, &[("vary", "*")]))]
fn what_replay_never_stores(#[case] request: CacheRequest, #[case] response: ResponseHead) {
    assert_eq!(replayable(&request, &response), None);
}

/// The replay key is the canonical body's: another key order is the same
/// request, another body another one.
/// Covers: story:6-36
#[test]
fn a_requests_replay_key_is_its_canonical_bodys() {
    let json = [("content-type", "application/json")];
    let with = |body: &'static [u8]| CacheRequest {
        body: Bytes::from_static(body),
        ..post(&json)
    };

    assert_eq!(
        with(br#"{"q":"shoes","page":1}"#).replay_key(),
        with(br#"{"page":1,"q":"shoes"}"#).replay_key()
    );
    assert_ne!(
        with(br#"{"q":"shoes"}"#).replay_key(),
        with(br#"{"q":"boots"}"#).replay_key()
    );
}

/// The variant keys the request headers `Vary` names, in any order and
/// case, and only them: another cookie-free header changes nothing, a
/// different `Accept-Language` another variant.
#[test]
fn the_vary_named_request_headers_key_the_variant() {
    let response = head(
        200,
        &[
            ("cache-control", "max-age=60"),
            ("vary", "Accept-Language, accept"),
        ],
    );
    let key = |headers: &[(&str, &str)]| storable(&get(headers), &response).unwrap().vary_key;

    let english = key(&[("accept", "text/html"), ("accept-language", "en")]);

    assert_eq!(
        key(&[
            ("accept-language", "en"),
            ("user-agent", "b"),
            ("accept", "text/html")
        ]),
        english
    );
    assert_ne!(
        key(&[("accept", "text/html"), ("accept-language", "fr")]),
        english
    );
    assert_ne!(key(&[("accept", "text/html")]), english);
    assert_eq!(
        storable(
            &get(&[
                ("host", "cache.test"),
                ("accept-language", "en"),
                ("user-agent", "b")
            ]),
            &response
        )
        .unwrap()
        .request_headers,
        pairs(&[("host", "cache.test"), ("accept-language", "en")])
    );
}

/// A stored variant answers only the request whose `Vary`-named headers
/// match: another `Accept-Language` is another request.
#[test]
fn a_stored_variant_answers_only_its_vary_named_headers() {
    let request = get(&[("accept-language", "en")]);
    let response = head(
        200,
        &[("cache-control", "max-age=60"), ("vary", "accept-language")],
    );
    let entry = stored(&request, &response, start());

    assert_eq!(
        freshness(&entry, &request, start()),
        Freshness::Fresh {
            age: Duration::ZERO
        }
    );
    assert_eq!(
        freshness(&entry, &get(&[("accept-language", "fr")]), start()),
        Freshness::Other
    );
}

/// A stale response is revalidated with `If-None-Match` from its `ETag`
/// and `If-Modified-Since` from its `Last-Modified`.
#[test]
fn a_stale_response_is_revalidated_with_its_validators() {
    let request = get(&[]);
    let response = head(
        200,
        &[
            ("cache-control", "max-age=60"),
            ("etag", "\"v1\""),
            ("last-modified", "Mon, 05 Jan 2026 08:00:00 GMT"),
        ],
    );
    let entry = stored(&request, &response, start());

    assert_eq!(
        freshness(&entry, &request, start() + Duration::from_secs(61)),
        Freshness::Stale {
            conditions: pairs(&[
                ("if-none-match", "\"v1\""),
                ("if-modified-since", "Mon, 05 Jan 2026 08:00:00 GMT"),
            ])
        }
    );
}

/// A 304 replaces the stored headers it also sends, in the stored order,
/// but those describing the body, and adds none; a 304 whose `ETag` names
/// another response, or a 200, does not let the stored body be served.
#[test]
fn a_304_updates_the_stored_headers_it_sends() {
    let request = get(&[]);
    let response = head(
        200,
        &[
            ("cache-control", "max-age=60"),
            ("etag", "\"v1\""),
            ("content-length", "5"),
            ("x-served-by", "a"),
        ],
    );
    let entry = stored(&request, &response, start());
    let later = start() + Duration::from_secs(120);

    let updated = not_modified(
        &entry,
        &request,
        &head(
            304,
            &[
                ("cache-control", "max-age=600"),
                ("etag", "\"v1\""),
                ("content-length", "0"),
                ("x-new", "b"),
            ],
        ),
        later,
    );

    assert_eq!(
        updated,
        Some(pairs(&[
            ("cache-control", "max-age=600"),
            ("etag", "\"v1\""),
            ("content-length", "5"),
            ("x-served-by", "a"),
        ]))
    );
    assert_eq!(
        not_modified(&entry, &request, &head(304, &[("etag", "\"v2\"")]), later),
        None
    );
    assert_eq!(
        not_modified(&entry, &request, &head(200, &[("etag", "\"v1\"")]), later),
        None
    );
}

/// A hit is served with its stored headers and an `Age` of its own.
#[test]
fn a_hit_is_served_with_its_age() {
    assert_eq!(
        served_headers(
            &pairs(&[("age", "3"), ("etag", "\"v1\"")]),
            Duration::from_millis(12_900)
        ),
        pairs(&[("etag", "\"v1\""), ("age", "12")])
    );
}

/// Freshness reads the moment the Clock gives, nothing else: a response
/// stored at the paused Clock's start is fresh until its `max-age` passes
/// on that Clock, then stale.
#[tokio::test(start_paused = true)]
async fn freshness_follows_the_clock() {
    let clock = Clock::paused(start());
    let request = get(&[]);
    let entry = stored(
        &request,
        &head(200, &[("cache-control", "max-age=60"), ("etag", "\"v1\"")]),
        clock.wall(),
    );

    tokio::time::advance(Duration::from_secs(59)).await;
    let before = freshness(&entry, &request, clock.wall());
    tokio::time::advance(Duration::from_secs(1)).await;
    let after = freshness(&entry, &request, clock.wall());

    assert_eq!(
        before,
        Freshness::Fresh {
            age: Duration::from_secs(59)
        }
    );
    assert_eq!(
        after,
        Freshness::Stale {
            conditions: pairs(&[("if-none-match", "\"v1\"")])
        }
    );
}

proptest! {
    /// Whatever its `max-age` and the moment the Clock gives, a response
    /// is fresh exactly while it is younger than its `max-age`, aged from
    /// when it was received, and the same moment always gives the same
    /// answer.
    /// Covers: story:6-37
    #[test]
    fn a_response_is_fresh_exactly_while_younger_than_its_max_age(
        max_age in 0_u64..100_000,
        elapsed in 0_u64..200_000,
        received in 0_u64..1_000_000,
    ) {
        let request = get(&[]);
        let fetched_at = start() + Duration::from_secs(received);
        let entry = stored(
            &request,
            &head(200, &[("cache-control", &format!("max-age={max_age}")), ("etag", "\"v\"")]),
            fetched_at,
        );
        let now = fetched_at + Duration::from_secs(elapsed);

        let answer = freshness(&entry, &request, now);

        prop_assert_eq!(&answer, &freshness(&entry, &request, now));
        if elapsed < max_age {
            prop_assert_eq!(answer, Freshness::Fresh { age: Duration::from_secs(elapsed) });
        } else {
            prop_assert_eq!(
                answer,
                Freshness::Stale { conditions: pairs(&[("if-none-match", "\"v\"")]) }
            );
        }
    }
}
