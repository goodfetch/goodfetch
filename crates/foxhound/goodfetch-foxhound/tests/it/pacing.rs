//! Pacing cases (spec #582's Testing Decisions; the protocol against the
//! Proving ground): crawls under `fetch.tier: http` on Foxhound's
//! executable against the `pacing` family's host, whose latency rises with
//! the Proving ground's time step, judged on when the host received each
//! page. The adaptive delay lengthens a domain's delay as its responses
//! slow, never below `politeness.domain.delay` and never past
//! `politeness.backoff.max`; and `politeness.domain.rate` holds two runs
//! sharing the domain key, each in an engine of its own under one
//! Goodfetch home, to the rate together. A Run whose Politeness is its own
//! (`politeness.sharing: run`), held in memory, keeps the same delay and
//! rate (#2171). Eight handlers and eight HTTP
//! fetches run at once, and eight requests may be in flight on the domain,
//! so only the pacing under test spaces the pages, not their answers.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;
use crate::robots_txt::arrivals;

/// The Proving ground's seed.
const SEED: u64 = 897;

/// The `pacing` family's host.
const HOST: &str = "pacing.test";

/// The time step whose answers the host holds about 4.9 s.
const SLOW: u64 = 245;

/// A home whose governor admits eight HTTP fetches at once.
fn home() -> TestEnv {
    let env = TestEnv::new();
    env.user_configuration("governor.http.floor = 8\ngovernor.http.ceiling = 8\n");
    env
}

/// A crawl of `pacing.test` under `fetch.tier: http`, eight handlers at
/// once and eight requests in flight on the domain at most, with `domain`
/// and `backoff` merged into `politeness`, whose Politeness is shared with
/// the user's other Runs or, under `sharing: run`, the Run's own.
fn settings(
    proving_ground: &ProvingGround,
    sharing: &str,
    domain: &Value,
    backoff: &Value,
) -> Value {
    let mut politeness_domain = json!({ "concurrency": 8 });
    if let (Some(into), Some(more)) = (politeness_domain.as_object_mut(), domain.as_object()) {
        into.extend(more.clone());
    }
    json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { HOST: [proving_ground.address().ip().to_string()] } },
        "egress": { "private_ranges": [HOST] },
        "crawl": { "handler": { "concurrency": 8 } },
        "politeness": { "sharing": sharing, "domain": politeness_domain, "backoff": backoff },
    })
}

fn articles(proving_ground: &ProvingGround, numbers: std::ops::RangeInclusive<u64>) -> Vec<Value> {
    numbers
        .map(|number| {
            json!({ "url": proving_ground.url(HOST, &format!("/articles/{number}")), "label": "page" })
        })
        .collect()
}

/// Crawls each of `runs`, a Run ID, its seeds and its settings, at once,
/// each in an engine of its own under `env`'s home, their handlers
/// committing nothing and `handled` told of each page handled, until every
/// Run finished with each of its seeds succeeded.
fn crawl_at_once(env: &TestEnv, runs: &[(&str, Vec<Value>, Value)], handled: &mut dyn FnMut()) {
    let mut engines: Vec<(Attached, &str, usize, bool)> = runs
        .iter()
        .map(|(run, seeds, settings)| {
            let mut engine = attached(env);
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
                "params": { "run_id": run, "seeds": seeds, "settings": settings, "labels": { "page": {} } },
            }));
            (engine, *run, seeds.len(), false)
        })
        .collect();
    while engines.iter().any(|(_, _, _, finished)| !finished) {
        for (engine, run, seeds, finished) in &mut engines {
            if *finished || engine.is_quiet_for(Duration::from_millis(20)) {
                continue;
            }
            let message = engine
                .next()
                .expect("the engine closed before its Run ended");
            if message["method"] == "crawl_request.handle" {
                handled();
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
                assert_eq!(
                    message["result"]["requests"]["succeeded"], *seeds,
                    "{message}"
                );
                *finished = true;
            }
        }
    }
    for (engine, _, _, _) in engines {
        assert!(engine.finished());
    }
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

