//! URL identity conformance cases, the fragment lines (ADR 0120; pitfall
//! U05), on Foxhound's executable against the Proving ground's
//! hash-routed app on `tiers.test` and Chrome for Testing: on a Label with
//! `crawl.keep_url_fragment` on, `#/a` and `#/b` are two requests, each
//! handled in the browser on its own view, forced or under `fetch.tier:
//! auto`, while the server receives the one document each time; with it
//! off they are one; and forcing a document tier on that Label fails the
//! start with `setting_not_applicable`, the Proving ground receiving
//! nothing.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::crawl_declaring;
use crate::robots_txt::arrivals;

/// The Proving ground's seed.
const SEED: u64 = 939;

/// The host of the hash-routed app.
const HOST: &str = "tiers.test";

/// The app's path; its fragment names its route.
const ROUTES: &str = "/routes";

/// One request as its handler saw it: its URL, its tier and the view its
/// page showed.
type Handled = (String, Value, Value);

/// Crawls the app's routes `#/a`, `#/b` and `#/a` again, as seeds of the
/// Label `app` that sets `label`, under the Run's `fetch.tier: auto`;
/// answers what each handler saw, the requests that ended and what the
/// Proving ground received.
fn crawl_routes(label: &Value, run_id: &str) -> (Vec<Handled>, usize, Vec<String>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let route = |route: &str| format!("{}#/{route}", proving_ground.url(HOST, ROUTES));

    let mut seen = Vec::new();
    crawl_declaring(
        &mut engine,
        run_id,
        &[
            json!({ "url": route("a"), "label": "app" }),
            json!({ "url": route("b"), "label": "app" }),
            json!({ "url": route("a"), "label": "app" }),
        ],
        &json!({ "app": { "settings": label } }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut |inside, handle| {
            let view = inside.call(
                "page.evaluate",
                &json!({
                    "page_id": handle["page_id"],
                    "function": "() => document.querySelector('.view')?.textContent ?? null",
                }),
            );
            seen.push((
                handle["crawl_request"]["url"].as_str().unwrap().to_owned(),
                handle["tier"].clone(),
                view["result"]["value"].clone(),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    assert!(engine.finish());
    let ended = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {outcomes}"))
        .iter()
        .filter(|outcome| outcome["kind"] == "succeeded")
        .count();
    let received = arrivals(&proving_ground, HOST)
        .into_iter()
        .map(|(target, _)| target)
        .filter(|target| target.starts_with(ROUTES))
        .collect();
    (seen, ended, received)
}

/// With `crawl.keep_url_fragment` on the Label, `#/a` and `#/b` are two
/// requests and `#/a` again a duplicate; each is handled in the browser,
/// forced or chosen under `auto` (never sampled on `http`), on its own
/// view, and the site receives `/routes` once per request, the fragment
/// never sent (the browser's own `/favicon.ico` aside).
///
/// Covers: story:6-221, setting:crawl.keep_url_fragment, operation:page.evaluate
#[rstest]
#[case::forced(json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "browser" } }))]
#[case::under_auto(json!({ "crawl": { "keep_url_fragment": true } }))]
fn a_label_keeping_fragments_makes_each_route_a_request_the_browser_handles(#[case] label: Value) {
    let (seen, ended, received) = crawl_routes(&label, "fragments-kept");

    let views: Vec<(bool, Value, Value)> = seen
        .into_iter()
        .map(|(url, tier, view)| (url.ends_with("#/a"), tier, view))
        .collect();
    assert_eq!(
        (views, ended, received),
        (
            vec![
                (true, json!("browser"), json!("view a")),
                (false, json!("browser"), json!("view b")),
            ],
            2,
            vec![ROUTES.to_owned(), ROUTES.to_owned()],
        )
    );
}

/// With `crawl.keep_url_fragment` off, the default, the fragment is
/// dropped from the key: the three routes are one request, handled once.
///
/// Covers: story:6-221, setting:crawl.keep_url_fragment
#[test]
fn a_label_dropping_fragments_makes_every_route_one_request() {
    let (seen, ended, _) = crawl_routes(
        &json!({ "fetch": { "tier": "browser" } }),
        "fragments-dropped",
    );

    assert_eq!((seen.len(), ended), (1, 1));
}

/// Forcing a document tier on a Label that keeps fragments, its own or
/// the Run's, fails the crawl's start with `setting_not_applicable` naming
/// `crawl.keep_url_fragment`, before the Run begins: no handler runs and
/// the Proving ground receives nothing.
///
/// Covers: story:6-221, story:6-262, setting:crawl.keep_url_fragment, error:setting_not_applicable
#[rstest]
#[case::http(json!({}), json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "http" } }))]
#[case::browser_fetch(json!({}), json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "browser_fetch" } }))]
#[case::discovered_api(json!({}), json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "discovered_api" } }))]
#[case::kept_by_the_run(json!({ "crawl": { "keep_url_fragment": true } }), json!({ "fetch": { "tier": "http" } }))]
fn forcing_a_document_tier_on_a_label_keeping_fragments_fails_the_start(
    #[case] run: Value,
    #[case] label: Value,
) {
    let refusal = refused_start(&run, &label);

    assert_eq!(
        refusal,
        (
            json!(["setting_not_applicable", "crawl.keep_url_fragment"]),
            Vec::<String>::new()
        )
    );
}

/// Starts a crawl of the app's route `#/a` whose Run sets `run` and whose
/// Label `app` sets `label`, under the Run's own settings beside; answers
/// the start's error code and key, and what the Proving ground received.
pub(crate) fn refused_start(run: &Value, label: &Value) -> (Value, Vec<String>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let url = format!("{}#/a", proving_ground.url(HOST, ROUTES));

    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "never-served",
            "seeds": [{ "url": url, "label": "app" }],
            "labels": { "app": { "settings": label } },
            "settings": settings(&proving_ground, run),
        }),
        &mut |_| None,
    );
    assert!(engine.finish());
    let received = arrivals(&proving_ground, HOST)
        .into_iter()
        .map(|(target, _)| target)
        .collect();
    (
        json!([
            started["error"]["data"]["code"],
            started["error"]["data"]["key"]
        ]),
        received,
    )
}
