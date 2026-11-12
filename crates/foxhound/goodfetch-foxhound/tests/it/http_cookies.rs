//! The cookies of a Session's own HTTP transport (RFC 6265; ADR 0021;
//! issue 1605), on Foxhound's executable against the Proving ground's
//! `fetch-edge.test` over TLS: under a forced `fetch.tier: http`, a cookie
//! a redirect sets is sent by the hop it leads to, so a site that sets a
//! cookie and redirects back to its first URL reaches the handler (ADR
//! 0118's Fetch edge line), and by the Session's later requests, and by
//! no other Session's.

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 1605;

/// The fetch edge pages' host, whose `/redirect/cookie` sets `hop=1`,
/// `Secure`, and redirects to itself until a request carries it.
const HOST: &str = "fetch-edge.test";

/// The URL of `path` on the host, over TLS.
fn over_tls(proving_ground: &ProvingGround, path: &str) -> String {
    proving_ground
        .url(HOST, path)
        .replacen("http://", "https://", 1)
}

/// Each request the host received for a path starting with `path`, in
/// order: its target and the cookies it carried.
fn received(proving_ground: &ProvingGround, path: &str) -> Vec<(String, Value)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host.starts_with(HOST) && request.target.starts_with(path))
        .map(|request| (request.target.clone(), json!(request.cookies)))
        .collect()
}

/// The Label `a` is seeded the page that sets a cookie and redirects back
/// to itself, then another page of the site; the Label `b`, whose pool is
/// its own, a third. The first reaches its handler on its second request,
/// which carries the cookie; the Session's next request carries it too;
/// the other Session's carries none.
///
/// Covers: story:5-123, story:5-124
#[test]
fn a_cookie_a_redirect_set_is_sent_by_its_sessions_later_hops_and_by_no_other_sessions() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!("tls.trusted_cas.\"{HOST}\" = ['{ca}']\n"));
    let mut engine = engine(&env);
    let mut handed = Vec::new();

    let finished = crawl_declaring(
        &mut engine,
        "http-cookies",
        &[
            json!({ "url": over_tls(&proving_ground, "/redirect/cookie"), "label": "a" }),
            json!({ "url": over_tls(&proving_ground, "/target"), "label": "a" }),
            json!({ "url": over_tls(&proving_ground, "/redirect/hops/0"), "label": "b" }),
        ],
        &json!({
            "a": {},
            "b": { "settings": { "session": { "source": null } } },
        }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "http" },
                "session": { "pool": { "max_size": 1 } },
            }),
        ),
        &mut |_: &mut Inside<'_>, handle: &Value| {
            handed.push(handle["crawl_request"]["url"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let hop = json!([["hop", "1"]]);
    let none = json!([]);
    assert_eq!(
        (
            &finished["result"]["requests"]["succeeded"],
            handed.len(),
            received(&proving_ground, "/redirect/cookie"),
            received(&proving_ground, "/target"),
            received(&proving_ground, "/redirect/hops/0"),
        ),
        (
            &json!(3),
            3,
            vec![
                ("/redirect/cookie".to_owned(), none.clone()),
                ("/redirect/cookie".to_owned(), hop.clone()),
            ],
            vec![("/target".to_owned(), hop)],
            vec![("/redirect/hops/0".to_owned(), none)],
        )
    );
}
