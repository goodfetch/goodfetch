//! Which catalog operations are commands, and how a command line becomes a
//! call, read from the catalog and from entries made up for one case each
//! (ADR 0099).

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::entries::operations_of;
use rstest::rstest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Offer, PRODUCT, offer};

/// What the made-up operations take.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WidgetParams {
    /// The widget's ID.
    widget_id: String,
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
            ("engine.info", Offer::Command),
            ("events.subscribe", Offer::Follow),
            ("page.body", Offer::Follow),
            ("run.start", Offer::Command),
            ("run.finish", Offer::Command),
            ("run.abort", Offer::Command),
            ("sample.echo", Offer::Command),
            ("config.explain", Offer::Command),
        ]
    );
}
