//! Limits across runs through the engine's public interface over the
//! in-memory network (spec #213's Seam 3): `usage.set` and `usage.list`; a
//! Run that charges Usage against its Slices and stops at its next safe
//! point once a Limit has nothing left, `stopped` and `incomplete` for that
//! Limit; a new Run refused with `limit_reached` or `limit_not_understood`;
//! a value only Foxhound carries refused by a Collie engine; and the Slices
//! of a dead engine's Run reclaimed by the next engine that opens the Usage
//! ledger, from what the Run journaled, open reservations at their worst
//! case. A test's own operation, `test.charge`, starts a Run that charges
//! `requests` one at a time, as a crawl does before each request, first
//! any it journals only in one write once they are all charged, as a
//! crawl's claim does; another,
//! `test.reserve`, one that reserves a paid solve's worst case and settles
//! it, as area 8's solvers will.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{LimitScope, RunId, RunKind};
use goodfetch_protocol::usage::Limit;
use goodfetch_test_support::TestEnv;
use jiff::Timestamp;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt as _;
use tokio::io::BufReader;

use super::run::call;
use super::{Client, attached_on, dependencies, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Dependencies;
use crate::ledger::Ledger;
use crate::runs::{ChargeError, EndedBy, Start, Started, UnjournaledCharges};
use crate::tasks::BlockingPolicy;
use crate::transport::{self, Network};

/// Starts a Run of its own and charges `unjournaled` requests, one at a
/// time, journaled in one write only once `amount` more are charged, each
/// journaled as it is, until one is refused.
const CHARGE: Operation = Operation {
    name: "test.charge",
    description: "Starts a Run that charges requests one at a time.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Starts a Run of its own that reserves a paid solve's worst case in USD,
/// then settles it at its cost when one is given.
const RESERVE: Operation = Operation {
    name: "test.reserve",
    description: "Starts a Run that reserves a paid solve's worst case.",
    ..CHARGE
};

/// Starts a Run of its own that takes a Slice of its requests and one of
/// their price, then holds the runtime's blocking threads while it makes
/// each change of its charging that the Slices cover, priced and not,
/// beside a charge that needs another Slice; then lets the threads go.
const COVERED: Operation = Operation {
    name: "test.covered",
    description: "Starts a Run that makes covered changes while the blocking threads are held.",
    ..CHARGE
};

/// Starts a Run of its own whose `tasks` tasks charge requests one at a
/// time, `each` of them each, all at once; answers how many they charged,
/// what the Run's state then journals of its Slices, and whether a
/// Slice's journaled Usage went back meanwhile.
const CHARGE_FROM_TASKS: Operation = Operation {
    name: "test.charge_from_tasks",
    description: "Starts a Run whose tasks charge requests at once.",
    ..CHARGE
};

/// How long [`COVERED`] holds a blocking thread at most: a change that
/// needed the thread then gets it, and the test says it was let go late.
const HELD_AT_MOST: std::time::Duration = std::time::Duration::from_secs(20);

/// Starts a new Collie Run named by `params`' `run_id`, tagged with its
/// `tags`.
async fn started(params: &Value, context: &CallContext) -> Result<crate::runs::Run, CallError> {
    let started = context
        .runs()
        .start(Start {
            run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
            tags: serde_json::from_value(params["tags"].clone()).unwrap_or_default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Finish,
            initialize: None,
            policy_version: None,
            weakened: Vec::new(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: None,
            timeout: None,
            settings: None,
        })
        .await
        .map_err(crate::run::start_error)?;
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    Ok(run)
}

/// A Limit a newer engine wrote, flagged *must understand*, on a scope kind
/// no engine of today knows.
fn newer_limit() -> Limit {
    Limit {
        scope: LimitScope {
            kind: "galaxy".to_owned(),
            id: "milky-way".to_owned(),
        },
        usage_kind: "requests".to_owned(),
        currency: None,
        period: None,
        amount: 10,
        must_understand: true,
        set_by: LimitSetter::Person,
    }
}

pub(super) fn charging() -> Contribution {
    Contribution::new("test-usage")
        .with_handler(Handler::new(
            &CHARGE,
            |params: Value, context: CallContext| async move {
                let run = started(&params, &context).await?;
                let mut charged = 0;
                let mut refused = false;
                let mut unjournaled = UnjournaledCharges::default();
                for _ in 0..params["unjournaled"].as_u64().unwrap_or(0) {
                    run.charge_unjournaled(&mut unjournaled, "requests", 1)
                        .await
                        .unwrap();
                    charged += 1;
                }
                for _ in 0..params["amount"].as_u64().unwrap() {
                    if params["newer_limit_after"].as_u64() == Some(charged) {
                        Ledger::at(context.engine().home().usage_ledger(), None)
                            .add_limit(&newer_limit(), Timestamp::UNIX_EPOCH)
                            .unwrap();
                    }
                    match run.charge("requests", 1).await {
                        Ok(()) => charged += 1,
                        Err(ChargeError::Refused(_)) => {
                            refused = true;
                            break;
                        }
                        Err(error) => panic!("the charge failed: {error}"),
                    }
                }
                if !unjournaled.is_empty() {
                    run.queue_charged(unjournaled, |_| Ok(()))
                        .await
                        .unwrap()
                        .await
                        .unwrap();
                }
                Ok::<Value, CallError>(json!({ "charged": charged, "refused": refused }))
            },
        ))
        .with_handler(Handler::new(&COVERED, covered))
        .with_handler(Handler::new(&CHARGE_FROM_TASKS, charge_from_tasks))
        .with_handler(Handler::new(
            &RESERVE,
            |params: Value, context: CallContext| async move {
                let run = started(&params, &context).await?;
                let reserved = run
                    .reserve(
                        "paid_solves",
                        Some("USD"),
                        params["worst"].as_u64().unwrap(),
                    )
                    .await;
                let Ok(reservation) = reserved else {
                    return Ok::<Value, CallError>(json!({ "refused": true }));
                };
                if let Some(cost) = params["cost"].as_u64() {
                    run.settle(reservation, cost).await.unwrap();
                }
                Ok(json!({ "refused": false }))
            },
        ))
}

/// Holds one of the runtime's blocking threads until its sender is used,
/// [`HELD_AT_MOST`]; the task answers whether it was let go in time. While
/// a blocking thread is held, the paused clock stands still.
async fn held_thread() -> (std::sync::mpsc::Sender<()>, tokio::task::JoinHandle<bool>) {
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (holding, holds) = tokio::sync::oneshot::channel();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test holds one of the runtime's own blocking threads, which a tracker's task would not name (TESTING.md, Time)"
    )]
    let thread = tokio::runtime::Handle::current().spawn_blocking(move || {
        holding.send(()).unwrap();
        held.recv_timeout(HELD_AT_MOST).is_ok()
    });
    holds.await.unwrap();
    (release, thread)
}

/// [`COVERED`]'s handler, on a runtime of two blocking threads.
async fn covered(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run = started(&params, &context).await?;
    // The clock stands still from here, so the Limits the Run reads as it
    // takes its Slice stay current, and the other thread serves that charge.
    let first = held_thread().await;
    run.charge("requests", 1).await.unwrap();
    let priced = run.reserve("requests", Some("USD"), 1).await.unwrap();
    run.settle(priced, 1).await.unwrap();
    let second = held_thread().await;

    let changes = async {
        run.charge("requests", 1).await.unwrap();
        run.charge_part("requests", "first", 1, None).await.unwrap();
        run.charge_part("requests", "first", 1, Some(("USD", 3)))
            .await
            .unwrap();
        let mut unjournaled = UnjournaledCharges::default();
        run.charge_unjournaled(&mut unjournaled, "requests", 1)
            .await
            .unwrap();
        let reservation = run.reserve("requests", None, 2).await.unwrap();
        run.settle(reservation, 1).await.unwrap();
        let allowance = run
            .reserve_part("requests", "first", None, |amount| amount, 2, 1)
            .await
            .unwrap()
            .unwrap();
        run.settle_part(allowance, 1, 0).await.unwrap();
        let allowance = run
            .reserve_part("requests", "first", Some("USD"), |amount| amount * 3, 2, 1)
            .await
            .unwrap()
            .unwrap();
        run.settle_part(allowance, 1, 3).await.unwrap();
        unjournaled
    };
    let another_slice = run.charge("requests", 20);
    tokio::pin!(another_slice);
    let (unjournaled, charged_while_held) = tokio::select! {
        biased;
        charged = &mut another_slice => (None, Some(charged.is_ok())),
        unjournaled = changes => (Some(unjournaled), None),
    };
    let mut held_throughout = true;
    for (release, thread) in [first, second] {
        // A thread held to its end is gone, and answers that it was.
        let _ = release.send(());
        held_throughout &= thread.await.unwrap();
    }
    let charged_once_let_go = match charged_while_held {
        Some(_) => None,
        None => Some(another_slice.await.is_ok()),
    };
    if let Some(unjournaled) = unjournaled {
        run.queue_charged(unjournaled, |_| Ok(()))
            .await
            .unwrap()
            .await
            .unwrap();
    }
    Ok(json!({
        "threads_held_throughout": held_throughout,
        "another_slice_charged_while_held": charged_while_held,
        "another_slice_charged_once_let_go": charged_once_let_go,
    }))
}

/// [`CHARGE_FROM_TASKS`]'s handler.
async fn charge_from_tasks(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run = started(&params, &context).await?;
    let each = params["each"].as_u64().unwrap();
    let ended: Vec<_> = (0..params["tasks"].as_u64().unwrap())
        .map(|task| {
            let (end, ended) = tokio::sync::oneshot::channel();
            let charging = run.clone();
            run.spawn(async move {
                let mut charged = 0_u64;
                for charge in 0..each {
                    // Every third charge is a claim's, journaled in a write
                    // of its own, as a crawl's tasks mix them.
                    let made = if (task + charge) % 3 == 0 {
                        let mut unjournaled = UnjournaledCharges::default();
                        match charging
                            .charge_unjournaled(&mut unjournaled, "requests", 1)
                            .await
                        {
                            Ok(()) => match charging.queue_charged(unjournaled, |_| Ok(())).await {
                                Ok(written) => written.await,
                                Err(error) => Err(error),
                            },
                            Err(error) => Err(error),
                        }
                    } else {
                        charging.charge("requests", 1).await
                    };
                    charged += u64::from(made.is_ok());
                }
                let _ = end.send(charged);
            });
            ended
        })
        .collect();
    let all_charged = async {
        let mut charged = 0;
        for ended in ended {
            charged += ended.await.unwrap();
        }
        charged
    };
    tokio::pin!(all_charged);
    // The journal is read again and again while the tasks charge: these
    // charges only add, so a Slice's journaled Usage that went back is a
    // journal that reached the run state after a later one.
    let mut journal = Journal::default();
    let charged = loop {
        tokio::select! {
            biased;
            charged = &mut all_charged => break charged,
            slices = journaled_slices(&run) => journal.read(slices),
        }
    };
    // A claim's journal rides the open batch at a level that batches
    // attempts, answered before the batch commits and seen by a reader only
    // once it does (ADR 0142): the last read follows the batch's seal.
    run.state().seal().await.unwrap();
    journal.read(journaled_slices(&run).await);
    Ok(json!({
        "charged": charged,
        "journaled": journal.used.values().sum::<u64>(),
        "went_back": journal.went_back,
    }))
}

/// What a Run's state journals of each Slice, as a test read it over time.
#[derive(Default)]
struct Journal {
    /// Each Slice's journaled Usage at the last read.
    used: BTreeMap<String, u64>,
    /// Whether a read found a Slice's journaled Usage below an earlier
    /// read's.
    went_back: bool,
}

impl Journal {
    fn read(&mut self, slices: Vec<(String, u64)>) {
        for (slice, used) in slices {
            let before = self.used.insert(slice, used);
            self.went_back |= before.is_some_and(|before| used < before);
        }
    }
}

/// Each Slice `run`'s state journals, with the Run's Usage of it.
async fn journaled_slices(run: &crate::runs::Run) -> Vec<(String, u64)> {
    run.state()
        .read(|connection| {
            connection
                .prepare("SELECT CAST(slice_id AS TEXT), used FROM usage")?
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get::<_, i64>(1)?.cast_unsigned()))
                })?
                .collect()
        })
        .await
        .unwrap()
}

