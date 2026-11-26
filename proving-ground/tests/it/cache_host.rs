//! `cache.test`, the Response cache's host (ADR 0046): its pages say how a
//! response may be stored and which answer of their path each is, so a
//! case tells a response served from a cache from one the site sent.

use crate::proving_ground::ProvingGround;
use crate::raw_clients;

const SEED: u64 = 793;

const HOST: &str = "cache.test";

#[test]
fn a_fresh_page_is_kept_an_hour_and_names_its_answer() {
    let proving_ground = ProvingGround::start(SEED);

    let first = raw_clients::get(&proving_ground, HOST, "/fresh/one", &[]);
    let second = raw_clients::get(&proving_ground, HOST, "/fresh/one", &[]);

    assert_eq!(
        (
            first.status(),
            first.header("cache-control"),
            String::from_utf8_lossy(first.body()).contains("Answer 1 of /fresh/one."),
            String::from_utf8_lossy(second.body()).contains("Answer 2 of /fresh/one."),
        ),
        (Some(200), Some("max-age=3600".to_owned()), true, true)
    );
}

#[test]
fn a_large_page_is_kept_an_hour_and_weighs_over_a_mebibyte() {
    let proving_ground = ProvingGround::start(SEED);

    let large = raw_clients::get(&proving_ground, HOST, "/large/one", &[]);

    assert_eq!(
        (
            large.status(),
            large.header("cache-control"),
            String::from_utf8_lossy(large.body()).contains("Answer 1 of /large/one."),
            large.body().len() > 1 << 20,
        ),
        (Some(200), Some("max-age=3600".to_owned()), true, true)
    );
}

#[test]
fn a_stale_page_answers_either_of_its_validators_with_a_304() {
    let proving_ground = ProvingGround::start(SEED);

    let page = raw_clients::get(&proving_ground, HOST, "/stale/one", &[]);
    let etag = page.header("etag").unwrap_or_default();
    let modified = page.header("last-modified").unwrap_or_default();
    let by_etag = raw_clients::get(
        &proving_ground,
        HOST,
        "/stale/one",
        &[("If-None-Match", &etag)],
    );
    let by_date = raw_clients::get(
        &proving_ground,
        HOST,
        "/stale/one",
        &[("If-Modified-Since", &modified)],
    );
    let other = raw_clients::get(
        &proving_ground,
        HOST,
        "/stale/one",
        &[("If-None-Match", "\"another\"")],
    );

    assert_eq!(
        (
            page.status(),
            page.header("cache-control"),
            etag.as_str(),
            modified.as_str(),
            (
                by_etag.status(),
                by_etag.header("etag"),
                by_etag.body().is_empty()
            ),
            by_date.status(),
            other.status(),
            String::from_utf8_lossy(other.body()).contains("Answer 4 of /stale/one."),
        ),
        (
            Some(200),
            Some("max-age=0".to_owned()),
            "\"one-v1\"",
            "Thu, 01 Jan 2026 00:00:00 GMT",
            (Some(304), Some("\"one-v1\"".to_owned()), true),
            Some(304),
            Some(200),
            true,
        )
    );
}

#[test]
fn a_page_varying_on_cookie_names_the_cookie_it_was_sent() {
    let proving_ground = ProvingGround::start(SEED);

    let without = raw_clients::get(&proving_ground, HOST, "/vary/one", &[]);
    let with = raw_clients::get(
        &proving_ground,
        HOST,
        "/vary/one",
        &[("Cookie", "visitor=one")],
    );
    let setting = raw_clients::get(&proving_ground, HOST, "/cookie/one", &[]);

    assert_eq!(
        (
            (without.header("vary"), without.header("cache-control")),
            String::from_utf8_lossy(without.body()).contains("Cookie: none."),
            String::from_utf8_lossy(with.body()).contains("Cookie: visitor=one."),
            (
                setting.status(),
                setting.header("location"),
                setting.header("set-cookie"),
                setting.header("cache-control"),
            ),
        ),
        (
            (Some("Cookie".to_owned()), Some("max-age=3600".to_owned())),
            true,
            true,
            (
                Some(302),
                Some("/vary/one".to_owned()),
                Some("visitor=one; Path=/".to_owned()),
                Some("no-store".to_owned()),
            ),
        )
    );
}

#[test]
fn an_api_path_answers_any_method_never_to_be_stored() {
    let proving_ground = ProvingGround::start(SEED);

    let posted = raw_clients::post(&proving_ground, HOST, "/api/one", br#"{"q":"shoes"}"#);
    let got = raw_clients::get(&proving_ground, HOST, "/api/one", &[]);

    let said = |arrived: &raw_clients::Arrived, text: &str| {
        String::from_utf8_lossy(arrived.body()).contains(text)
    };
    assert_eq!(
        (
            posted.status(),
            posted.header("cache-control"),
            said(&posted, "Answer 1 of /api/one."),
            said(&posted, "POST with 13 bytes."),
            said(&got, "Answer 2 of /api/one."),
            said(&got, "GET with 0 bytes."),
        ),
        (
            Some(200),
            Some("no-store".to_owned()),
            true,
            true,
            true,
            true
        )
    );
}

#[test]
fn the_cache_host_has_no_robots_txt() {
    let proving_ground = ProvingGround::start(SEED);

    let robots = raw_clients::get(&proving_ground, HOST, "/robots.txt", &[]);

    assert_eq!(robots.status(), Some(404));
}
