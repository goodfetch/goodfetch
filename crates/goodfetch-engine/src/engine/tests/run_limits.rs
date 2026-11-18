//! The Limits a program gives at a Run's start (ADRs 0041, 0080 and
//! 0102), through the engine's public interface over the in-memory network:
//! each holds the Run on its own `run` scope, set by the Client; a start
//! that names the Run again lowers a Limit and never raises one, a value
//! equal to the one given when the Run began changing nothing, so a
//! person's raise stands; and a Usage kind or currency `usage.set` refuses
//! is refused the same way.

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::usage::{replies, replies_from};

/// `run.start` of the Run `run_id` with a Limit of `amount` requests.
fn start_with(run_id: &str, amount: u64) -> (&'static str, Value) {
    (
        "run.start",
        json!({
            "run_id": run_id,
            "limits": [{ "usage_kind": "requests", "amount": amount }],
        }),
    )
}

/// Each `run`-scope Limit of a `usage.list` reply, as who set it and its
/// amount.
fn run_limits(listed: &Value) -> Vec<(Value, Value)> {
    listed["result"]["limits"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|usage| usage["limit"]["scope"]["kind"] == "run")
        .map(|usage| {
            (
                usage["limit"]["set_by"].clone(),
                usage["limit"]["amount"].clone(),
            )
        })
        .collect()
}

/// Covers: error:limit_reached
#[test]
fn a_limit_given_at_a_runs_start_refuses_the_call_that_finds_it_used_up_and_stops_the_run() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            start_with("capped", 2),
            (
                "test.charge_live",
                json!({ "run_id": "capped", "amount": 3 }),
            ),
            ("run.finish", json!({ "run_id": "capped" })),
            ("usage.list", json!({})),
        ],
    );

    let refused = &replies[1]["error"]["data"];
    let report = &replies[2]["result"];
    let listed = &replies[3]["result"]["limits"][0];
    assert_eq!(
        (
            &refused["code"],
            &refused["scope"],
            &report["lifecycle"],
            &report["stop_reason"]["kind"],
            &report["stop_reason"]["scope"],
            &listed["limit"],
            &listed["charged"],
        ),
        (
            &json!("limit_reached"),
            &json!({ "kind": "run", "id": "capped" }),
            &json!("stopped"),
            &json!("limit"),
            &json!({ "kind": "run", "id": "capped" }),
            &json!({
                "scope": { "kind": "run", "id": "capped" },
                "usage_kind": "requests",
                "amount": 2,
                "must_understand": true,
                "set_by": "client",
            }),
            &json!(2),
        )
    );
}

#[test]
fn a_start_naming_the_run_again_with_the_value_it_began_with_keeps_a_persons_raise() {
    let env = TestEnv::new();

    let replies = replies_from(
        &env,
        Product::Collie,
        "cli",
        &[
            start_with("raised", 5),
            (
                "usage.raise",
                json!({ "scope": "run", "scope_id": "raised", "usage_kind": "requests", "amount": 10 }),
            ),
            start_with("raised", 5),
            ("usage.list", json!({})),
        ],
    );

    assert_eq!(
        (&replies[2]["result"]["started_as"], run_limits(&replies[3])),
        (&json!("existing"), vec![(json!("person"), json!(10))])
    );
}

#[test]
fn a_start_naming_the_run_again_with_a_lower_value_lowers_its_limit() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            start_with("lowered", 5),
            start_with("lowered", 3),
            start_with("lowered", 5),
            ("usage.list", json!({})),
        ],
    );

    assert_eq!(
        (
            &replies[1]["result"]["started_as"],
            &replies[2]["result"]["started_as"],
            run_limits(&replies[3]),
        ),
        (
            &json!("existing"),
            &json!("existing"),
            vec![(json!("client"), json!(5)), (json!("client"), json!(3))]
        )
    );
}

/// Covers: error:limit_raise_refused
#[test]
fn a_start_naming_the_run_again_above_its_limit_is_refused_with_limit_raise_refused() {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[
            start_with("held", 5),
            start_with("held", 8),
            ("usage.list", json!({})),
        ],
    );

    let data = &replies[1]["error"]["data"];
    assert_eq!(
        (
            &data["code"],
            &data["usage_kind"],
            &data["given"],
            &data["standing"],
            run_limits(&replies[2]),
        ),
        (
            &json!("limit_raise_refused"),
            &json!("requests"),
            &json!(8),
            &json!(5),
            vec![(json!("client"), json!(5))]
        )
    );
}

/// The Error code and JSON-RPC code of the reply to a start of a new Run
/// given `limits`.
fn start_refusal(limits: &Value) -> (Value, Value) {
    let env = TestEnv::new();

    let replies = replies(
        &env,
        Product::Collie,
        &[(
            "run.start",
            json!({ "run_id": "refused", "limits": limits }),
        )],
    );

    let error = &replies[0]["error"];
    (error["data"]["code"].clone(), error["code"].clone())
}

/// Covers: error:not_in_product
#[test]
fn a_collie_start_refuses_a_limit_on_a_usage_kind_only_foxhound_carries() {
    assert_eq!(
        start_refusal(
            &json!([{ "usage_kind": "paid_fetch", "currency": "USD", "amount": 5_000_000 }])
        )
        .0,
        json!("not_in_product")
    );
}

#[test]
fn a_start_refuses_a_malformed_currency_as_invalid_params() {
    assert_eq!(
        start_refusal(&json!([{ "usage_kind": "paid_solves", "currency": "usd", "amount": 1 }])).1,
        json!(-32602)
    );
}

#[test]
fn a_start_refuses_two_limits_on_one_usage_kind_and_currency_as_invalid_params() {
    assert_eq!(
        start_refusal(&json!([
            { "usage_kind": "requests", "amount": 1 },
            { "usage_kind": "requests", "amount": 2 },
        ]))
        .1,
        json!(-32602)
    );
}