pub(super) fn monthly_install_limit(amount: u64) -> Value {
    json!({
        "scope": "install",
        "usage_kind": "requests",
        "period": { "every": "month", "time_zone": "UTC" },
        "amount": amount,
    })
}

/// The install's monthly Limit on requests, as the engine names it.
pub(super) fn install_month() -> Value {
    json!({
        "scope": { "kind": "install", "id": "" },
        "usage_kind": "requests",
        "period": { "every": "month", "time_zone": "UTC" },
        "resets_at": "2026-02-01T00:00:00Z",
    })
}

/// Runs a Collie engine with `test.charge` in `env` while the Client makes
/// `calls`, after its `hello`, and answers each reply.
fn replies(env: &TestEnv, product: Product, calls: &[(&str, Value)]) -> Vec<Value> {
    replies_from(env, product, "sdk_typescript", calls)
}

/// As [`replies`], from a Client whose `hello` names `surface`.
fn replies_from(
    env: &TestEnv,
    product: Product,
    surface: &str,
    calls: &[(&str, Value)],
) -> Vec<Value> {
    let engine = engine_with(product, Some(charging()));
    let calls: Vec<(String, Value)> = calls
        .iter()
        .map(|(method, params)| ((*method).to_owned(), params.clone()))
        .collect();
    let needs = product.as_str();
    let mut opening = hello(needs, "0.1");
    opening["params"]["surface"] = json!(surface);
    attached_on(&engine, env, |mut client: Client| async move {
        client.send(&[opening]).await;
        client.next().await;
        let mut replies = Vec::new();
        for (id, (method, params)) in (1..).zip(calls) {
            replies.push(call(&mut client, id, &method, params).await);
        }
        replies
    })
}

