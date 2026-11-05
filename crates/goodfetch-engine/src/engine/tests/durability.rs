//! `run_state.durability` through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADR 0002): a Run in `fast` mode
//! commits its run file with `synchronous=NORMAL`, and its run report and
//! `config.explain` name the setting as one that weakens a guarantee.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's settings files themselves"
)]

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::run::call;
use super::{attached_on, engine_with, hello};
use crate::composition::{CallContext, CallError, Contribution, Handler};

/// Reads the `synchronous` pragma of a live Run's run file, on its writer.
const SYNCHRONOUS: Operation = Operation {
    name: "test.synchronous",
    description: "Reads how a live Run's run file commits.",
    class: OperationClass::Read,
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

fn reading_synchronous() -> Contribution {
    Contribution::new("test-durability").with_handler(Handler::new(
        &SYNCHRONOUS,
        |params: Value, context: CallContext| async move {
            let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
            let run = context.runs().live(run_id).await.unwrap();
            let synchronous: i64 = run
                .state()
                .write(|connection| {
                    connection.pragma_query_value(None, "synchronous", |row| row.get(0))
                })
                .await
                .unwrap();
            Ok::<Value, CallError>(json!(synchronous))
        },
    ))
}

/// Writes `text` as the test home's user configuration.
fn configure(env: &TestEnv, text: &str) {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("configuration.toml"), text).unwrap();
}

/// Covers: story:3-27, setting:run_state.durability
#[test]
fn a_run_in_fast_durability_commits_with_normal_synchronous_and_its_report_says_so() {
    let env = TestEnv::new();
    configure(
        &env,
        "version = \"0.1\"\n\n[settings]\nrun_state.durability = \"fast\"\n",
    );

    let (synchronous, finished) = attached_on(
        &engine_with(Product::Collie, Some(reading_synchronous())),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "quick" })).await;
            let synchronous = call(
                &mut client,
                2,
                "test.synchronous",
                json!({ "run_id": "quick" }),
            )
            .await;
            let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "quick" })).await;
            (synchronous, finished)
        },
    );

    assert_eq!(
        (&synchronous["result"], &finished["result"]["weakened"]),
        (
            &json!(1),
            &json!([{
                "key": "run_state.durability",
                "value": "fast",
                "layer": "user_configuration",
            }])
        )
    );
}

/// A Run that sets nothing commits with `synchronous=FULL` and names no
/// weakening setting.
#[test]
fn a_run_in_default_durability_commits_with_full_synchronous() {
    let env = TestEnv::new();

    let (synchronous, finished) = attached_on(
        &engine_with(Product::Collie, Some(reading_synchronous())),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "safe" })).await;
            let synchronous = call(
                &mut client,
                2,
                "test.synchronous",
                json!({ "run_id": "safe" }),
            )
            .await;
            let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "safe" })).await;
            (synchronous, finished)
        },
    );

    assert_eq!(
        (&synchronous["result"], &finished["result"]["weakened"]),
        (&json!(2), &Value::Null)
    );
}

/// Covers: story:3-27
#[test]
fn config_explain_labels_fast_durability_as_weakening() {
    let env = TestEnv::new();
    configure(
        &env,
        "version = \"0.1\"\n\n[settings]\nrun_state.durability = \"fast\"\n",
    );

    let explained = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(
                &mut client,
                1,
                "config.explain",
                json!({ "keys": ["run_state.durability"] }),
            )
            .await
        },
    );

    assert_eq!(
        explained["result"]["settings"],
        json!([{
            "key": "run_state.durability",
            "products": ["collie", "foxhound"],
            "applied": true,
            "value": "fast",
            "layer": "user_configuration",
            "weakens": true,
            "user_settable": true,
        }])
    );
}
