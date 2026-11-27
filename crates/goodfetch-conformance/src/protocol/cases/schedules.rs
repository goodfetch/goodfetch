//! The Schedule store's cases (spec 589, ADR 0036), through the catalog's
//! `schedule.*` operations only: `schedule.create` answers the Schedule ID
//! given and refuses a taken one, `schedule.get` reads the definition as it
//! was stored, `schedule.list` holds every Schedule created, and
//! `schedule.delete` answers whether it deleted one. Each case creates the
//! Schedules it reads under IDs of its own, so the cases run in any order
//! on one store. On an Attached engine the calls reach the major's Daemon,
//! which keeps the store.

use goodfetch_protocol::schedule::{
    SCHEDULE_CREATE, SCHEDULE_DELETE, SCHEDULE_EXISTS, SCHEDULE_GET, SCHEDULE_LIST,
    SCHEDULE_NOT_FOUND, SCHEDULE_PREVIEW, SCHEDULE_TRIGGER_INVALID,
};
use serde_json::{Value, json};

use super::Checks;
use crate::protocol::connection::{Connection, Gone, code_of};

/// A Schedule store case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Create,
    Get,
    List,
    Delete,
    Preview,
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-create-answers-the-schedule-id-given-and-refuses-a-taken-one" => Self::Create,
            "a-get-reads-the-schedule-as-it-was-defined" => Self::Get,
            "a-list-holds-every-schedule-created" => Self::List,
            "a-delete-answers-whether-it-deleted-the-schedule" => Self::Delete,
            "a-preview-reads-both-dst-transitions-and-refuses-a-bad-trigger" => Self::Preview,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection.
    pub(crate) fn run(self, connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
        match self {
            Self::Create => create(connection, checks),
            Self::Get => get(connection, checks),
            Self::List => list(connection, checks),
            Self::Delete => delete(connection, checks),
            Self::Preview => preview(connection, checks),
        }
    }
}

/// The answer of `operation` with `params`.
fn answer_of(connection: &mut Connection, operation: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(operation, params)?;
    Ok(connection.answer(id)?.message)
}

/// The answer of `schedule.create` for a Schedule `id` that starts the
/// Recipe `shop-prices` every hour in Paris.
fn created(connection: &mut Connection, id: &str) -> Result<Value, Gone> {
    let definition = json!({
        "id": id,
        "target": { "recipe": "shop-prices" },
        "trigger": { "every": 3_600_000 },
        "time_zone": "Europe/Paris",
    });
    answer_of(
        connection,
        SCHEDULE_CREATE.name,
        &json!({ "definition": definition }),
    )
}

/// The answer of `schedule.get` for `id`.
fn got(connection: &mut Connection, id: &str) -> Result<Value, Gone> {
    answer_of(connection, SCHEDULE_GET.name, &json!({ "id": id }))
}

fn create(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let first = created(connection, "conformance-create")?;
    let again = created(connection, "conformance-create")?;
    checks.check(
        "schedule.create answers the Schedule ID given, with no warnings",
        first["result"]["schedule_id"] == json!("conformance-create")
            && first["result"]["warnings"] == json!([]),
    );
    checks.check(
        "schedule.create of a taken ID fails with schedule_exists",
        *code_of(&again) == json!(SCHEDULE_EXISTS.name),
    );
    Ok(())
}

fn get(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    created(connection, "conformance-get")?;
    let info = got(connection, "conformance-get")?;
    let unknown = got(connection, "conformance-never-created")?;
    let result = &info["result"];
    checks.check(
        "schedule.get reads the Schedule's target, trigger and zone as defined, at version 1 and on",
        result["schedule_id"] == json!("conformance-get")
            && result["target"] == json!({ "recipe": "shop-prices" })
            && result["trigger"]["every"] == json!(3_600_000)
            && result["time_zone"] == json!("Europe/Paris")
            && result["version"] == json!(1)
            && result["state"] == json!("on"),
    );
    checks.check(
        "schedule.get of an unknown ID fails with schedule_not_found naming it",
        *code_of(&unknown) == json!(SCHEDULE_NOT_FOUND.name)
            && unknown["error"]["data"]["schedule_id"] == json!("conformance-never-created"),
    );
    Ok(())
}