/// Covers: story:3-71
#[test]
fn a_run_that_cannot_take_another_slice_stops_at_its_next_safe_point_naming_the_limit() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            ("usage.set", monthly_install_limit(2)),
            ("test.charge", json!({ "run_id": "capped", "amount": 3 })),
            ("run.finish", json!({ "run_id": "capped" })),
        ],
    );

    let report = &replies[2]["result"];
    let mut stop_reason = install_month();
    stop_reason["kind"] = json!("limit");
    assert_eq!(
        (
            &replies[1]["result"],
            &report["lifecycle"],
            &report["completeness"],
            &report["stop_reason"],
        ),
        (
            &json!({ "charged": 2, "refused": true }),
            &json!("stopped"),
            &json!("incomplete"),
            &stop_reason,
        )
    );
}

/// Covers: story:3-71, error:limit_reached
#[test]
fn a_new_run_is_refused_with_limit_reached_naming_the_scope_and_period() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            ("usage.set", monthly_install_limit(2)),
            ("test.charge", json!({ "run_id": "first", "amount": 2 })),
            ("run.finish", json!({ "run_id": "first" })),
            ("run.start", json!({ "run_id": "second" })),
        ],
    );

    let data = &replies[3]["error"]["data"];
    let named: serde_json::Map<String, Value> = install_month()
        .as_object()
        .unwrap()
        .keys()
        .map(|key| (key.clone(), data[key].clone()))
        .collect();
    assert_eq!(
        (&data["code"], Value::Object(named)),
        (&json!("limit_reached"), install_month())
    );
}

