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
    SCHEDULE_NOT_FOUND,
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
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-create-answers-the-schedule-id-given-and-refuses-a-taken-one" => Self::Create,
            "a-get-reads-the-schedule-as-it-was-defined" => Self::Get,
            "a-list-holds-every-schedule-created" => Self::List,
            "a-delete-answers-whether-it-deleted-the-schedule" => Self::Delete,
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
