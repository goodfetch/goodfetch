//! `run_state.durability` through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADRs 0002 and 0138): a Run at a
//! level below `full` commits its run file as the level says, and its run
//! report and `config.explain` name the setting as one that weakens a
//! guarantee; a Run paused at `off` resumes, and one whose run file a crash
//! left at `off` is discarded and starts again.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's settings files themselves; its SQL statements are its own, on no Shared path (ADR 0140)"
)]

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::run::call;
use super::{attached_on, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};

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
    input: None,
    bound: None,
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

/// Each Durability level below `full` commits as it says, and its run
/// report's `weakened` names the sync its preset sets, and `off` itself
/// (ADRs 0138 and 0142): `fast` with `synchronous=NORMAL`, `off` with `OFF`.
/// `periodic` commits on either of its writer's connections as its interval
/// says, which no one pragma shows, so its case asserts the report only.
/// Covers: story:3-27, setting:run_state.durability, setting:run_state.batch.sync
#[rstest::rstest]
#[case::fast(
    "fast",
    Some(1),
    json!([{
        "key": "run_state.batch.sync",
        "value": "never",
        "layer": "user_configuration",
        "preset": { "key": "run_state.durability", "value": "fast" },
    }])
)]
#[case::periodic(
    "periodic",
    None,
    json!([{
        "key": "run_state.batch.sync",
        "value": "interval",
        "layer": "user_configuration",
        "preset": { "key": "run_state.durability", "value": "periodic" },
    }])
)]
#[case::off(
    "off",
    Some(0),
    json!([
        {
            "key": "run_state.batch.sync",
            "value": "off",
            "layer": "user_configuration",
            "preset": { "key": "run_state.durability", "value": "off" },
        },
        {
            "key": "run_state.durability",
            "value": "off",
            "layer": "user_configuration",
        },
    ])
)]
fn a_run_below_full_commits_as_its_level_says_and_its_report_says_so(
    #[case] level: &str,
    #[case] synchronous: Option<i64>,
    #[case] weakened: Value,
) {
    let env = TestEnv::new();
    configure(
        &env,
        &format!("version = \"0.1\"\n\n[settings]\nrun_state.durability = \"{level}\"\n"),
    );

    let (read, finished) = attached_on(
        &engine_with(Product::Collie, Some(reading_synchronous())),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "quick" })).await;
            let read = call(
                &mut client,
                2,
                "test.synchronous",
                json!({ "run_id": "quick" }),
            )
            .await;
            let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "quick" })).await;
            (read, finished)
        },
    );

    assert_eq!(
        (
            synchronous.map(|_| read["result"].clone()),
            &finished["result"]["weakened"]
        ),
        (synchronous.map(|synchronous| json!(synchronous)), &weakened)
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

/// `config.explain` labels the sync each level below `full` sets as one that
/// weakens a guarantee and names its preset as its source, and labels the
/// level itself only at `off` (ADRs 0138 and 0142).
/// Covers: story:3-27, setting:run_state.batch.sync
#[rstest::rstest]
#[case::fast("fast", "never", false)]
#[case::periodic("periodic", "interval", false)]
#[case::off("off", "off", true)]
fn config_explain_labels_each_level_below_full_as_weakening(
    #[case] level: &str,
    #[case] sync: &str,
    #[case] weakens: bool,
) {
    let env = TestEnv::new();
    configure(
        &env,
        &format!("version = \"0.1\"\n\n[settings]\nrun_state.durability = \"{level}\"\n"),
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
                json!({ "keys": ["run_state.durability", "run_state.batch.sync"] }),
            )
            .await
        },
    );

    assert_eq!(
        explained["result"]["settings"],
        json!([
            {
                "key": "run_state.batch.sync",
                "products": ["collie", "foxhound"],
                "applied": true,
                "value": sync,
                "layer": "user_configuration",
                "preset": { "key": "run_state.durability", "value": level },
                "weakens": true,
                "user_settable": true,
            },
            {
                "key": "run_state.durability",
                "products": ["collie", "foxhound"],
                "applied": true,
                "value": level,
                "layer": "user_configuration",
                "weakens": weakens,
                "user_settable": true,
            },
        ])
    );
}

/// The run file of the Run `run_id` in `env`'s home.
fn run_file(env: &TestEnv, run_id: &str) -> std::path::PathBuf {
    env.home()
        .join("1")
        .join("runs")
        .join(run_id)
        .join("run.sqlite")
}

/// Configures `env` at `off`, starts the Run `run_id` and pauses it, its
/// clean stop.
fn paused_at_off(env: &TestEnv, run_id: &str) -> Value {
    let run_id = run_id.to_owned();
    configure(
        env,
        "version = \"0.1\"\n\n[settings]\nrun_state.durability = \"off\"\n",
    );
    attached_on(
        &engine_with(Product::Collie, None),
        env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": &run_id })).await;
            call(&mut client, 2, "run.pause", json!({ "run_id": &run_id })).await
        },
    )
}

/// A Run paused at `off`, a clean stop, leaves no "unclean" mark, so it
/// resumes in another engine and goes on to its end (ADR 0138).
/// Covers: story:3-27, operation:run.pause, operation:run.resume
#[test]
fn a_run_paused_at_off_resumes_in_another_engine() {
    let env = TestEnv::new();
    let paused = paused_at_off(&env, "paused");

    let (resumed, finished) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let resumed = call(&mut client, 1, "run.resume", json!({ "run_id": "paused" })).await;
            let finished = call(&mut client, 2, "run.finish", json!({ "run_id": "paused" })).await;
            (resumed, finished)
        },
    );

    assert_eq!(
        (
            &paused["result"]["lifecycle"],
            &resumed["result"]["lifecycle"],
            &finished["result"]["lifecycle"],
        ),
        (&json!("stopped"), &json!("running"), &json!("finished"))
    );
}

/// A run file a crash left at `off`, its "unclean" mark still set, is
/// discarded rather than read (ADR 0138): a resume finds no Run, and a
/// start starts it again from nothing.
/// Covers: story:3-27, operation:run.start, operation:run.resume
#[test]
fn a_run_file_a_crash_left_at_off_is_discarded_and_the_run_starts_again() {
    let env = TestEnv::new();
    paused_at_off(&env, "crashed");
    // What a crash leaves: the mark its writer set as it opened the file,
    // which no clean stop cleared.
    rusqlite::Connection::open(run_file(&env, "crashed"))
        .unwrap()
        .execute("INSERT INTO unclean (id) VALUES (1)", [])
        .unwrap();

    let (resumed, started) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            let resumed = call(&mut client, 1, "run.resume", json!({ "run_id": "crashed" })).await;
            let started = call(&mut client, 2, "run.start", json!({ "run_id": "crashed" })).await;
            (resumed, started)
        },
    );

    assert_eq!(
        (
            &resumed["error"]["data"]["code"],
            &started["result"]["started_as"],
        ),
        (&json!("run_not_found"), &json!("new")),
        "{started}"
    );
}