fn list(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    created(connection, "conformance-list-a")?;
    created(connection, "conformance-list-b")?;
    let listed = answer_of(connection, SCHEDULE_LIST.name, &json!({}))?;
    let ids: Vec<&str> = listed["result"]["schedules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|info| info["schedule_id"].as_str())
        .collect();
    checks.check(
        "schedule.list holds both Schedules created",
        ["conformance-list-a", "conformance-list-b"]
            .iter()
            .all(|id| ids.contains(id)),
    );
    Ok(())
}

fn delete(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    created(connection, "conformance-delete")?;
    let params = json!({ "schedule_id": "conformance-delete" });
    let deleted = answer_of(connection, SCHEDULE_DELETE.name, &params)?;
    let again = answer_of(connection, SCHEDULE_DELETE.name, &params)?;
    let gone = got(connection, "conformance-delete")?;
    checks.check(
        "schedule.delete answers true, then false once the Schedule is gone",
        deleted["result"]["deleted"] == json!(true) && again["result"]["deleted"] == json!(false),
    );
    checks.check(
        "schedule.get of a deleted Schedule fails with schedule_not_found",
        *code_of(&gone) == json!(SCHEDULE_NOT_FOUND.name),
    );
    Ok(())
}

fn preview(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    // Spring DST gap in Europe/Paris (2026-03-29: 02:00..03:00 skipped).
    let fixed_spring = answer_of(
        connection,
        SCHEDULE_PREVIEW.name,
        &json!({
            "trigger": { "cron": "30 2 * * *" },
            "time_zone": "Europe/Paris",
            "after": "2026-03-29T00:00:00Z",
            "count": 2,
        }),
    )?;
    let spring_ats: Vec<&str> = occurrence_ats(&fixed_spring);
    checks.check(
        "fixed-time cron in the spring DST gap fires once at the first valid instant after the gap",
        spring_ats == ["2026-03-29T01:00:00Z", "2026-03-30T00:30:00Z"]
            && fixed_spring["result"]["description"]["structured"]["fixed_time"] == json!(true)
            && fixed_spring["result"]["description"]["english"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
    );

    let step_spring = answer_of(
        connection,
        SCHEDULE_PREVIEW.name,
        &json!({
            "trigger": { "cron": "*/30 * * * *" },
            "time_zone": "Europe/Paris",
            "after": "2026-03-29T00:45:00Z",
            "count": 2,
        }),
    )?;
    checks.check(
        "step cron skips the spring DST gap",
        occurrence_ats(&step_spring) == ["2026-03-29T01:00:00Z", "2026-03-29T01:30:00Z"],
    );

    // Autumn DST overlap in Europe/Paris (2026-10-25: 02:00..03:00 happens twice).
    let fixed_autumn = answer_of(
        connection,
        SCHEDULE_PREVIEW.name,
        &json!({
            "trigger": { "cron": "30 2 * * *" },
            "time_zone": "Europe/Paris",
            "after": "2026-10-24T23:00:00Z",
            "count": 2,
        }),
    )?;
    checks.check(
        "fixed-time cron fires once in the repeated autumn hour",
        occurrence_ats(&fixed_autumn) == ["2026-10-25T00:30:00Z", "2026-10-26T01:30:00Z"],
    );

    let step_autumn = answer_of(
        connection,
        SCHEDULE_PREVIEW.name,
        &json!({
            "trigger": { "cron": "*/30 * * * *" },
            "time_zone": "Europe/Paris",
            "after": "2026-10-24T23:45:00Z",
            "count": 4,
        }),
    )?;
    checks.check(
        "step cron fires on both passes of the repeated autumn hour",
        occurrence_ats(&step_autumn)
            == [
                "2026-10-25T00:00:00Z",
                "2026-10-25T00:30:00Z",
                "2026-10-25T01:00:00Z",
                "2026-10-25T01:30:00Z",
            ],
    );

    let interval = answer_of(
        connection,
        SCHEDULE_PREVIEW.name,
        &json!({
            "trigger": { "every": 3_600_000, "anchor": "2026-03-29T00:15:00Z" },
            "time_zone": "Europe/Paris",
            "after": "2026-03-29T00:15:00Z",
            "count": 2,
        }),
    )?;
    checks.check(
        "anchored interval counts elapsed time across DST",
        occurrence_ats(&interval) == ["2026-03-29T01:15:00Z", "2026-03-29T02:15:00Z"],
    );

    let bad = answer_of(
        connection,
        SCHEDULE_PREVIEW.name,
        &json!({
            "trigger": { "cron": "*/0 * * * *" },
            "time_zone": "Europe/Paris",
        }),
    )?;
    checks.check(
        "malformed cron fails with schedule_trigger_invalid naming expression, position and reason",
        *code_of(&bad) == json!(SCHEDULE_TRIGGER_INVALID.name)
            && bad["error"]["data"]["expression"] == json!("*/0 * * * *")
            && bad["error"]["data"]["position"] == json!(1)
            && bad["error"]["data"]["reason"] == json!("bad_step"),
    );

    Ok(())
}

fn occurrence_ats(answer: &Value) -> Vec<&str> {
    answer["result"]["occurrences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|occ| occ["at"].as_str())
        .collect()
}
