//! The limits on what the HTTP tiers read (ADR 0067, SC-05, pitfall H14),
//! on Foxhound's executable against the Proving ground: under a forced
//! `http` tier, each decompression bomb (stacked codings included), the
//! bodies past 64 MiB (a gzip one within the ratio included) and the
//! endless bodies of the hostile-input limits table end their crawl request
//! `failed` with `response_limit_exceeded` naming its setting, while the
//! page compressed within the ratio, the positive control, succeeds; a
//! handler's `page.request` on that page to a bomb or an endless body
//! answers the same refusal. The table's JSON past its depth is met as the
//! crawl reads its `TDMRep` file, the JSON it parses before it sends a request
//! under a Purpose, and `network.max_json_depth` decides.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::{
    ExpectedError, HostileTables, LimitSetting, ProvingGround, ReceivedRequests, ServedBy, TestEnv,
};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::crawl_declaring;

/// The Proving ground's seed.
const SEED: u64 = 644;

/// Each page of the committed limits table that a Proving ground page serves
/// and whose refusal names a setting, with that setting.
fn refused() -> Vec<(String, LimitSetting)> {
    let tables = HostileTables::committed().unwrap();
    let refused: Vec<_> = tables
        .limits
        .cases
        .into_iter()
        .filter_map(|case| match (case.served_by, case.expected) {
            (ServedBy::Page(task), ExpectedError::ResponseLimitExceeded { setting }) => {
                Some((task, setting))
            }
            _ => None,
        })
        .collect();
    for setting in [
        LimitSetting::MaxBodySize,
        LimitSetting::MaxDecompressionRatio,
        LimitSetting::MaxJsonDepth,
    ] {
        assert!(
            refused.iter().any(|(_, served)| *served == setting),
            "the table serves no case naming {setting:?}"
        );
    }
    refused
}

/// The page compressed within the ratio.
const SAFE: &str = "bombs/6";

/// Each request's end by its URL: its kind, its error's code and the
/// setting its details name.
fn ended(engine: &mut Attached, run_id: &str) -> Vec<(String, Value, Value, Value)> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let mut ended: Vec<_> = answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                outcome["kind"].clone(),
                outcome["error"]["code"].clone(),
                outcome["error"]["details"]["setting"].clone(),
            )
        })
        .collect();
    ended.sort_by(|left, right| left.0.cmp(&right.0));
    ended
}

/// Every page of the limits table, crawled on a forced `http` tier under a
/// Label that reads `text/plain` (the type of the `body-limits` pages) and a
/// Purpose, which reads each host's `TDMRep` file first, ends `failed` with
/// `response_limit_exceeded` naming its setting, and the page within the
/// limits succeeds; the handler, which runs on that page alone, gets the same
/// refusal from `page.request` to a bomb, to an endless body and to a host
/// whose `TDMRep` file nests past the depth.
///
/// Covers: story:4-107, row:SC-05, error:response_limit_exceeded, setting:fetch.tier, setting:tls.trusted_cas, setting:crawl.content_types, setting:site_preferences.purpose, setting:network.max_json_depth, operation:page.request
#[test]
fn every_body_past_a_limit_fails_its_request_and_its_page_request_on_the_http_tier() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    // The `body-limits` pages, and the Brotli and zstd bombs, start over
    // `https://`.
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"body-limits.test\" = ['{ca}']\ntls.trusted_cas.\"bombs.test\" = ['{ca}']\n"
    ));
    let mut engine = engine(&env);
    let url = |task: &str| proving_ground.start_url(task).unwrap();
    let refused = refused();
    let seeds: Vec<Value> = refused
        .iter()
        .map(|(task, _)| task.as_str())
        .chain([SAFE])
        .map(|task| json!({ "url": url(task), "label": "page" }))
        .collect();
    let requested = [url("bombs/5"), url("body-limits/2"), url("body-limits/4")];

    let mut answers = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "body-limits",
        &seeds,
        &json!({ "page": { "settings": { "crawl": { "content_types": ["text/html", "text/plain"] } } } }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "http" },
                "crawl": { "retry": { "max": 0 } },
                "politeness": { "domain": { "delay": 0 } },
                "site_preferences": {
                    "respect_robots_txt": false,
                    "respect_use_signals": true,
                    "purpose": "ai-input",
                },
            }),
        ),
        &mut |inside, handle| {
            for target in &requested {
                let answer = inside.call(
                    "page.request",
                    &json!({ "page_id": handle["page_id"], "url": target }),
                );
                answers.push((
                    answer["error"]["data"]["code"].clone(),
                    answer["error"]["data"]["setting"].clone(),
                ));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "body-limits");
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let mut expected: Vec<(String, Value, Value, Value)> = refused
        .iter()
        .map(|(task, setting)| {
            (
                url(task),
                json!("failed"),
                json!("response_limit_exceeded"),
                json!(setting),
            )
        })
        .chain([(url(SAFE), json!("succeeded"), Value::Null, Value::Null)])
        .collect();
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(ended, expected);
    assert_eq!(
        answers,
        [
            (
                json!("response_limit_exceeded"),
                json!("network.max_decompression_ratio")
            ),
            (
                json!("response_limit_exceeded"),
                json!("network.max_body_size")
            ),
            (
                json!("response_limit_exceeded"),
                json!("network.max_json_depth")
            ),
        ]
    );
}

