//! A Run's local addresses through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADR 0044): `run.start` refuses
//! `egress.local_addresses` that yield no address this host can send from
//! with `local_address_unavailable`, and a Run that starts with them names
//! them and both block sizes in its report's settings.

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::run::call;
use super::{attached_on, engine_with, hello};

/// Starts the Run `local` with `settings` and finishes it when it starts;
/// answers the start's and the finish's replies.
fn start_and_finish(env: &TestEnv, settings: &Value) -> (Value, Value) {
    let params = json!({ "run_id": "local", "settings": settings });
    attached_on(
        &engine_with(Product::Collie, None),
        env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let started = call(&mut client, 1, "run.start", params).await;
            let finished = call(&mut client, 2, "run.finish", json!({ "run_id": "local" })).await;
            (started, finished)
        },
    )
}

/// A documentation prefix and address, which no host holds.
/// Covers: story:4-46, error:local_address_unavailable, setting:egress.local_addresses
#[rstest]
#[case::a_prefix_with_no_usable_address(json!("2001:db8:ffff::/64"), "2001:db8:ffff::/64", "prefix_unusable")]
#[case::an_address_the_host_does_not_hold(json!(["127.0.0.1", "192.0.2.77"]), "192.0.2.77", "not_held")]
fn local_addresses_that_yield_no_usable_address_refuse_the_run(
    #[case] local_addresses: Value,
    #[case] entry: &str,
    #[case] failure: &str,
) {
    let env = TestEnv::new();

    let (started, finished) = start_and_finish(
        &env,
        &json!({ "egress": { "local_addresses": local_addresses } }),
    );

    assert_eq!(
        (
            &started["error"]["data"]["code"],
            [
                &started["error"]["data"]["entry"],
                &started["error"]["data"]["failure"],
                &started["error"]["data"]["setting"],
            ],
            &finished["error"]["data"]["code"],
        ),
        (
            &json!("local_address_unavailable"),
            [
                &json!(entry),
                &json!(failure),
                &json!("egress.local_addresses")
            ],
            &json!("run_not_found"),
        )
    );
}

/// Covers: story:4-44, story:4-47, setting:egress.local_addresses, setting:egress.rotation_block.ipv4, setting:egress.rotation_block.ipv6
#[test]
fn a_run_with_local_addresses_records_them_and_its_block_sizes_in_its_report() {
    let env = TestEnv::new();

    let (started, finished) = start_and_finish(
        &env,
        &json!({ "egress": { "local_addresses": ["127.0.0.1"], "rotation_block": { "ipv4": 16 } } }),
    );
    let recorded: Vec<(&Value, &Value, &Value)> = finished["result"]["settings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|setting| {
            setting["key"].as_str().is_some_and(|key| {
                key == "egress.local_addresses" || key.starts_with("egress.rotation_block.")
            })
        })
        .map(|setting| (&setting["key"], &setting["value"], &setting["layer"]))
        .collect();

    assert_eq!(started["error"], Value::Null, "{started}");
    assert_eq!(
        recorded,
        [
            (
                &json!("egress.local_addresses"),
                &json!(["127.0.0.1"]),
                &json!("run")
            ),
            (
                &json!("egress.rotation_block.ipv4"),
                &json!(16),
                &json!("run")
            ),
            (
                &json!("egress.rotation_block.ipv6"),
                &json!(64),
                &json!("default")
            ),
        ]
    );
}
