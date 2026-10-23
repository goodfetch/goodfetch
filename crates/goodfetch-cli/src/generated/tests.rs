//! Which catalog operations are commands, and how a command line becomes a
//! call, read from the catalog and from entries made up for one case each
//! (ADR 0099).

use clap::Command;
use clap::error::ErrorKind;
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::entries::operations_of;
use proptest::prelude::{prop, prop_assert_eq, proptest};
use rstest::rstest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Call, NAMESPACES, Namespace, Offer, PRODUCT, command, duration, member, members_of, offer,
};

/// What the made-up operations take.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WidgetParams {
    /// The widget's ID.
    widget_id: String,
    /// How long the call may take, in milliseconds; `null` for no timeout.
    #[serde(default)]
    timeout: Option<u64>,
}

/// What they answer.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WidgetResult {}

/// A made-up operation named `name`, of `kind`, offered on `reach`.
const fn operation(name: &'static str, kind: OperationKind, reach: Reach) -> Operation {
    Operation {
        name,
        description: "Spins a widget.",
        class: OperationClass::Read,
        kind,
        reach,
        params: schema_of::<WidgetParams>,
        result: schema_of::<WidgetResult>,
        events: &[],
        cancellable: false,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    }
}

#[rstest]
#[case::round_trip(
    operation("widget.spin", OperationKind::RoundTrip, Reach::AllSurfaces),
    Some(Offer::Command)
)]
#[case::query(
    operation("widget.count", OperationKind::Query, Reach::AllSurfaces),
    Some(Offer::Command)
)]
#[case::read(
    operation("widget.name", OperationKind::Read, Reach::AllSurfaces),
    Some(Offer::Command)
)]
#[case::opener(
    operation("widget.open", OperationKind::Opener, Reach::AllSurfaces),
    Some(Offer::Command)
)]
#[case::sequence(
    operation("widget.frames", OperationKind::Sequence, Reach::AllSurfaces),
    Some(Offer::Follow)
)]
#[case::waiter(
    operation("widget.wait_for_stop", OperationKind::Waiter, Reach::AllSurfaces),
    None
)]
#[case::event(
    operation("widget.spun", OperationKind::Event, Reach::AllSurfaces),
    None
)]
#[case::sdk_only(
    operation("widget.lend", OperationKind::RoundTrip, Reach::SdkOnly),
    None
)]
#[case::person_only(
    operation("widget.accept", OperationKind::RoundTrip, Reach::PersonOnly),
    None
)]
#[case::sdk_only_name_unmarked(
    operation("page.lend", OperationKind::RoundTrip, Reach::AllSurfaces),
    None
)]
#[case::session_cookies_unmarked(
    operation("session.cookies", OperationKind::Read, Reach::AllSurfaces),
    None
)]
fn an_operation_is_a_command_by_its_kind_and_reach(
    #[case] operation: Operation,
    #[case] offered: Option<Offer>,
) {
    assert_eq!(offer(&operation), offered);
}

/// Covers: story:11-198, story:11-218
#[test]
fn the_commands_of_collies_catalog_are_its_offered_operations() {
    let commands: Vec<(&str, Offer)> = operations_of(PRODUCT)
        .filter_map(|operation| Some((operation.name, offer(operation)?)))
        .collect();

    assert_eq!(
        commands,
        [
            ("daemon.status", Offer::Command),
            ("daemon.stop", Offer::Command),
            ("daemon.health", Offer::Command),
            ("certs.list", Offer::Command),
            ("engine.info", Offer::Command),
            ("events.subscribe", Offer::Follow),
            ("run.new_page", Offer::Command),
            ("run.new_session", Offer::Command),
            ("session.new_page", Offer::Command),
            ("session.close", Offer::Command),
            ("page.goto", Offer::Command),
            ("page.reload", Offer::Command),
            ("page.go_back", Offer::Command),
            ("page.go_forward", Offer::Command),
            ("page.formats", Offer::Command),
            ("page.body", Offer::Follow),
            ("locator.click", Offer::Command),
            ("locator.fill", Offer::Command),
            ("locator.press", Offer::Command),
            ("response.body", Offer::Command),
            ("run.start", Offer::Command),
            ("run.finish", Offer::Command),
            ("run.stop", Offer::Command),
            ("run.abort", Offer::Command),
            ("run.usage", Offer::Command),
            ("run.checkpoint", Offer::Command),
            ("run.pause", Offer::Command),
            ("run.migrate", Offer::Command),
            ("run.resume", Offer::Command),
            ("key_value_store.get", Offer::Command),
            ("key_value_store.set", Offer::Command),
            ("key_value_store.delete", Offer::Command),
            ("key_value_store.keys", Offer::Follow),
            ("audit.verify", Offer::Command),
            ("sample.echo", Offer::Command),
            ("config.explain", Offer::Command),
            ("usage.list", Offer::Command),
            ("usage.set", Offer::Command),
        ]
    );
}

