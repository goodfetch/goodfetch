//! A crawl that sets a setting whose feature the engine does not build yet
//! (spec #213; ADR 0083), on Foxhound's executable over stdio: its start
//! fails with `capability_unavailable` naming the key, before the Run
//! begins, where the value would otherwise be ignored.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;

/// The answer to the start of a crawl of one seed, in a home whose user
/// configuration sets `configuration`.
fn started(configuration: &str, settings: &Value) -> Value {
    let env = TestEnv::new();
    env.user_configuration(configuration);
    let mut engine = attached(&env);

    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "unbuilt",
            // A port nothing listens on: a crawl that starts reaches no site.
            "seeds": [{ "url": "http://127.0.0.1:9/", "label": "page" }],
            "labels": { "page": {} },
            "settings": settings,
        }),
        &mut |_| None,
    );
    assert!(engine.finished());
    started
}

/// A crawl whose settings hold a `paid_fetch.*` key or the sample crawl's
/// own, which no code reads yet, fails its start with
/// `capability_unavailable` naming the key and the layer that set it.
///
/// Covers: error:capability_unavailable, operation:crawl.start
#[rstest]
/// Covers: setting:paid_fetch.domains
#[case::paid_fetch_domains(
    "paid_fetch.domains = [\"shop.example\"]\n",
    json!({}),
    "paid_fetch.domains",
    "user_configuration"
)]
/// Covers: setting:paid_fetch.max_price_per_page
#[case::paid_fetch_max_price_per_page(
    "paid_fetch.max_price_per_page = \"0.05 USD\"\n",
    json!({}),
    "paid_fetch.max_price_per_page",
    "user_configuration"
)]
/// Covers: setting:paid_fetch.exchange_rates
#[case::paid_fetch_exchange_rates(
    "paid_fetch.exchange_rates = { \"EUR/USD\" = \"1.1\" }\n",
    json!({}),
    "paid_fetch.exchange_rates",
    "user_configuration"
)]
/// Covers: setting:paid_fetch.chain_rpc_endpoints
#[case::paid_fetch_chain_rpc_endpoints(
    "paid_fetch.chain_rpc_endpoints = { base = \"https://rpc.example\" }\n",
    json!({}),
    "paid_fetch.chain_rpc_endpoints",
    "user_configuration"
)]
/// Covers: setting:crawl.sample_pages
#[case::the_sample_crawls_pages(
    "",
    json!({ "crawl": { "sample_pages": 2 } }),
    "crawl.sample_pages",
    "run"
)]
fn a_crawl_setting_what_the_engine_does_not_build_yet_fails_its_start(
    #[case] configuration: &str,
    #[case] settings: Value,
    #[case] key: &str,
    #[case] layer: &str,
) {
    let refused = started(configuration, &settings);

    let details = &refused["error"]["data"];
    assert_eq!(
        (&details["code"], &details["key"], &details["layer"]),
        (&json!("capability_unavailable"), &json!(key), &json!(layer))
    );
}
