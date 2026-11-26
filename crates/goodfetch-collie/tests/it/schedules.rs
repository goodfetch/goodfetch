//! The `schedule` commands through Collie's executable (spec 589, ADRs
//! 0024 and 0036): the Schedule store is the Daemon's, so an Attached
//! engine forwards each `schedule.*` call to the major's Daemon, starting
//! it when none runs. A Schedule created by one Client is read by another
//! and survives a `daemon restart`.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, Hello};
use goodfetch_test_support::{CliOutput, TestEnv, cli};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::daemon_commands::assert_transcript;
use crate::engine_choice::{On, daemon_dispatched};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// A Schedule that starts the Recipe `shop-prices` every hour in Paris.
const DEFINITION: &str = r#"{"id":"nightly-prices","target":{"recipe":"shop-prices"},"trigger":{"every":3600000},"time_zone":"Europe/Paris"}"#;

/// Runs `arguments` under `--json` and `flags`, checks that its stdout is
/// one document of `definition`, and answers its output, without the
/// engine's dispatch records, and that document.
fn checked(
    env: &TestEnv,
    arguments: &[&str],
    flags: &[&str],
    definition: &str,
) -> (CliOutput, Value) {
    let checked = cli(env, COLLIE)
        .args(arguments)
        .args(flags)
        .arg("--json")
        .json(definition)
        .unwrap();
    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    (
        checked.output.without_dispatch_records(),
        checked.documents[0].clone(),
    )
}

/// `schedule create`, `list`, `get` and `delete` keep the Schedule in the
/// Daemon's store, which the first command starts, whether the command
/// runs on an Attached engine or on the Daemon; both print the same.
///
/// Covers: story:10-1, story:10-13, command:schedule.create, command:schedule.list, command:schedule.get, command:schedule.delete
#[rstest::rstest]
#[case::attached(On::Attached)]
#[case::daemon(On::Daemon)]
fn schedule_commands_keep_the_schedule_in_the_daemon_they_start(#[case] on: On) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let flags = [on.flag()];

    let create: &[&str] = &["schedule", "create", "--definition", DEFINITION];
    let list: &[&str] = &["schedule", "list"];
    let get: &[&str] = &["schedule", "get", "--id", "nightly-prices"];
    let delete: &[&str] = &["schedule", "delete", "nightly-prices"];
    let (created, _) = checked(&env, create, &flags, "ScheduleCreated");
    let (listed, schedules) = checked(&env, list, &flags, "ScheduleList");
    let (got, info) = checked(&env, get, &flags, "ScheduleInfo");
    let (deleted, _) = checked(&env, delete, &flags, "ScheduleDeleted");

    assert_eq!(
        (
            schedules["schedules"][0]["created_at"] == info["created_at"],
            [
                "schedule.create",
                "schedule.list",
                "schedule.get",
                "schedule.delete"
            ]
            .map(|method| daemon_dispatched(&env, method)),
        ),
        (true, [true; 4]),
    );
    assert_transcript(
        "schedule_create_list_get_delete",
        &[
            (create, &created),
            (list, &listed),
            (get, &got),
            (delete, &deleted),
        ],
    );
}

/// A Schedule the CLI created is read by another Client, the TypeScript
/// SDK's Connection to an Attached engine, and is still there, as it was,
/// once the Daemon restarted.
///
/// Covers: story:10-1, story:10-13, command:schedule.create, command:schedule.get, command:daemon.restart
#[test]
fn a_schedule_one_client_created_is_read_by_another_and_survives_a_daemon_restart() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let create: &[&str] = &["schedule", "create", "--definition", DEFINITION];
    let get: &[&str] = &["schedule", "get", "--id", "nightly-prices"];

    checked(&env, create, &[], "ScheduleCreated");
    let mut engine = Attached::start(&env, COLLIE);
    engine.open(&Hello::needing(Product::Collie).surface(Surface::SdkTypescript));
    let read = engine.ask(1, "schedule.get", &json!({ "id": "nightly-prices" }));
    drop(engine);
    let restart = cli(&env, COLLIE)
        .args(["daemon", "restart", "--json"])
        .run()
        .unwrap();
    let (_, after) = checked(&env, get, &["--daemon"], "ScheduleInfo");

    assert_eq!(
        (
            restart.exit,
            &read["result"],
            read["result"]["version"].clone()
        ),
        (Some(0), &after, json!(1)),
        "{read}\n{restart}"
    );
}