/// A handler's navigation on the browser tier to a host whose `TDMRep` file
/// nests past the depth raises `response_limit_exceeded` naming the setting,
/// through the page's navigation gate, and never reaches the host's page.
///
/// Covers: row:SC-05, error:response_limit_exceeded, setting:network.max_json_depth, setting:site_preferences.purpose, operation:page.goto
#[test]
fn a_browsers_navigation_to_a_host_whose_tdmrep_file_is_too_deep_is_refused() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"body-limits.test\" = ['{ca}']\n"
    ));
    let mut engine = engine(&env);
    let target = proving_ground.start_url("body-limits/4").unwrap();
    let mut answer = Value::Null;

    let finished = crawl_declaring(
        &mut engine,
        "browser-depth",
        &[json!({ "url": proving_ground.start_url(SAFE).unwrap(), "label": "page" })],
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "browser" },
                "crawl": { "retry": { "max": 0 } },
                "politeness": { "domain": { "delay": 0 } },
                "site_preferences": {
                    "respect_robots_txt": false,
                    "respect_use_signals": true,
                    "purpose": "ai-input",
                },
            }),
        ),
        &mut |inside, handle| {
            answer = inside.call(
                "page.goto",
                &json!({ "page_id": handle["page_id"], "url": target }),
            );
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(
        (
            &answer["error"]["data"]["code"],
            &answer["error"]["data"]["setting"],
        ),
        (
            &json!("response_limit_exceeded"),
            &json!("network.max_json_depth"),
        ),
        "{answer}"
    );
    let observations = proving_ground.observations().unwrap();
    let asked: Vec<String> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.host.starts_with("4.body-limits.test"))
        .map(|request| request.target.clone())
        .filter(|target| target != "/robots.txt")
        .collect();
    assert_eq!(asked, ["/.well-known/tdmrep.json"]);
}

