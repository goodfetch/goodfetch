//! Browser-fetch (spec #579; ADR 0020; pitfall H20), on Foxhound's
//! executable against the Proving ground and Chrome for Testing: a Session
//! that may move to the browser, any Session under `fetch.tier: auto`,
//! sends its HTTP tier's requests from inside its pool Session's browser
//! context, so the cookies a page set in the browser are sent by the HTTP
//! tier's next request in the same Session and by no other Session (the
//! tier parity of cookies, Foxhound's half), and the site sees one TLS
//! fingerprint across both tiers.

use std::time::Duration;

use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 769;

/// The host of the `access` family's corpus pages.
const HOST: &str = "access.test";

/// An engine whose pages launch Chrome for Testing from the test's cache.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    Attached::start_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl that reaches every host of `proving_ground` on
/// `tier`, one Session in each pool at a time.
fn settings(proving_ground: &ProvingGround, tier: &str) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "network": { "resolve": resolve },
        "fetch": { "tier": tier },
        "session": { "pool": { "max_size": 1 } },
    })
}

/// A seed of `label` at `path` of the corpus.
fn seed(proving_ground: &ProvingGround, path: &str, label: &str) -> Value {
    json!({ "url": proving_ground.url(HOST, path), "label": label })
}

/// Each request the site received for a corpus article, in order: its
/// target and the cookies it carried.
fn articles(proving_ground: &ProvingGround) -> Vec<(String, Value)> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| {
            request["target"]
                .as_str()
                .is_some_and(|target| target.starts_with("/corpus/article"))
        })
        .map(|request| {
            (
                request["target"].as_str().unwrap().to_owned(),
                request["cookies"].clone(),
            )
        })
        .collect()
}

/// The handler: on a browser page of the Label `a` it sets the cookie
/// `who=a` once; it answers no Record.
fn handler(set: &mut bool) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        if handle["tier"] == "browser" && handle["crawl_request"]["label"] == "a" && !*set {
            *set = true;
            let _ = inside.call(
                "page.evaluate",
                &json!({
                    "page_id": handle["page_id"],
                    "function": "() => { document.cookie = 'who=a; path=/'; }",
                }),
            );
        }
        json!({ "result": { "records": [] } })
    }
}

/// Under `auto`, each of a Label's first requests is a sample: its run on
/// `http`, then on `browser`. The cookie the first sample's browser page
/// set is sent by the `http` run of the Label's next request, in the same
/// pool Session, and never by a request of the Label `b`, whose pool is
/// its own.
///
/// Covers: story:5-123, story:5-121
#[test]
fn a_cookie_the_browser_sample_set_is_sent_by_the_http_tiers_next_request_in_its_session_only() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut set = false;
    let finished = crawl_declaring(
        &mut engine,
        "cookie-parity",
        &[
            seed(&proving_ground, "/corpus/article?n=1", "a"),
            seed(&proving_ground, "/corpus/article?n=2", "a"),
            seed(&proving_ground, "/corpus/article?n=3", "b"),
        ],
        &json!({
            "a": {},
            "b": { "settings": { "session": { "source": null } } },
        }),
        &settings(&proving_ground, "auto"),
        &mut handler(&mut set),
    );
    assert!(engine.finish());

    let who = json!([["who", "a"]]);
    let none = json!([]);
    assert_eq!(
        (articles(&proving_ground), &finished["result"]["lifecycle"]),
        (
            vec![
                // The first sample: on `http`, then on `browser`.
                ("/corpus/article?n=1".to_owned(), none.clone()),
                ("/corpus/article?n=1".to_owned(), none.clone()),
                // The next, after the browser page set its cookie.
                ("/corpus/article?n=2".to_owned(), who.clone()),
                ("/corpus/article?n=2".to_owned(), who),
                // Another Session's.
                ("/corpus/article?n=3".to_owned(), none.clone()),
                ("/corpus/article?n=3".to_owned(), none),
            ],
            &json!("finished")
        )
    );
}