/// Covers: story:3-79, operation:usage.list, operation:usage.set
#[test]
fn usage_lists_the_usage_against_each_limit_by_scope() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            ("usage.set", monthly_install_limit(1_000)),
            (
                "usage.set",
                json!({ "scope": "person", "usage_kind": "requests", "amount": 50 }),
            ),
            ("test.charge", json!({ "run_id": "counted", "amount": 3 })),
            ("run.finish", json!({ "run_id": "counted" })),
            ("usage.list", json!({})),
        ],
    );

    assert_eq!(
        replies[4]["result"],
        json!({ "limits": [
            {
                "limit": {
                    "scope": { "kind": "install", "id": "" },
                    "usage_kind": "requests",
                    "period": { "every": "month", "time_zone": "UTC" },
                    "amount": 1_000,
                    "must_understand": true,
                    "set_by": "client",
                },
                "standing": "held",
                "charged": 3,
                "held": 0,
                "left": 997,
                "period_started_at": "2026-01-01T00:00:00Z",
                "resets_at": "2026-02-01T00:00:00Z",
            },
            {
                "limit": {
                    "scope": { "kind": "person", "id": "os_user" },
                    "usage_kind": "requests",
                    "amount": 50,
                    "must_understand": true,
                    "set_by": "client",
                },
                "standing": "held",
                "charged": 3,
                "held": 0,
                "left": 47,
            },
        ]})
    );
}

