//! The `schedule` commands through Collie's executable (spec 589, ADRs
//! 0024 and 0036): the Schedule store is the Daemon's, so an Attached
//! engine forwards each `schedule.*` call to the major's Daemon, starting
//! it when none runs. A Schedule created by one Client is read by another
//! and survives a `daemon restart`.
#![expect(
    clippy::doc_markdown,
    reason = "Covers: declarations carry raw Feature IDs (ADR 0072)"
)]

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

/// `schedule create` accepts a cron trigger and an anchored `every` trigger,
/// and `schedule preview` prints plain-words descriptions and next
/// Occurrences from `now` or from `after`, matching one snapshot in both
/// `attached` and `daemon` cases with `--json` validated against its schema.
///
/// Covers: story:10-9, story:10-15, story:10-16, story:10-17, command:schedule.create, command:schedule.preview, operation:schedule.preview
#[rstest::rstest]
#[case::attached(On::Attached)]
#[case::daemon(On::Daemon)]
fn schedule_create_accepts_cron_and_anchored_every_and_preview_matches_one_snapshot(
    #[case] on: On,
) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let flags = [on.flag()];

    let cron_def = r#"{"id":"weekday-prices","target":{"recipe":"shop-prices"},"trigger":{"cron":"0 9 * * MON-FRI"},"time_zone":"Europe/Paris"}"#;
    let anchored_def = r#"{"id":"anchored-prices","target":{"recipe":"shop-prices"},"trigger":{"every":900000,"anchor":"2026-03-29T00:00:00Z"},"time_zone":"Europe/Paris"}"#;
    let create_cron: &[&str] = &["schedule", "create", "--definition", cron_def];
    let create_anchored: &[&str] = &["schedule", "create", "--definition", anchored_def];
    let preview_after: &[&str] = &[
        "schedule",
        "preview",
        "--trigger",
        r#"{"cron":"30 2 * * *"}"#,
        "--time-zone",
        "Europe/Paris",
        "--after",
        "2026-03-29T00:00:00Z",
        "--count",
        "2",
    ];
    let preview_now: &[&str] = &[
        "schedule",
        "preview",
        "--trigger",
        r#"{"every":900000}"#,
        "--time-zone",
        "Europe/Paris",
    ];

    let (created_cron, _) = checked(&env, create_cron, &flags, "ScheduleCreated");
    let (created_anchored, _) = checked(&env, create_anchored, &flags, "ScheduleCreated");
    let (previewed_json, preview_doc) = checked(&env, preview_after, &flags, "SchedulePreview");
    let previewed_human = cli(&env, COLLIE)
        .args(preview_after)
        .args(flags)
        .run()
        .unwrap();
    assert_eq!(previewed_human.exit, Some(0), "{previewed_human}");
    let (_, from_now) = checked(&env, preview_now, &flags, "SchedulePreview");

    assert_eq!(
        preview_doc["occurrences"][0]["at"],
        json!("2026-03-29T01:00:00Z")
    );
    assert_eq!(
        from_now["occurrences"].as_array().map(Vec::len),
        Some(5),
        "default preview count is 5: {from_now}"
    );
    assert_eq!(
        from_now["description"]["english"],
        json!("Every 15 minutes.")
    );
    assert_transcript(
        "schedule_triggers_and_preview",
        &[
            (create_cron, &created_cron),
            (create_anchored, &created_anchored),
            (preview_after, &previewed_json),
            (
                &[
                    "schedule",
                    "preview",
                    "--human",
                    "--trigger",
                    r#"{"cron":"30 2 * * *"}"#,
                ],
                &previewed_human.without_dispatch_records(),
            ),
        ],
    );
}

/// A malformed trigger raises `schedule_trigger_invalid` naming the
/// expression, character position and stable `reason`, and exits with its
/// declared ADR 0069 exit code (`3`, `ProgramError`) through the binary.
///
/// Covers: story:10-15, error:schedule_trigger_invalid, command:schedule.preview
#[test]
fn malformed_schedule_trigger_reaches_its_exit_code_through_the_binary() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let failed = cli(&env, COLLIE)
        .args([
            "schedule",
            "preview",
            "--trigger",
            r#"{"cron":"*/0 * * * *"}"#,
            "--json",
        ])
        .json("CommandFailed")
        .unwrap();

    assert_eq!(failed.output.exit, Some(3), "{}", failed.output);
    let error = &failed.documents[0]["error"];
    assert_eq!(error["code"], json!("schedule_trigger_invalid"));
    assert_eq!(error["retryable"], json!(false));
    assert_eq!(error["expression"], json!("*/0 * * * *"));
    assert_eq!(error["position"], json!(1));
    assert_eq!(error["reason"], json!("bad_step"));
}
