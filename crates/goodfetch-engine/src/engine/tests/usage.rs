//! Limits across runs through the engine's public interface over the
//! in-memory network (spec #213's Seam 3): `usage.set` and `usage.list`; a
//! Run that charges Usage against its Slices and stops at its next safe
//! point once a Limit has nothing left, `stopped` and `incomplete` for that
//! Limit; a new Run refused with `limit_reached` or `limit_not_understood`;
//! a value only Foxhound carries refused by a Collie engine; and the Slices
//! of a dead engine's Run reclaimed by the next engine that opens the Usage
//! ledger, from what the Run journaled, open reservations at their worst
//! case. A test's own operation, `test.charge`, starts a Run that charges
//! `requests` one at a time, as a crawl does before each request; another,
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
use crate::composition::{CallContext, CallError, Contribution, Handler};
use crate::ledger::Ledger;
use crate::runs::{ChargeError, EndedBy, Start, Started};
use crate::transport::{self, Network};

/// Starts a Run of its own and charges `amount` requests, one at a time,
/// until one is refused.
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
                for _ in 0..params["amount"].as_u64().unwrap() {
                    if params["newer_limit_after"].as_u64() == Some(charged) {
                        Ledger::at(context.runs().home().usage_ledger(), None)
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
                Ok::<Value, CallError>(json!({ "charged": charged, "refused": refused }))
            },
        ))
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
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let engine = engine_with(Product::Collie, Some(charging()));
        let serving = engine.run_attached(Network::InMemory(network), dependencies(env));
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            for (id, (method, params)) in (1..).zip(calls) {
                let reply = call(&mut client, id, method, params.clone()).await;
                assert!(reply.get("result").is_some(), "{reply}");
            }
        };
        tokio::select! {
            served = serving => panic!("the engine ended: {served:?}"),
            () = talk => {}
        }
    });
    drop(runtime);
}

/// Covers: story:3-72
#[test]
fn the_next_engine_to_open_the_ledger_charges_a_dead_engines_slices_what_its_run_journaled() {
    let env = TestEnv::new();
    killed_after(
        &env,
        &[
            ("usage.set", monthly_install_limit(1_000)),
            ("test.charge", json!({ "run_id": "orphan", "amount": 4 })),
        ],
    );

    let replies = replies(&env, Product::Collie, &[("usage.list", json!({}))]);

    let install = &replies[0]["result"]["limits"][0];
    assert_eq!(
        (&install["charged"], &install["held"], &install["left"]),
        (&json!(4), &json!(0), &json!(996))
    );
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
