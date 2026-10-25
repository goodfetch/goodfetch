//! Pacing cases (spec #582's Testing Decisions; the protocol against the
//! Proving ground): crawls under `fetch.tier: http` on Foxhound's
//! executable against the `pacing` family's host, whose latency rises with
//! the Proving ground's time step, judged on when the host received each
//! page. The adaptive delay lengthens a domain's delay as its responses
//! slow, never below `politeness.domain.delay` and never past
//! `politeness.backoff.max`; and `politeness.domain.rate` holds two runs
//! sharing the domain key, each in an engine of its own under one
//! Goodfetch home, to the rate together.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::crawl_with;
use crate::robots_txt::arrivals;

/// The Proving ground's seed.
const SEED: u64 = 897;

/// The `pacing` family's host.
const HOST: &str = "pacing.test";

/// The time step whose answers the host holds about 410 ms.
const SLOW: u64 = 20;

/// A crawl of `pacing.test` under `fetch.tier: http`, four handlers at
/// once, with `politeness` merged in.
fn settings(proving_ground: &ProvingGround, politeness: &Value) -> Value {
    json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { HOST: [proving_ground.address().ip().to_string()] } },
        "crawl": { "handler": { "concurrency": 4 } },
        "politeness": politeness,
    })
}

fn articles(proving_ground: &ProvingGround, numbers: std::ops::RangeInclusive<u64>) -> Vec<Value> {
    numbers
        .map(|number| {
            json!({ "url": proving_ground.url(HOST, &format!("/articles/{number}")), "label": "page" })
        })
        .collect()
}

/// The gaps between the arrivals of the host's pages, robots.txt left out,
/// in microseconds.
fn gaps(proving_ground: &ProvingGround) -> Vec<u64> {
    let pages: Vec<u64> = arrivals(proving_ground, HOST)
        .into_iter()
        .filter(|(target, _)| target != "/robots.txt")
        .map(|(_, at)| at)
        .collect();
    pages
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect()
}

/// A crawl of the host's 20 articles, with the adaptive delay on, a
/// `politeness.domain.delay` of 100 ms and a `politeness.backoff.max` of
/// 250 ms, while the host slows from about 15 ms an answer to about 410 ms
/// once three pages were handled: its pages arrive at least the delay
/// apart throughout (within the tenth the robots.txt cases allow, since a
/// delay counts between starts), their gaps lengthen to the cap as the
/// slow answers come in, and never pass it by more than a fifth, where the
/// latency alone would space them 410 ms.
///
/// Covers: story:6-247, setting:politeness.domain.adaptive_delay, setting:politeness.backoff.max, format:politeness-file
#[test]
fn the_adaptive_delay_lengthens_as_the_host_slows_within_the_delay_and_the_backoff_max() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = Attached::start(&env);
    let mut handled = 0;

    let report = crawl_with(
        &mut engine,
        "adaptive",
        &articles(&proving_ground, 1..=20),
        &settings(
            &proving_ground,
            &json!({
                "domain": { "delay": 100, "adaptive_delay": true },
                "backoff": { "max": 250 },
            }),
        ),
        &mut |_, _| {
            handled += 1;
            if handled == 3 {
                proving_ground.set_time_step(SLOW).unwrap();
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());
    let gaps = gaps(&proving_ground);

    let never_below = gaps.iter().all(|gap| *gap >= 90_000);
    let lengthened = gaps.iter().rev().take(4).all(|gap| *gap >= 200_000);
    let never_past = gaps.iter().all(|gap| *gap <= 300_000);
    assert!(
        report["result"]["lifecycle"] == "finished"
            && gaps.len() == 19
            && never_below
            && lengthened
            && never_past,
        "{gaps:?} {report}"
    );
}

/// Two runs, each in an engine of its own under one Goodfetch home,
/// crawling six articles each of one domain key at once, each asking for
/// 240 requests a minute with a burst of 1 and no delay of its own: the
/// host receives the twelve pages together no closer than 250 ms apart,
/// within the tenth a start's send may come after it, so the two runs
/// together never pass the rate; with no rate they would arrive
/// milliseconds apart.
///
/// Covers: story:6-248, setting:politeness.domain.rate, setting:politeness.domain.burst, format:politeness-file
#[test]
fn a_domain_rate_holds_two_runs_sharing_its_key_to_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let politeness = json!({ "domain": { "rate": 240, "burst": 1 } });
    let runs = [("first", 1..=6), ("second", 7..=12)];
    let mut engines: Vec<(Attached, &str, bool)> = runs
        .iter()
        .map(|(run, numbers)| {
            let mut engine = Attached::start(&env);
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
                "params": {
                    "run_id": run,
                    "seeds": articles(&proving_ground, numbers.clone()),
                    "settings": settings(&proving_ground, &politeness),
                },
            }));
            (engine, *run, false)
        })
        .collect();
    while engines.iter().any(|(_, _, finished)| !finished) {
        for (engine, run, finished) in &mut engines {
            if *finished || engine.is_quiet_for(Duration::from_millis(50)) {
                continue;
            }
            let message = engine
                .next()
                .expect("the engine closed before its Run ended");
            if message["method"] == "crawl_request.handle" {
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
                }));
            } else if message["id"] == "start" {
                assert_eq!(message["result"]["run_id"], *run, "{message}");
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": "finish", "method": "run.finish",
                    "params": { "run_id": run },
                }));
            } else if message["id"] == "finish" {
                assert_eq!(message["result"]["lifecycle"], "finished", "{message}");
                *finished = true;
            }
        }
    }
    for (engine, _, _) in engines {
        assert!(engine.finish());
    }
    let gaps = gaps(&proving_ground);

    assert!(
        gaps.len() == 11 && gaps.iter().all(|gap| *gap >= 225_000),
        "{gaps:?}"
    );
}