/// The Error code of `product`'s engine's reply to a `usage.set` of a
/// `paid_fetch` Limit, null when it holds it.
fn paid_fetch_set_by(product: Product) -> Value {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        product,
        &[(
            "usage.set",
            json!({ "scope": "install", "usage_kind": "paid_fetch", "currency": "USD", "amount": 5_000_000 }),
        )],
    );

    replies[0]["error"]["data"]["code"].clone()
}

/// `paid_fetch` is a usage kind only Foxhound carries: a Collie engine
/// refuses a Limit on it with `not_in_product`.
/// Covers: story:3-67, error:not_in_product
#[test]
fn a_collie_engine_refuses_a_paid_fetch_limit_with_not_in_product() {
    assert_eq!(paid_fetch_set_by(Product::Collie), json!("not_in_product"));
}

/// `paid_fetch` is a usage kind only Foxhound carries: a Foxhound engine
/// holds a Limit on it.
/// Covers: story:3-67
#[test]
fn a_foxhound_engine_holds_a_paid_fetch_limit() {
    assert_eq!(paid_fetch_set_by(Product::Foxhound), Value::Null);
}

/// Covers: story:3-74, error:limit_not_understood
#[test]
fn a_limit_a_newer_engine_wrote_refuses_runs_with_limit_not_understood() {
    let env = TestEnv::new();
    Ledger::at(env.home().join("usage-ledger.sqlite"), None)
        .add_limit(
            &Limit {
                scope: LimitScope {
                    kind: "galaxy".to_owned(),
                    id: "milky-way".to_owned(),
                },
                usage_kind: "requests".to_owned(),
                currency: None,
                period: None,
                amount: 10,
                must_understand: true,
                set_by: LimitSetter::Person,
            },
            Timestamp::UNIX_EPOCH,
        )
        .unwrap();

    let replies = replies(
        &env,
        Product::Collie,
        &[("run.start", json!({ "run_id": "refused" }))],
    );

    assert_eq!(
        replies[0]["error"]["data"]["code"],
        json!("limit_not_understood")
    );
}

/// An engine's start reclaims nothing from a ledger no engine created, and
/// leaves it uncreated: a Run call is answered only once the reclaim ended.
#[test]
fn an_engine_that_starts_in_a_new_home_creates_no_usage_ledger() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[("run.finish", json!({ "run_id": "absent" }))],
    );

    assert_eq!(
        (
            &replies[0]["error"]["data"]["code"],
            env.home().join("usage-ledger.sqlite").exists()
        ),
        (&json!("run_not_found"), false)
    );
}

/// Runs a Collie engine in `env` that makes `calls`, then dies with its
/// Runs live: its runtime is dropped, as a killed process's would be.
fn killed_after(env: &TestEnv, calls: &[(&str, Value)]) {
    called_on(
        goodfetch_test_support::runtime(),
        BlockingPolicy::Inline,
        env,
        calls,
    );
}

/// A paused runtime with two blocking threads, which a test can hold: no
/// blocking call is then made until it lets one go.
fn runtime_with_two_blocking_threads() -> tokio::runtime::Runtime {
    #[expect(
        clippy::disallowed_methods,
        reason = "the test bounds the runtime's blocking threads, which the shared test runtime does not (ADR 0060)"
    )]
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .max_blocking_threads(2)
        .build()
        .unwrap()
}

/// Runs a Collie engine in `env` on `runtime`, its blocking work run as
/// `blocking` says, that makes `calls`; answers each call's result. The
/// engine then dies with its Runs live, as [`killed_after`]'s does.
fn called_on(
    runtime: tokio::runtime::Runtime,
    blocking: BlockingPolicy,
    env: &TestEnv,
    calls: &[(&str, Value)],
) -> Vec<Value> {
    let results = runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let engine = engine_with(Product::Collie, Some(charging()));
        let serving = engine.run_attached(
            Network::InMemory(network),
            Dependencies {
                blocking,
                ..dependencies(env)
            },
        );
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let mut results = Vec::new();
            for (id, (method, params)) in (1..).zip(calls) {
                let reply = call(&mut client, id, method, params.clone()).await;
                assert!(reply.get("result").is_some(), "{reply}");
                results.push(reply["result"].clone());
            }
            results
        };
        tokio::select! {
            served = serving => panic!("the engine ended: {served:?}"),
            results = talk => results,
        }
    });
    drop(runtime);
    results
}

