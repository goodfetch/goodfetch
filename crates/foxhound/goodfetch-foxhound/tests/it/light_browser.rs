//! The light-browser tier (spec #579; ADRs 0020 and 0093), on Foxhound's
//! executable against the Proving ground, Chrome for Testing and
//! Lightpanda, laid out in the test's Browser cache from the tests' copy
//! (`ci/lightpanda-fetch.sh`): where `fetch.light_browser` allows it, a
//! Label whose script-rendered pages match Chrome's on Lightpanda is
//! learned onto `light_browser`, whose requests Lightpanda sends from its
//! own process through the engine's bridge; a Label whose pages need
//! pixels stays on the browser; and a call the tier cannot serve
//! escalates under `auto` and fails with `browser_required` under a
//! forced tier.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache_with_lightpanda};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::fetch_tiers::{engine_on, settings, tiers};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 771;

const HOST: &str = "tiers.test";

/// Seeds on [`HOST`], each path with its Label.
fn seeds(proving_ground: &ProvingGround, seeds: &[(&str, &str)]) -> Vec<Value> {
    seeds
        .iter()
        .map(|(path, label)| json!({ "url": proving_ground.url(HOST, path), "label": label }))
        .collect()
}

/// Each `fetch_tier` Provenance entry of `run_id` of a Label in `labels`,
/// as `<label> <tier> <chosen>`, with the call of an escalation.
fn chosen(env: &TestEnv, run_id: &str, labels: &[&str]) -> Vec<String> {
    tiers(env, run_id)
        .iter()
        .filter(|entry| labels.iter().any(|label| entry[0] == *label))
        .map(|entry| {
            let line = format!(
                "{} {} {}",
                entry[0].as_str().unwrap(),
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            );
            match entry[4].as_str() {
                Some(call) if entry[2] == "escalation" => format!("{line} {call}"),
                _ => line,
            }
        })
        .collect()
}

/// The page's raw HTML, as served or as the DOM stands in the browser.
fn raw_html(inside: &mut Inside<'_>, handle: &Value) -> String {
    let reply = inside.call(
        "page.formats",
        &json!({ "page_id": handle["page_id"], "formats": { "html": { "kind": "raw" } } }),
    );
    reply["result"]["html"]
        .as_str()
        .unwrap_or_else(|| panic!("no HTML: {reply}"))
        .to_owned()
}

/// The texts of the elements `open` starts in `html`, up to their closing
/// tag.
fn texts(html: &str, open: &str) -> Vec<String> {
    html.split(open)
        .skip(1)
        .filter_map(|rest| rest.split('<').next())
        .map(str::to_owned)
        .collect()
}

/// The path of `url`.
fn path_of(url: &str) -> String {
    format!("/{}", url.splitn(4, '/').nth(3).unwrap_or_default())
}

/// The engine, its Browser cache holding Chrome for Testing and Lightpanda.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache_with_lightpanda(env)
        .expect("Playwright's Chrome for Testing and the tests' Lightpanda are installed");
    engine_on(env, &cache)
}

/// Under `auto`, a Run that allows the light-browser tier samples each
/// Label on `http`, `light_browser` and `browser`: a listing its script
/// fills differs over `http` and matches on Lightpanda, so its Label moves
/// to `light_browser`, where its next request is served from the
/// document Lightpanda's run of its script left, Lightpanda sending it
/// from its own process; a page whose text only painted pixels give
/// differs on Lightpanda too, so its Label stays on the browser.
///
/// Covers: story:5-120, story:5-114, setting:fetch.light_browser, operation:page.formats
#[test]
fn a_label_whose_sample_matches_chromes_runs_on_lightpanda_and_one_that_differs_stays_on_the_browser()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled: Vec<(String, Value, Value)> = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "light",
        &seeds(
            &proving_ground,
            &[
                ("/blog/1", "listing"),
                ("/blog/2", "listing"),
                ("/blog/3", "listing"),
                ("/blog/4", "listing"),
                ("/painted/1", "painted"),
                ("/painted/2", "painted"),
                ("/painted/3", "painted"),
                ("/painted/4", "painted"),
            ],
        ),
        &json!({ "listing": {}, "painted": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "light_browser": true } }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            let data = if handle["crawl_request"]["label"] == "listing" {
                json!(texts(&html, "<li class=\"item\">"))
            } else {
                json!(texts(&html, "<p class=\"painted\">"))
            };
            handled.push((
                path_of(handle["crawl_request"]["url"].as_str().unwrap()),
                handle["tier"].clone(),
                data.clone(),
            ));
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "read": data } }] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());

    let sampled = |label: &str| {
        let mut lines = Vec::new();
        for _ in 0..3 {
            for tier in ["http", "light_browser", "browser"] {
                lines.push(format!("{label} {tier} sample"));
            }
        }
        lines
    };
    let lightpanda_sent = observations.to_string().contains("Lightpanda/");
    let fourth: Vec<(String, Value)> = handled
        .iter()
        .filter(|(path, _, _)| path.ends_with("/4"))
        .map(|(path, tier, _)| (path.clone(), tier.clone()))
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            chosen(&env, "light", &["listing"]),
            chosen(&env, "light", &["painted"]),
            fourth,
            handled
                .iter()
                .filter(|(path, tier, _)| path == "/painted/1" && tier != "http")
                .map(|(_, tier, data)| (tier.clone(), data.clone()))
                .collect::<Vec<_>>(),
            lightpanda_sent,
        ),
        (
            &json!("finished"),
            [
                sampled("listing"),
                vec!["listing light_browser compared".to_owned()]
            ]
            .concat(),
            [
                sampled("painted"),
                vec!["painted browser compared".to_owned()]
            ]
            .concat(),
            vec![
                ("/blog/4".to_owned(), json!("light_browser")),
                ("/painted/4".to_owned(), json!("browser")),
            ],
            vec![
                (json!("light_browser"), json!(["alpha 0"])),
                (json!("browser"), json!(["alpha 255"])),
            ],
            true,
        ),
        "{observations}"
    );
}

