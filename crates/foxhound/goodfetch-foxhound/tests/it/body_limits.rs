//! The limits on what the HTTP tiers read (ADR 0067, SC-05, pitfall H14),
//! on Foxhound's executable against the Proving ground: under a forced
//! `http` tier, each decompression bomb (stacked codings included), the
//! body past 64 MiB and the endless bodies end their crawl request
//! `failed` with `response_limit_exceeded` naming its setting, while the
//! page compressed within the ratio, the positive control, succeeds; a
//! handler's `page.request` on that page to a bomb or an endless body
//! answers the same refusal.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::crawl_declaring;

/// The Proving ground's seed.
const SEED: u64 = 644;

/// Each body page and the setting its refusal names.
const REFUSED: [(&str, &str); 8] = [
    ("bombs/1", "network.max_decompression_ratio"),
    ("bombs/2", "network.max_decompression_ratio"),
    ("bombs/3", "network.max_decompression_ratio"),
    ("bombs/4", "network.max_decompression_ratio"),
    ("bombs/5", "network.max_decompression_ratio"),
    ("body-limits/1", "network.max_body_size"),
    ("body-limits/2", "network.max_body_size"),
    ("body-limits/3", "network.max_body_size"),
];

/// The page compressed within the ratio.
const SAFE: &str = "bombs/6";

/// Each request's end by its URL: its kind, its error's code and the
/// setting its details name.
fn ended(engine: &mut crate::crawl::Attached, run_id: &str) -> Vec<(String, Value, Value, Value)> {
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

/// Every body page of the limits table, crawled on a forced `http` tier
/// under a Label that reads `text/plain` (the type of the `body-limits`
/// pages), ends `failed` with `response_limit_exceeded` naming its setting, and
/// the page within the limits succeeds; the handler, which runs on that
/// page alone, gets the same refusal from `page.request` to a bomb and to
/// an endless body.
///
/// Covers: row:SC-05, error:response_limit_exceeded, setting:fetch.tier, setting:tls.trusted_cas, setting:crawl.content_types, operation:page.request
#[test]
fn every_body_past_a_limit_fails_its_request_and_its_page_request_on_the_http_tier() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"body-limits.test\" = ['{}']\n",
        proving_ground.description().ca.display()
    ));
    let mut engine = engine(&env);
    let url = |task: &str| proving_ground.start_url(task).unwrap();
    let seeds: Vec<Value> = REFUSED
        .iter()
        .map(|(task, _)| *task)
        .chain([SAFE])
        .map(|task| json!({ "url": url(task), "label": "page" }))
        .collect();
    let requested = [url("bombs/5"), url("body-limits/2")];

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
                "site_preferences": { "respect_robots_txt": false },
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
    assert!(engine.finish());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let mut expected: Vec<(String, Value, Value, Value)> = REFUSED
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
        ]
    );
}
