//! A Connection's Schedules through the Rust SDK (spec 589, ADR 0099), the
//! `schedules-crud` client conformance case in `linked` mode:
//! `schedules().create` answers a Schedule under the ID given, or one it
//! made when none was, `list` lists both, `get` answers the handle of a
//! held ID and fails an unknown one with `schedule_not_found`, and a
//! handle's `info()` reads what the store holds and its `delete()` answers
//! whether it deleted it. The blocking API reaches the same store.

use std::time::Duration;

use goodfetch_collie::{
    Collie, CreatedSchedule, Error, ErrorCode, EveryTrigger, RecipeTarget, Schedule,
    ScheduleDefinition, ScheduleId, ScheduleInfo, ScheduleState, ScheduleTarget, ScheduleTrigger,
    blocking,
};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::schedule::ScheduleNotFoundDetails;
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};

use super::options;

/// A definition of `id`, or of none, that starts the Recipe `shop-prices`
/// every `every`.
fn definition(id: Option<&str>, every: Duration, time_zone: Option<&str>) -> ScheduleDefinition {
    ScheduleDefinition {
        id: id.map(|id| ScheduleId::parse(id).unwrap()),
        target: ScheduleTarget::Recipe(RecipeTarget {
            recipe: "shop-prices".to_owned(),
            inputs: None,
            settings: None,
            timeout: None,
        }),
        trigger: ScheduleTrigger::Every(EveryTrigger {
            every: u64::try_from(every.as_millis()).unwrap(),
            anchor: None,
        }),
        time_zone: time_zone.map(str::to_owned),
    }
}

/// What the `schedules-crud` steps answered.
struct Answered {
    named: CreatedSchedule,
    made: CreatedSchedule,
    listed: Vec<ScheduleId>,
    got: Schedule,
    info: ScheduleInfo,
    unknown: Error,
    deleted: [bool; 2],
    gone: ErrorCode,
}

/// Runs the case's steps on a Connection to `env`'s engine: creates
/// `nightly`, then one with no ID, lists them, gets `nightly` and reads it,
/// gets `unknown_id`, and deletes `nightly` twice before reading it again.
async fn schedules_crud(env: &TestEnv, nightly: &ScheduleId, unknown_id: &ScheduleId) -> Answered {
    let collie = Collie::connect(options(env)).await.unwrap();
    let schedules = collie.schedules();
    let named = schedules
        .create(definition(
            Some(nightly.as_str()),
            Duration::from_secs(3_600),
            Some("Europe/Paris"),
        ))
        .await
        .unwrap();
    let made = schedules
        .create(definition(None, Duration::from_secs(60), None))
        .timeout(Duration::from_secs(10))
        .await
        .unwrap();
    let mut listed: Vec<_> = schedules
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|info| info.schedule_id)
        .collect();
    listed.sort();
    let got = schedules.get(nightly).no_timeout().await.unwrap();
    let info = got.info().await.unwrap().into_inner();
    let unknown = schedules.get(unknown_id).await.unwrap_err();
    let deleted = named.schedule().delete().await.unwrap();
    let again = got.delete().await.unwrap();
    let gone = got.info().await.unwrap_err().code();
    collie.close().await.unwrap();
    Answered {
        named,
        made,
        listed,
        got,
        info,
        unknown,
        deleted: [deleted, again],
        gone,
    }
}

/// Covers: story:10-2
#[test]
fn a_connections_schedules_create_list_read_and_delete_a_schedule() {
    let mut case =
        ClientCaseRun::start(case_logs().as_deref(), "schedules-crud", RustMode::Linked).unwrap();
    let env = TestEnv::new();
    let nightly = ScheduleId::parse("nightly-prices").unwrap();
    let unknown_id = ScheduleId::parse("never-created").unwrap();
    let Answered {
        named,
        made,
        listed,
        got,
        info,
        unknown,
        deleted,
        gone,
    } = real_time_runtime().block_on(schedules_crud(&env, &nightly, &unknown_id));

    case.check(
        "create answers the Schedule under the ID given, with no warnings",
        || {
            assert_eq!(
                (named.schedule().id(), named.warnings().len()),
                (&nightly, 0)
            );
        },
    );
    case.check("create makes an ID when the definition names none", || {
        let made = made.schedule().id().as_str();
        assert!(
            made.len() == 27
                && made.starts_with('s')
                && made[1..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase()),
            "{made}"
        );
    });
    case.check("list holds both Schedules", || {
        let mut expected = vec![made.schedule().id().clone(), nightly.clone()];
        expected.sort();
        assert_eq!(listed, expected);
    });
    case.check(
        "get answers the handle, whose info reads the stored Schedule",
        || {
            assert_eq!(got.id(), &nightly);
            assert_eq!(
                (
                    &info.schedule_id,
                    &info.target,
                    &info.trigger,
                    info.time_zone.as_str(),
                    info.version,
                    &info.state,
                ),
                (
                    &nightly,
                    &definition(None, Duration::ZERO, None).target,
                    &ScheduleTrigger::Every(EveryTrigger {
                        every: 3_600_000,
                        anchor: None
                    }),
                    "Europe/Paris",
                    1,
                    &ScheduleState::On,
                )
            );
        },
    );
    case.check("get of an unknown ID fails with schedule_not_found", || {
        assert_eq!(unknown.code(), ErrorCode::SCHEDULE_NOT_FOUND);
        assert_eq!(
            unknown
                .details::<ScheduleNotFoundDetails>()
                .unwrap()
                .schedule_id,
            unknown_id
        );
    });
    case.check(
        "delete answers whether it deleted, and info then fails",
        || {
            assert_eq!(deleted, [true, false]);
            assert_eq!(gone, ErrorCode::SCHEDULE_NOT_FOUND);
        },
    );
}

/// Covers: story:10-2
#[test]
fn the_blocking_api_reaches_the_same_schedule_store() {
    let env = TestEnv::new();
    let collie = blocking::Collie::connect(options(&env)).unwrap();
    let schedules = collie.schedules();
    let created = schedules
        .create(definition(
            Some("hourly-prices"),
            Duration::from_secs(3_600),
            None,
        ))
        .send()
        .unwrap();
    let listed = schedules.list().send().unwrap();
    let got = schedules.get(created.schedule().id()).send().unwrap();
    let zone = got.info().send().unwrap().into_inner().time_zone;
    let deleted = got.delete().send().unwrap();
    collie.close().unwrap();

    assert_eq!(
        (
            listed
                .iter()
                .map(|info| info.schedule_id.as_str())
                .collect::<Vec<_>>(),
            deleted,
        ),
        (vec!["hourly-prices"], true)
    );
    assert!(!zone.is_empty(), "a Schedule stores an explicit zone");
}
