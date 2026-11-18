//! Pitfall H03's acceptance test on the `http` tier (spec #565, stories
//! 104 and 106; ADR 0042), on Foxhound's executable against the Proving
//! ground: a crawl request whose host's certificate fails verification
//! ends `failed` with `certificate_invalid` after one attempt, never
//! retried, nothing sent to the host, while a verified host's request, the
//! positive control, succeeds; under its domain's exception the same
//! request succeeds, and the Provenance of its Records marks the exception
//! with the key it accepted.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;
use crate::frontier::crawl_from;
use crate::provenance::{push, records};

/// The Proving ground's seed.
const SEED: u64 = 679;

/// The expired host of the `certificates` family.
const EXPIRED: &str = "expired.certificates.test";

/// A host whose certificate the Proving ground's authority makes valid,
/// once `tls.trusted_cas` trusts that authority for it.
const VERIFIED: &str = "scope.test";

/// User configuration trusting the Proving ground's authority for
/// `scope.test`, which only configuration may.
fn trusting(proving_ground: &ProvingGround) -> String {
    format!(
        "tls.trusted_cas.\"{VERIFIED}\" = ['{}']\n",
        proving_ground.description().ca.display()
    )
}

/// The settings of a crawl on `tier`, robots.txt unread, with
/// `tls.accept_invalid_certs`.
fn crawl_settings(
    proving_ground: &ProvingGround,
    tier: &str,
    accept_invalid_certs: &Value,
) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier": tier },
            "politeness": { "domain": { "delay": 0 } },
            "site_preferences": { "respect_robots_txt": false },
            "tls": { "accept_invalid_certs": accept_invalid_certs },
        }),
    )
}

/// Crawls the expired host and the verified one as the Run `run_id` on the
/// `http` tier under `accept_invalid_certs`, each handled request pushing
/// one Record naming its URL: the Run's outcomes, by URL, as their kind,
/// attempts and error codes, and its Records' Provenance marks, by URL.
fn crawled(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    accept_invalid_certs: &Value,
) -> (Vec<Value>, Vec<Value>) {
    crawled_on(
        engine,
        proving_ground,
        (run_id, "http"),
        accept_invalid_certs,
    )
}