/// A call the light-browser tier cannot serve exactly, `page.evaluate`, a
/// script of the handler's, escalates its request to the browser under
/// `auto`, where it is served; under a forced `light_browser`, a gesture
/// fails with `browser_required` naming `fetch.tier` and the call, and
/// nothing escalates (ADR 0093).
///
/// Covers: story:5-120, story:5-115, error:browser_required, operation:page.evaluate, operation:locator.click
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one test drives two crawls, under `auto` and forced, whose answers it compares together"
)]
fn a_call_the_light_browser_cannot_serve_escalates_under_auto_and_fails_when_forced() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut evaluated = Vec::new();
    let learned = crawl_declaring(
        &mut engine,
        "light-auto",
        &seeds(
            &proving_ground,
            &[
                ("/blog/1", "listing"),
                ("/blog/2", "listing"),
                ("/blog/3", "listing"),
                ("/blog/4", "listing"),
            ],
        ),
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "auto", "light_browser": true } }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            if handle["crawl_request"]["url"]
                .as_str()
                .unwrap()
                .ends_with("/blog/4")
            {
                let reply = inside.call(
                    "page.evaluate",
                    &json!({ "page_id": handle["page_id"], "function": "() => document.querySelectorAll('li.item').length" }),
                );
                evaluated.push((
                    handle["tier"].clone(),
                    reply.get("error").map_or_else(
                        || reply["result"].clone(),
                        |error| error["data"]["code"].clone(),
                    ),
                ));
                if reply.get("error").is_some() {
                    return json!({ "error": reply["error"] });
                }
            }
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "items": texts(&html, "<li class=\"item\">") } }] } })
        },
    );

    let mut clicked = Vec::new();
    let forced = crawl_declaring(
        &mut engine,
        "light-forced",
        &seeds(&proving_ground, &[("/blog/5", "listing")]),
        &json!({ "listing": {} }),
        &settings(
            &proving_ground,
            &json!({ "fetch": { "tier": "light_browser", "light_browser": true } }),
        ),
        &mut |inside, handle| {
            let html = raw_html(inside, handle);
            let reply = inside.call(
                "locator.click",
                &json!({ "locator": { "page_id": handle["page_id"], "selector": "li.item" } }),
            );
            clicked.push((
                handle["tier"].clone(),
                texts(&html, "<li class=\"item\">").len(),
                reply["error"]["data"]["code"].clone(),
                reply["error"]["data"]["call"].clone(),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());

    let learned_lines = chosen(&env, "light-auto", &["listing"]);
    assert_eq!(
        (
            &learned["result"]["lifecycle"],
            learned_lines[9..].to_vec(),
            evaluated.len(),
            evaluated.last().cloned(),
            &forced["result"]["lifecycle"],
            chosen(&env, "light-forced", &["listing"]),
            clicked,
        ),
        (
            &json!("finished"),
            vec![
                "listing light_browser compared".to_owned(),
                "listing browser escalation page.evaluate".to_owned(),
            ],
            2,
            Some((json!("browser"), json!({ "value": 3 }))),
            &json!("finished"),
            vec!["listing light_browser forced".to_owned()],
            vec![(
                json!("light_browser"),
                3,
                json!("browser_required"),
                json!("locator.click")
            )],
        )
    );
}