/// A change of a Run's charging that its Slices cover is made where the
/// Run asks for it, with no blocking call (ADR 0140's E3, issue 2603): a
/// charge, a part's with a price and without, a claim's, a reservation,
/// an allowance with a price and without, and their settling all end
/// while the runtime's blocking threads are held, in an
/// engine that runs blocking work on them, as a Product does. A charge
/// that needs another Slice still waits for a thread, and is made once one
/// is let go.
///
/// Covers: story:3-67
#[test]
fn a_change_the_runs_slices_cover_makes_no_blocking_call_and_one_that_needs_a_slice_does() {
    let env = TestEnv::new();

    let results = called_on(
        runtime_with_two_blocking_threads(),
        BlockingPolicy::Threads,
        &env,
        &[
            ("usage.set", monthly_install_limit(1_000)),
            (
                "usage.set",
                json!({
                    "scope": "install",
                    "usage_kind": "requests",
                    "currency": "USD",
                    "period": { "every": "month", "time_zone": "UTC" },
                    "amount": 1_000_000,
                }),
            ),
            ("test.covered", json!({ "run_id": "covered" })),
        ],
    );

    assert_eq!(
        results[2],
        json!({
            "threads_held_throughout": true,
            "another_slice_charged_while_held": null,
            "another_slice_charged_once_let_go": true,
        })
    );
}

/// Several worker threads, as a Product's runtime has: a Run's tasks then
/// change its charging at the same instant.
fn runtime_of_several_threads() -> tokio::runtime::Runtime {
    #[expect(
        clippy::disallowed_methods,
        reason = "the test needs tasks that run at once, which the shared test runtime's one thread does not give (ADR 0060)"
    )]
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}

/// Journals reach the run state in the order of the changes, whichever
/// task made them and wherever: charges of several tasks on several
/// threads at once, on a blocking thread when they need a Slice or find
/// the charging held, where the task runs when its Slices cover them,
/// some journaled in a claim's write. A journal that reached the run
/// state after a later one would leave a Slice's journaled Usage below
/// what was journaled before, or below what the Run used of it.
///
/// Covers: story:3-72
#[test]
fn charges_of_several_tasks_at_once_are_journaled_in_the_order_they_were_made() {
    let env = TestEnv::new();

    let results = called_on(
        runtime_of_several_threads(),
        BlockingPolicy::Threads,
        &env,
        &[
            ("usage.set", monthly_install_limit(1_000)),
            (
                "test.charge_from_tasks",
                json!({ "run_id": "at-once", "tasks": 8, "each": 60 }),
            ),
        ],
    );

    assert_eq!(
        results[1],
        json!({ "charged": 480, "journaled": 480, "went_back": false })
    );
}

/// A write that journals charges as a claim does needs no sync of its
/// own, so a power cut may take it: its file declares how much, and the
/// reclaim charges that much more, at most the Slice of 10, as it cannot
/// tell a process's death from a power cut (ADR 0138's note of #1926). A
/// Limit is then over-charged, never under-charged.
///
/// Covers: story:3-72
#[rstest]
#[case::each_charge_journaled(json!({ "run_id": "orphan", "amount": 4 }), 4)]
#[case::charges_journaled_in_one_write(json!({ "run_id": "orphan", "unjournaled": 4, "amount": 0 }), 10)]
// The write journals each Slice as it is when queued, so it never
// overwrites a later charge's journal with an older figure.
#[case::a_charge_journaled_before_that_write(json!({ "run_id": "orphan", "unjournaled": 2, "amount": 2 }), 10)]
fn the_next_engine_to_open_the_ledger_charges_a_dead_engines_slices_what_its_run_journaled(
    #[case] charge: Value,
    #[case] charged: u64,
) {
    let env = TestEnv::new();
    killed_after(
        &env,
        &[
            ("usage.set", monthly_install_limit(1_000)),
            ("test.charge", charge),
        ],
    );

    let replies = replies(&env, Product::Collie, &[("usage.list", json!({}))]);

    let install = &replies[0]["result"]["limits"][0];
    assert_eq!(
        (&install["charged"], &install["held"], &install["left"]),
        (&json!(charged), &json!(0), &json!(1_000 - charged))
    );
}

