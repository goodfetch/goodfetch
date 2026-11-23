//! A crawl stopping while a handler's Effect body runs, through the
//! engine's public interface over the in-memory network (spec #213's Seam
//! 3), on the paused clock: stopped by its `timeout.run`, a Limit or a
//! Daemon's drain, the crawl waits for the body's `effect.close`, which
//! the Effect journal keeps `done`, before it stops (ADR 0106).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::daemon::{DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::ops::Ops;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::transport::{self, Network};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    Client, Reply, crawl_connected, default_dependencies, engine, page_of, run_file, seeds,
};

/// How long the first attempt's Effect body runs: past the 5 s
/// `timeout.run`, and within the 60 s handler timeout of the settings'
/// defaults.
const BODY: Duration = Duration::from_secs(8);

/// The handler of the tests' crawls: the first attempt runs the Effect
/// `ship`, whose body takes `took` and returns 1, then answers one Record,
/// as every later attempt does at once; with no `took`, the body never
/// returns and the attempt is never answered.
fn shipping(run_id: &str, took: Option<Duration>) -> impl FnMut(&Value) -> Reply {
    let run_id = run_id.to_owned();
    let mut first = true;
    move |params| {
        let records = Reply::Records(vec![json!({ "page": page_of(params) })]);
        if !std::mem::take(&mut first) {
            return records;
        }
        let attempt = json!({
            "crawl_request_id": params["crawl_request"]["id"],
            "lease_epoch": params["lease_epoch"],
        });
        let close = json!({
            "run_id": run_id,
            "entry": "1",
            "outcome": { "ended": "returned", "output": 1 },
        });
        let then = match took {
            Some(took) => Reply::After(
                took,
                Box::new(Reply::Calls(
                    vec![("effect.close", close)],
                    Box::new(records),
                )),
            ),
            None => Reply::Silent,
        };
        Reply::Calls(
            vec![(
                "effect.open",
                json!({ "run_id": run_id, "key": "ship", "attempt": attempt }),
            )],
            Box::new(then),
        )
    }
}

/// Each Effect journal row's key and state, in entry order.
fn journal(env: &TestEnv, run_id: &str) -> Vec<(String, String)> {
    run_file(env, run_id)
        .prepare("SELECT key, state FROM effect_journal ORDER BY entry")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A crawl whose `timeout.run` passes, or whose one-request Limit is used
/// up, while its first attempt's Effect body runs, waits for the body's
/// `effect.close`, judged `done`, and stops for that reason, its second
/// work item, admitted 10 s after the first, still pending.
///
/// Covers: story:3-52
#[rstest]
#[case::run_timeout(json!({ "timeout": { "run": 5_000 } }), None, json!({ "kind": "timeout" }))]
#[case::limit(
    json!({}),
    Some(json!([{ "usage_kind": "requests", "amount": 1 }])),
    json!({ "kind": "limit", "scope": { "kind": "run", "id": "shipped" }, "usage_kind": "requests" })
)]
fn a_crawl_stopping_while_an_effect_runs_waits_for_its_close(
    #[case] settings: Value,
    #[case] limits: Option<Value>,
    #[case] stopped: Value,
) {
    let env = TestEnv::new();
    let mut start = json!({
        "labels": { "page": {} },
        "run_id": "shipped",
        "seeds": seeds(2),
        "settings": { "admission": { "max_requests_per_minute": 6 } },
    });
    if let Some(limits) = limits {
        start["limits"] = limits;
    }

    let (report, closed) = crawl_connected(
        &settings,
        || default_dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("crawl.start", start, &mut |_| Reply::Silent)
                .await;
            let report = client
                .call(
                    "run.finish",
                    json!({ "run_id": "shipped" }),
                    &mut shipping("shipped", Some(BODY)),
                )
                .await;
            (report, client.replies.get(1).cloned())
        },
    );

    assert_eq!(
        (
            closed.as_ref().map(|closed| &closed["result"]["judged"]),
            (
                &report["result"]["lifecycle"],
                &report["result"]["completeness"],
                &report["result"]["stop_reason"],
            ),
            journal(&env, "shipped"),
        ),
        (
            Some(&json!("done")),
            (&json!("stopped"), &json!("incomplete"), &stopped),
            vec![("ship".to_owned(), "done".to_owned())],
        ),
        "{report:#?} {closed:#?}"
    );
}

/// The Daemon's token, which `hello` carries.
const TOKEN: &str = "the-launch-token";

/// The Daemon's major's pick, which never hands over.
struct Pick;

impl Succession for Pick {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

/// A Daemon asked to drain while a crawl's handler runs an Effect body
/// waits for the body's `effect.close` before the crawl stops for the
/// drain (ADR 0049), judged `done`; a body that never returns is waited
/// for only until half the drain's grace, then cut short, and dropped as
/// nothing was sent for it, the Daemon still drained within its grace.
///
/// Covers: story:3-52, story:3-99
#[rstest]
#[case::closed(
    Duration::from_secs(110),
    Some(Duration::from_secs(4)),
    Some(json!("done")),
    vec![("ship".to_owned(), "done".to_owned())]
)]
#[case::cut_short(Duration::from_secs(4), None, None, Vec::new())]
fn a_drained_crawl_waits_for_its_running_effect_to_close(
    #[case] grace: Duration,
    #[case] took: Option<Duration>,
    #[case] judged: Option<Value>,
    #[case] journaled: Vec<(String, String)>,
) {
    let env = TestEnv::new();
    let asked = CancellationToken::new();
    let options = DaemonOptions {
        token: TOKEN.to_owned(),
        endpoint: DaemonEndpoint::Unix("daemon.sock".into()),
        pid: 4242,
        idle_timeout: Duration::from_secs(600),
        keep_alive: true,
        holds: Holds::new(),
        succession: Arc::new(Pick),
        drain: Drain {
            asked: Some(asked.clone()),
            grace,
        },
        install: None,
        ops: Ops::default(),
        websocket: None,
    };

    let engine = engine();
    let (end, (report, closed)) = goodfetch_test_support::runtime().block_on(async {
        let dependencies = default_dependencies(&env, RunStateOptions::default());
        let (network, dialer) = transport::in_memory();
        let (listener, _) = Network::InMemory(network)
            .listen(std::path::Path::new("daemon.sock"))
            .unwrap();
        let client = async {
            let mut client = Client::dialed(&dialer).await;
            client
                .call(
                    "hello",
                    json!({
                        "protocol_version": "0.1",
                        "surface": "rust",
                        "needs": "foxhound",
                        "product_version": "1.4.0",
                        "token": TOKEN,
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            client
                .call(
                    "crawl.start",
                    json!({ "labels": { "page": {} }, "run_id": "drained", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let drain = async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                asked.cancel();
            };
            let mut handler = shipping("drained", took);
            let ((), report) = tokio::join!(
                drain,
                client.call("run.finish", json!({ "run_id": "drained" }), &mut handler)
            );
            let closed = client.replies.get(1).cloned();
            client.close().await;
            (report, closed)
        };
        tokio::join!(engine.run_daemon(listener, dependencies, options), client)
    });

    assert_eq!(
        (
            matches!(end, DaemonEnd::Drained),
            closed.as_ref().map(|closed| &closed["result"]["judged"]),
            (
                &report["result"]["lifecycle"],
                &report["result"]["stop_reason"],
            ),
            journal(&env, "drained"),
        ),
        (
            true,
            judged.as_ref(),
            (&json!("stopped"), &json!({ "kind": "drain" })),
            journaled,
        ),
        "{end:?} {report:#?} {closed:#?}"
    );
}