/// [`crawled`] on `tier`.
fn crawled_on(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    (run_id, tier): (&str, &str),
    accept_invalid_certs: &Value,
) -> (Vec<Value>, Vec<Value>) {
    let seeds: Vec<Value> = [EXPIRED, VERIFIED]
        .iter()
        .map(|host| json!({ "url": https(proving_ground, host), "label": "page" }))
        .collect();
    let finished = crawl_from(
        engine,
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "settings": crawl_settings(proving_ground, tier, accept_invalid_certs),
            "datasets": [{ "name": "pages" }],
            "labels": { "page": {} },
        }),
        &mut |inside, handle| {
            let url = handle["crawl_request"]["url"].clone();
            let pushed = push(inside, handle, "pages", &json!({ "url": url }));
            assert!(pushed.get("result").is_some(), "{pushed}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let mut outcomes: Vec<Value> = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            json!({
                "url": outcome["request"]["url"],
                "kind": outcome["kind"],
                "attempts": outcome["attempts"],
                "errors": outcome["errors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|error| error["code"].clone())
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    outcomes.sort_by_key(|outcome| outcome["url"].to_string());
    let mut marks: Vec<Value> = records(engine, run_id, "pages")
        .iter()
        .map(|record| {
            json!({
                "url": record["value"]["url"],
                "certificate_exception": record["provenance"].get("certificate_exception"),
            })
        })
        .collect();
    marks.sort_by_key(|mark| mark["url"].to_string());
    (outcomes, marks)
}

/// The root of `host` on `proving_ground`, over TLS.
fn https(proving_ground: &ProvingGround, host: &str) -> String {
    format!("https://{host}:{}/", proving_ground.address().port())
}

/// The requests the Proving ground received for `host`.
fn requests_to(proving_ground: &ProvingGround, host: &str) -> usize {
    ReceivedRequests::of(&proving_ground.observations().unwrap())
        .every()
        .filter(|request| request.host == host)
        .count()
}

/// The expired host's request fails with `certificate_invalid` once, not
/// retried, nothing sent to it; the verified host's succeeds. Listed for
/// trust on first use, the expired host's request succeeds, and its Record
/// is marked in Provenance with the exception and the key it pinned; the
/// verified host's Record carries no mark.
///
/// Covers: story:4-104, story:4-106, error:certificate_invalid, setting:tls.accept_invalid_certs, setting:tls.trusted_cas, setting:fetch.tier
#[test]
fn an_invalid_certificate_fails_its_request_unretried_and_its_exception_marks_the_records() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&trusting(&proving_ground));
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let (expired, verified) = (
        https(&proving_ground, EXPIRED),
        https(&proving_ground, VERIFIED),
    );

    let (strict, strict_records) = crawled(&mut engine, &proving_ground, "strict", &json!({}));
    let unsent = requests_to(&proving_ground, EXPIRED);
    let (excepted, excepted_records) = crawled(
        &mut engine,
        &proving_ground,
        "excepted",
        &json!({ "certificates.test": null }),
    );
    assert!(engine.finished());

    assert_eq!(
        (strict, strict_records, unsent),
        (
            vec![
                json!({ "url": expired, "kind": "failed", "attempts": 1, "errors": ["certificate_invalid"] }),
                json!({ "url": verified, "kind": "succeeded", "attempts": 1, "errors": [] }),
            ],
            vec![json!({ "url": verified, "certificate_exception": null })],
            0,
        )
    );
    assert_eq!(
        excepted,
        vec![
            json!({ "url": expired, "kind": "succeeded", "attempts": 1, "errors": [] }),
            json!({ "url": verified, "kind": "succeeded", "attempts": 1, "errors": [] }),
        ]
    );
    let [expired_record, verified_record] = excepted_records.as_slice() else {
        panic!("one Record for each request: {excepted_records:?}");
    };
    let mark = &expired_record["certificate_exception"];
    assert_eq!(
        (
            &expired_record["url"],
            &mark["kind"],
            mark["spki_sha256"]
                .as_str()
                .is_some_and(|key| key.starts_with("sha256/") && key.len() == 51),
        ),
        (&json!(expired), &json!("trust_on_first_use"), true),
        "{mark}"
    );
    assert_eq!(
        verified_record,
        &json!({ "url": verified, "certificate_exception": null })
    );
}

/// Story 4-106 on the `browser` tier: the Session's egress relay terminates
/// the expired host's connections under its domain's exception, and the
/// Record its page gave is marked in Provenance with the exception and the
/// key it pinned, as on `http`; the verified host's Record, terminated for
/// its trusted authority with no exception, carries no mark.
///
/// Covers: story:4-106, setting:tls.accept_invalid_certs, setting:fetch.tier
#[test]
fn a_browser_page_fetched_under_an_exception_marks_its_records() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&trusting(&proving_ground));
    let mut engine = crate::fetch_tiers::engine(&env);
    let (expired, verified) = (
        https(&proving_ground, EXPIRED),
        https(&proving_ground, VERIFIED),
    );

    let (outcomes, records) = crawled_on(
        &mut engine,
        &proving_ground,
        ("browser-excepted", "browser"),
        &json!({ "certificates.test": null }),
    );
    assert!(engine.finished());

    assert_eq!(
        outcomes,
        vec![
            json!({ "url": expired, "kind": "succeeded", "attempts": 1, "errors": [] }),
            json!({ "url": verified, "kind": "succeeded", "attempts": 1, "errors": [] }),
        ]
    );
    let [expired_record, verified_record] = records.as_slice() else {
        panic!("one Record for each request: {records:?}");
    };
    let mark = &expired_record["certificate_exception"];
    assert_eq!(
        (
            &expired_record["url"],
            &mark["kind"],
            mark["spki_sha256"]
                .as_str()
                .is_some_and(|key| key.starts_with("sha256/") && key.len() == 51),
            verified_record,
        ),
        (
            &json!(expired),
            &json!("trust_on_first_use"),
            true,
            &json!({ "url": verified, "certificate_exception": null }),
        ),
        "{mark}"
    );
}