/// A Run resumed after its engine died while it held a claim's charges in
/// a write a power cut could take says, in its report, that its Limit may
/// be over-charged by as much as the reclaim charged past its journal (ADR
/// 0138's note of #1926); one whose every charge was synced says nothing.
///
/// Covers: story:3-72
#[rstest]
#[case::charged_in_a_claims_write(json!({ "run_id": "orphan", "unjournaled": 4, "amount": 0 }), json!(["limit_may_be_over_charged"]))]
#[case::each_charge_synced(json!({ "run_id": "orphan", "amount": 4 }), json!([]))]
fn a_run_resumed_after_an_unclean_end_says_its_limit_may_be_over_charged(
    #[case] charge: Value,
    #[case] codes: Value,
) {
    let env = TestEnv::new();
    killed_after(
        &env,
        &[
            ("usage.set", monthly_install_limit(1_000)),
            ("test.charge", charge),
        ],
    );

    let replies = replies(
        &env,
        Product::Collie,
        &[
            ("run.resume", json!({ "run_id": "orphan" })),
            ("run.abort", json!({ "run_id": "orphan" })),
        ],
    );

    let warned: Vec<&Value> = replies[1]["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|warning| &warning["code"])
        .collect();
    assert_eq!(json!(warned), codes, "{replies:?}");
}

/// A monthly install Limit of `amount` millionths of a dollar on paid
/// solves.
fn dollars_a_month(amount: u64) -> Value {
    json!({
        "scope": "install",
        "usage_kind": "paid_solves",
        "currency": "USD",
        "period": { "every": "month", "time_zone": "UTC" },
        "amount": amount,
    })
}

/// Covers: story:3-70
#[rstest]
#[case::an_open_reservation_at_its_worst_case(json!({ "run_id": "solver", "worst": 50_000 }), 50_000)]
#[case::a_settled_one_at_its_cost(json!({ "run_id": "solver", "worst": 50_000, "cost": 30_000 }), 30_000)]
fn the_next_engine_charges_a_dead_engines_reservations_as_its_run_journaled_them(
    #[case] reserve: Value,
    #[case] charged: u64,
) {
    let env = TestEnv::new();
    killed_after(
        &env,
        &[
            ("usage.set", dollars_a_month(1_000_000)),
            ("test.reserve", reserve),
        ],
    );

    let replies = replies(&env, Product::Collie, &[("usage.list", json!({}))]);

    let install = &replies[0]["result"]["limits"][0];
    assert_eq!(
        (&install["charged"], &install["held"]),
        (&json!(charged), &json!(0))
    );
}

/// Covers: story:3-70
#[test]
fn a_worst_case_beyond_what_a_currency_limit_has_left_is_refused_before_it_is_paid() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            ("usage.set", dollars_a_month(40_000)),
            (
                "test.reserve",
                json!({ "run_id": "solver", "worst": 50_000 }),
            ),
            ("run.finish", json!({ "run_id": "solver" })),
            ("usage.list", json!({})),
        ],
    );

    let install = &replies[3]["result"]["limits"][0];
    assert_eq!(
        (
            &replies[1]["result"]["refused"],
            &replies[2]["result"]["stop_reason"]["kind"],
            &install["charged"],
        ),
        (&json!(true), &json!("limit"), &json!(0))
    );
}

/// Covers: story:3-76
#[rstest]
#[case::with_no_currency_limit(None, json!(true))]
#[case::under_a_currency_limit(Some(dollars_a_month(1_000_000)), Value::Null)]
fn a_run_that_can_spend_money_no_limit_caps_is_marked_uncapped(
    #[case] limit: Option<Value>,
    #[case] uncapped: Value,
) {
    let env = TestEnv::new();
    let mut calls: Vec<(&str, Value)> = limit
        .into_iter()
        .map(|limit| ("usage.set", limit))
        .collect();
    calls.push((
        "test.reserve",
        json!({ "run_id": "solver", "worst": 50_000, "cost": 20_000 }),
    ));
    calls.push(("run.finish", json!({ "run_id": "solver" })));

    let replies = replies(&env, Product::Collie, &calls);

    assert_eq!(
        replies.last().unwrap()["result"]["usage"]["uncapped"],
        uncapped
    );
}