/// `network.max_json_depth` decides what a crawl's read of JSON refuses: a Run
/// that raises it past the file's nesting sends the request, the file 128
/// deep passes the default, which the parser alone refused, and a Run that
/// lowers it below refuses that file.
///
/// Covers: setting:network.max_json_depth, setting:site_preferences.purpose
#[rstest::rstest]
#[case::raised_past_the_file("body-limits/4", Some(129), false)]
#[case::at_the_default("body-limits/5", None, false)]
#[case::lowered_below_the_file("body-limits/5", Some(127), true)]
fn the_runs_json_depth_decides_what_its_tdmrep_read_refuses(
    #[case] task: &str,
    #[case] depth: Option<u64>,
    #[case] is_refused: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"body-limits.test\" = ['{ca}']\n"
    ));
    let mut engine = engine(&env);
    let url = proving_ground.start_url(task).unwrap();
    let mut settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "crawl": { "retry": { "max": 0 } },
            "politeness": { "domain": { "delay": 0 } },
            "site_preferences": {
                "respect_robots_txt": false,
                "respect_use_signals": true,
                "purpose": "ai-input",
            },
        }),
    );
    if let Some(depth) = depth {
        settings["network"]["max_json_depth"] = json!(depth);
    }

    let finished = crawl_declaring(
        &mut engine,
        "json-depth",
        &[json!({ "url": url, "label": "page" })],
        &json!({ "page": { "settings": { "crawl": { "content_types": ["application/json"] } } } }),
        &settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    let ended = ended(&mut engine, "json-depth");
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let expected = if is_refused {
        (
            url,
            json!("failed"),
            json!("response_limit_exceeded"),
            json!("network.max_json_depth"),
        )
    } else {
        (url, json!("succeeded"), Value::Null, Value::Null)
    };
    assert_eq!(ended, [expected]);
}

/// A download is not counted against `network.max_body_size` (ADR 0067, SC-05):
/// under a 10-byte cap, navigating to the Proving ground's `downloads/1`
/// (~25-byte CSV served with `Content-Disposition: attachment`) turns into a
/// download (`response: null` and a `page.download` event) instead of failing
/// with `response_limit_exceeded`, while `page.request` to the exact same URL
/// on the same Run is refused with `response_limit_exceeded` naming
/// `network.max_body_size`.
///
/// Covers: story:4-107, row:SC-05, setting:network.max_body_size, error:response_limit_exceeded, event:page.download, operation:page.goto, operation:page.request
#[test]
fn a_download_is_not_counted_against_network_max_body_size() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let url = proving_ground.start_url("downloads/1").unwrap();
    let mut run_settings = settings(&proving_ground, &json!({}));
    run_settings["network"]["max_body_size"] = json!(10);
    let mut none = |_: &Value| None;
    let started = engine.call(
        "start",
        "run.start",
        &json!({ "run_id": "download-uncapped", "settings": run_settings }),
        &mut none,
    );
    assert!(started.get("result").is_some(), "{started}");
    let opened = engine.call(
        "page",
        "run.new_page",
        &json!({ "run_id": "download-uncapped" }),
        &mut none,
    );
    let page_id = opened["result"]["page_id"].clone();

    let downloaded = engine.call(
        "goto",
        "page.goto",
        &json!({ "page_id": page_id, "url": url }),
        &mut none,
    );
    let fetched = engine.call(
        "request",
        "page.request",
        &json!({ "page_id": page_id, "url": url }),
        &mut none,
    );
    let finished = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "download-uncapped" }),
        &mut none,
    );
    engine.subscribe_from_start("sub");
    engine.send(&json!({
        "jsonrpc": "2.0",
        "method": "$/credit",
        "params": { "id": "sub", "credits": 64 },
    }));
    let batch = engine.next().expect("the engine replays its events");
    let (_, exited) = engine.finish();
    assert!(exited);

    let downloads: Vec<Value> = batch["params"]["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event["event"] == "page.download")
        .map(|event| event["data"].clone())
        .collect();

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(
        downloaded["result"],
        json!({ "response": null }),
        "{downloaded}"
    );
    assert_eq!(
        downloads,
        [json!({
            "page_id": page_id,
            "url": url,
            "suggested_filename": "export.csv",
        })]
    );
    assert_eq!(
        (
            &fetched["error"]["data"]["code"],
            &fetched["error"]["data"]["setting"],
            &fetched["error"]["data"]["limit"],
        ),
        (
            &json!("response_limit_exceeded"),
            &json!("network.max_body_size"),
            &json!(10),
        ),
        "{fetched}"
    );
}