/// The namespaces `xtask codegen` writes for a start to read, so that it
/// walks no catalog: each namespace of those commands once, in the
/// catalog's order, with its spelling and its group's help.
#[test]
fn the_generated_namespaces_are_those_of_collies_commands() {
    let names: Vec<&str> = NAMESPACES.iter().map(|namespace| namespace.name).collect();

    assert_eq!(
        names,
        [
            "daemon",
            "certs",
            "engine",
            "events",
            "run",
            "session",
            "page",
            "locator",
            "response",
            "key_value_store",
            "audit",
            "sample",
            "config",
            "usage"
        ]
    );
    assert_eq!(
        NAMESPACES[4],
        Namespace {
            name: "run",
            spelled: "run",
            about: "Calls the catalog's `run` operations",
        }
    );
}

static SPIN: Operation = operation("widget.spin", OperationKind::RoundTrip, Reach::AllSurfaces);
static FRAMES: Operation = operation("widget.frames", OperationKind::Sequence, Reach::AllSurfaces);
static WAIT_FOR_STOP: Operation = operation(
    "widget.wait_for_stop",
    OperationKind::Waiter,
    Reach::AllSurfaces,
);

/// The params `arguments` give `operation`'s command, or clap's refusal.
fn parse(operation: &'static Operation, arguments: &[&str]) -> Result<Value, clap::Error> {
    let matches = command(operation).try_get_matches_from(
        std::iter::once(member(operation)).chain(arguments.iter().copied()),
    )?;
    Ok(Value::Object(Call::of(operation, &matches).params))
}

/// Covers: story:11-198
#[test]
fn an_entry_of_an_offered_kind_adds_its_command() {
    let group = members_of(
        Command::new("widget"),
        [&SPIN, &FRAMES, &WAIT_FOR_STOP].into_iter(),
    );

    let names: Vec<&str> = group.get_subcommands().map(Command::get_name).collect();
    assert_eq!(names, ["spin", "frames"]);
}

#[test]
fn a_sequences_command_takes_follow_and_refuses_to_run_without_it() {
    let refused = parse(&FRAMES, &["w1"]).unwrap_err();

    assert_eq!(
        (refused.kind(), refused.exit_code()),
        (ErrorKind::MissingRequiredArgument, 2)
    );
    assert_eq!(
        parse(&FRAMES, &["w1", "--follow"]).unwrap(),
        json!({ "widget_id": "w1" })
    );
}

#[rstest]
#[case::omitted(&[], json!({ "widget_id": "w1" }))]
#[case::seconds(&["--timeout", "45s"], json!({ "widget_id": "w1", "timeout": 45_000 }))]
#[case::milliseconds(&["--timeout", "45000"], json!({ "widget_id": "w1", "timeout": 45_000 }))]
#[case::minutes(&["--timeout", "2m"], json!({ "widget_id": "w1", "timeout": 120_000 }))]
#[case::hours(&["--timeout", "1h"], json!({ "widget_id": "w1", "timeout": 3_600_000 }))]
#[case::suffixed_milliseconds(&["--timeout", "250ms"], json!({ "widget_id": "w1", "timeout": 250 }))]
#[case::none(&["--timeout", "none"], json!({ "widget_id": "w1", "timeout": null }))]
fn timeout_sets_the_three_states_of_a_calls_timeout(
    #[case] arguments: &[&str],
    #[case] params: Value,
) {
    let mut line = vec!["w1"];
    line.extend_from_slice(arguments);

    assert_eq!(parse(&SPIN, &line).unwrap(), params);
}

#[rstest]
#[case::zero("0")]
#[case::zero_seconds("0s")]
#[case::negative("-5")]
#[case::no_number("soon")]
#[case::unknown_unit("5d")]
fn a_timeout_that_is_no_positive_duration_is_a_usage_error(#[case] timeout: &str) {
    let refused = parse(&SPIN, &["w1", &format!("--timeout={timeout}")]).unwrap_err();

    assert_eq!(
        (refused.kind(), refused.exit_code()),
        (ErrorKind::ValueValidation, 2)
    );
}

proptest! {
    #[test]
    fn a_duration_in_any_unit_reads_as_its_milliseconds(
        count in 1_u64..=1_000_000,
        (unit, milliseconds) in prop::sample::select(
            vec![("ms", 1), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)]
        ),
    ) {
        prop_assert_eq!(
            duration(&format!("{count}{unit}")),
            duration(&(count * milliseconds).to_string())
        );
    }
}