/// Covers: story:3-74
#[test]
fn a_live_run_meeting_a_limit_it_cannot_evaluate_stops_at_its_next_safe_point() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            (
                "test.charge",
                json!({ "run_id": "outrun", "amount": 3, "newer_limit_after": 1 }),
            ),
            ("run.finish", json!({ "run_id": "outrun" })),
        ],
    );

    let report = &replies[1]["result"];
    assert_eq!(
        (
            &replies[0]["result"],
            &report["lifecycle"],
            &report["completeness"],
            &report["stop_reason"],
        ),
        (
            &json!({ "charged": 1, "refused": true }),
            &json!("stopped"),
            &json!("incomplete"),
            &json!({
                "kind": "limit_not_understood",
                "scope": { "kind": "galaxy", "id": "milky-way" },
                "usage_kind": "requests",
            }),
        )
    );
}

/// Covers: story:3-75, operation:usage.raise, error:person_required
#[rstest]
#[case::from_an_sdk("sdk_typescript")]
#[case::from_an_agent("mcp")]
fn raising_a_limit_is_refused_to_any_surface_but_a_persons(#[case] surface: &str) {
    let env = TestEnv::new();

    let replies = replies_from(
        &env,
        Product::Collie,
        surface,
        &[
            ("usage.set", monthly_install_limit(5)),
            ("usage.raise", monthly_install_limit(10)),
            ("usage.list", json!({})),
        ],
    );

    assert_eq!(
        (
            &replies[1]["error"]["data"]["code"],
            &replies[2]["result"]["limits"][0]["limit"]["amount"],
        ),
        (&json!("person_required"), &json!(5))
    );
}

/// Covers: story:3-75, operation:usage.raise
#[test]
fn a_person_raises_a_limit_in_place_keeping_the_usage_counted_against_it() {
    let env = TestEnv::new();

    let replies = replies_from(
        &env,
        Product::Collie,
        "cli",
        &[
            ("usage.set", monthly_install_limit(2)),
            ("test.charge", json!({ "run_id": "first", "amount": 2 })),
            ("run.finish", json!({ "run_id": "first" })),
            ("usage.raise", monthly_install_limit(10)),
            ("usage.list", json!({})),
        ],
    );

    let limits = &replies[4]["result"]["limits"];
    assert_eq!(
        (
            limits.as_array().map(Vec::len),
            &limits[0]["limit"]["amount"],
            &limits[0]["limit"]["set_by"],
            &limits[0]["charged"],
        ),
        (Some(1), &json!(10), &json!("person"), &json!(2))
    );
}

/// Covers: story:3-75
#[test]
fn a_client_lowering_a_persons_limit_adds_its_own_row_and_the_persons_stays() {
    let env = TestEnv::new();

    let replies = replies_from(
        &env,
        Product::Collie,
        "cli",
        &[
            ("usage.raise", monthly_install_limit(10)),
            ("usage.set", monthly_install_limit(4)),
            ("usage.list", json!({})),
        ],
    );

    let limits = replies[2]["result"]["limits"].as_array().unwrap();
    let rows: Vec<(&Value, &Value)> = limits
        .iter()
        .map(|usage| (&usage["limit"]["set_by"], &usage["limit"]["amount"]))
        .collect();
    assert_eq!(
        rows,
        [
            (&json!("person"), &json!(10)),
            (&json!("client"), &json!(4))
        ]
    );
}

/// Covers: story:3-75, operation:usage.set
#[test]
fn a_client_sets_limits_only_on_its_own_work() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[(
            "usage.set",
            json!({
                "scope": "connection",
                "scope_id": "another-clients-connection",
                "usage_kind": "requests",
                "amount": 1,
            }),
        )],
    );

    assert_eq!(replies[0]["error"]["code"], json!(-32602));
}

/// Covers: story:3-77
#[test]
fn an_engine_writes_the_install_limit_of_its_managed_operator_policy_when_it_starts() {
    let env = TestEnv::new();
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("operator-policy.toml"),
        "version = \"0.1\"\n\n[managed]\ninstall_limits = [{ usage_kind = \"requests\", amount = 40 }]\n",
    )
    .unwrap();

    let replies = replies(&env, Product::Collie, &[("usage.list", json!({}))]);

    let install = &replies[0]["result"]["limits"][0];
    assert_eq!(
        (
            &install["limit"]["amount"],
            &install["limit"]["set_by"],
            &install["managed_major"],
        ),
        (&json!(40), &json!("managed_policy"), &json!("1"))
    );
}
