//! The limits on what the HTTP tiers read (ADR 0067, SC-05, pitfall H14),
//! on Foxhound's executable against the Proving ground: under a forced
//! `http` tier, each decompression bomb (stacked codings included), the
//! bodies past 64 MiB (a gzip one within the ratio included) and the
//! endless bodies of the hostile-input limits table end their crawl request
//! `failed` with `response_limit_exceeded` naming its setting, while the
//! page compressed within the ratio, the positive control, succeeds; a
//! handler's `page.request` on that page to a bomb or an endless body
//! answers the same refusal.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{
    ExpectedError, HostileTables, LimitSetting, ProvingGround, ServedBy, TestEnv,
};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::crawl_declaring;

/// The Proving ground's seed.
const SEED: u64 = 644;

/// Each body page of the committed limits table and the setting its
/// refusal names: every case a Proving ground page serves that names a
/// body setting. The table's JSON depth case is no crawl request's: its
/// page is read as text, and no JSON is parsed.
fn refused() -> Vec<(String, LimitSetting)> {
    let tables = HostileTables::committed().unwrap();
    let refused: Vec<_> = tables
        .limits
        .cases
        .into_iter()
        .filter_map(|case| match (case.served_by, case.expected) {
            (ServedBy::Page(task), ExpectedError::ResponseLimitExceeded { setting })
                if setting != LimitSetting::MaxJsonDepth =>
            {
                Some((task, setting))
            }
            _ => None,
        })
        .collect();
    for setting in [
        LimitSetting::MaxBodySize,
        LimitSetting::MaxDecompressionRatio,
    ] {
        assert!(
            refused.iter().any(|(_, served)| *served == setting),
            "the table serves no body case naming {setting:?}"
        );
    }
    refused
}

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
        ]
    );
}