/// A crawl of 12 of the host's articles, with the adaptive delay on, a
/// `politeness.domain.delay` of 2 s and a `politeness.backoff.max` of 3 s,
/// while the host slows from about 15 ms an answer to about 4.9 s once
/// three pages were handled: its pages arrive at least the delay apart
/// throughout (within the tenth the robots.txt cases allow, since a delay
/// counts between sends), their gaps lengthen to the cap as the slow
/// answers come in, and none reaches 4 s, short of the 4.9 s the latency
/// alone would space them. (The delay is long enough for that tenth to
/// cover a loaded host: as the host slows, each page goes on a new
/// connection while the earlier ones are held, and there a page reached
/// the Proving ground up to about 180 ms after its engine recorded the
/// send, fix-queue item 79. Under that load a gap also came up to 850 ms
/// past the delay in force, from a send that left late.) A Run whose
/// Politeness is its own, held in memory, paces its pages alike: its grant
/// reserves each send, and each send and latency it notes hold the next
/// (#2171).
///
/// Covers: story:6-247, setting:politeness.domain.adaptive_delay, setting:politeness.backoff.max, format:politeness-file
#[rstest]
#[case::shared("shared")]
/// Covers: setting:politeness.sharing
#[case::the_runs_own("run")]
fn the_adaptive_delay_lengthens_as_the_host_slows_within_the_delay_and_the_backoff_max(
    #[case] sharing: &str,
) {
    let env = home();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut handled = 0;

    crawl_at_once(
        &env,
        &[(
            "adaptive",
            articles(&proving_ground, 1..=12),
            settings(
                &proving_ground,
                sharing,
                &json!({ "delay": 2_000, "adaptive_delay": true }),
                &json!({ "max": 3_000 }),
            ),
        )],
        &mut || {
            handled += 1;
            if handled == 3 {
                proving_ground.set_time_step(SLOW).unwrap();
            }
        },
    );
    let gaps = gaps(&proving_ground);

    let never_below = gaps.iter().all(|gap| *gap >= 1_800_000);
    let lengthened = gaps.iter().rev().take(4).all(|gap| *gap >= 2_250_000);
    let never_past = gaps.iter().all(|gap| *gap < 4_000_000);
    assert!(
        gaps.len() == 11 && never_below && lengthened && never_past,
        "{gaps:?}"
    );
}

/// Two runs, each in an engine of its own under one Goodfetch home,
/// crawling three articles each of one domain key at once, each asking for
/// 30 requests a minute with a burst of 1 and no delay of its own: the
/// host receives the six pages together no closer than 2 s apart, within
/// the tenth a start's send may come before the host sees it, so the two
/// runs together never pass the rate; with no rate they would arrive
/// milliseconds apart. (The interval is long enough for that tenth to
/// cover a loaded host: there a page reached the Proving ground up to
/// about 190 ms after its engine recorded the send, fix-queue item 77.)
///
/// Covers: story:6-248, setting:politeness.domain.rate, setting:politeness.domain.burst, format:politeness-file
#[test]
fn a_domain_rate_holds_two_runs_sharing_its_key_to_it() {
    let env = home();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let rated = settings(
        &proving_ground,
        "shared",
        &json!({ "rate": 30, "burst": 1 }),
        &json!({}),
    );

    crawl_at_once(
        &env,
        &[
            ("first", articles(&proving_ground, 1..=3), rated.clone()),
            ("second", articles(&proving_ground, 4..=6), rated),
        ],
        &mut || {},
    );
    let gaps = gaps(&proving_ground);

    assert!(
        gaps.len() == 5 && gaps.iter().all(|gap| *gap >= 1_800_000),
        "{gaps:?}"
    );
}

/// A Run whose Politeness is its own (`politeness.sharing: run`), held in
/// memory, crawling six articles of one domain key at 30 requests a minute
/// with a burst of 1 and no delay: the host receives its pages no closer
/// than 2 s apart, within the tenth the shared case allows, as each grant
/// reserves its send at the token the rate allows and the send waits for
/// it without a second round trip to the store (#2171).
///
/// Covers: story:6-248, setting:politeness.domain.rate, setting:politeness.sharing
#[test]
fn a_domain_rate_holds_a_run_whose_politeness_is_its_own_to_it() {
    let env = home();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let rated = settings(
        &proving_ground,
        "run",
        &json!({ "rate": 30, "burst": 1 }),
        &json!({}),
    );

    crawl_at_once(
        &env,
        &[("own", articles(&proving_ground, 1..=6), rated)],
        &mut || {},
    );
    let gaps = gaps(&proving_ground);

    assert!(
        gaps.len() == 5 && gaps.iter().all(|gap| *gap >= 1_800_000),
        "{gaps:?}"
    );
}
