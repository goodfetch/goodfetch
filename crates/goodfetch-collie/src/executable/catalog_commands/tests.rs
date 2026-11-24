//! Which catalog operations are commands, and how a command line becomes a
//! call, read from the catalog and from entries made up for one case each
//! (ADR 0099).

use clap::Command;
use clap::error::ErrorKind;
use goodfetch_protocol::catalog::{
    Bound, Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability,
    StreamInput, schema_of,
};
use goodfetch_protocol::entries::operations_of;
use proptest::prelude::{prop, prop_assert_eq, proptest};
use rstest::rstest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Call, Generated, Offer, OwnCommands, command, duration, member, members_of, offer};
use crate::executable::engine_choice::{Choice, Lifetime};

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
        input: None,
        bound: None,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    }
}

/// The read that bounds a made-up sequence.
const WIDGET_PAGE: Operation =
    operation("widget.page", OperationKind::RoundTrip, Reach::AllSurfaces);

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
    None
)]
#[case::opener(
    operation("widget.open", OperationKind::Opener, Reach::AllSurfaces),
    Some(Offer::Command)
)]
#[case::sequence(
    operation("widget.frames", OperationKind::Sequence, Reach::AllSurfaces),
    Some(Offer::Follow)
)]
#[case::sequence_taking_a_stream(
    Operation {
        input: Some(StreamInput { notification: "$/widgets", field: "widgets" }),
        ..operation("widget.add", OperationKind::Sequence, Reach::AllSurfaces)
    },
    Some(Offer::Input)
)]
#[case::sequence_of_what_a_run_committed(
    Operation {
        bound: Some(Bound { read: &WIDGET_PAGE, params: Value::clone, through: "through" }),
        ..operation("widget.spins", OperationKind::Sequence, Reach::AllSurfaces)
    },
    Some(Offer::Committed)
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

/// The command line of `product`'s executable, with no hand-written
/// command, and the generated commands it carries.
fn command_line(product: Product) -> (Command, Generated) {
    let own = match product {
        Product::Collie => OwnCommands::COLLIE,
        Product::Foxhound => OwnCommands {
            commands: &[],
            members: crate::executable::tests::foxhounds_catalog_members,
            answers: &[],
        },
    };
    let generated = Generated::beside(product, own, &Command::new("goodfetch"));
    (generated.add_to(Command::new("goodfetch")), generated)
}

/// The call `arguments` ask of `product`'s generated commands, or clap's
/// refusal.
fn asked(product: Product, arguments: &[&str]) -> Result<Call, clap::Error> {
    let (command, generated) = command_line(product);
    let matches = command
        .try_get_matches_from(std::iter::once("goodfetch").chain(arguments.iter().copied()))?;
    Ok(generated
        .call(&matches)
        .expect("a generated command parsed"))
}

/// Foxhound's executable carries a crawl command, a value-bound one taking
/// the Run ID first; Collie's has no such command, which its executable
/// refuses with `not_in_product` before clap reads it. Foxhound's crate
/// tests its own commands, `crawl start` among them.
///
/// Covers: story:11-1130, story:11-1131
#[rstest]
#[case::run_outcomes(
    &["run", "outcomes", "r1", "--labels", "page"],
    ("run.outcomes", json!({ "run_id": "r1", "labels": ["page"] }))
)]
#[case::run_records(
    &["run", "records", "r1", "--dataset", "pages", "--after", "4"],
    ("run.records", json!({ "run_id": "r1", "dataset": "pages", "after": 4 }))
)]
fn foxhounds_executable_carries_the_crawl_commands_and_collies_does_not(
    #[case] arguments: &[&str],
    #[case] expected: (&str, Value),
) {
    let call = asked(Product::Foxhound, arguments).unwrap();
    let refused = asked(Product::Collie, arguments).err().unwrap();

    assert_eq!(
        (
            (call.operation.name, Value::Object(call.params)),
            refused.kind()
        ),
        (expected, ErrorKind::InvalidSubcommand)
    );
}

/// A sequence of what a crawl committed and a round trip answering pages
/// of its endings each run with or without `--follow`; Collie's sequences
/// still need it.
///
/// Covers: story:11-1131
#[rstest]
#[case::committed_without(&["run", "records", "r1", "--dataset", "pages"], false)]
#[case::committed_with(&["run", "records", "r1", "--dataset", "pages", "--follow"], true)]
#[case::pages_without(&["run", "outcomes", "r1"], false)]
#[case::pages_with(&["run", "outcomes", "r1", "--follow"], true)]
fn a_crawls_reads_follow_only_under_follow(#[case] arguments: &[&str], #[case] follow: bool) {
    assert_eq!(asked(Product::Foxhound, arguments).unwrap().follow, follow);
}

/// An optional boolean's flag takes `true` or `false`, so `run records
/// --wall false` reads only the Records read past no Wall, and its absence
/// leaves the param out.
///
/// Covers: story:11-1131
#[rstest]
#[case::only_the_others(&["--wall", "false"], Some(json!(false)))]
#[case::only_the_walled(&["--wall=true"], Some(json!(true)))]
#[case::absent(&[], None)]
fn an_optional_booleans_flag_takes_true_or_false(
    #[case] wall: &[&str],
    #[case] expected: Option<Value>,
) {
    let mut arguments = vec!["run", "records", "r1", "--dataset", "pages"];
    arguments.extend(wall);

    let call = asked(Product::Foxhound, &arguments).unwrap();

    assert_eq!(call.params.get("wall").cloned(), expected);
}

/// The values a command reads from stdin go one per line, each sent as
/// its line is read, a line that is no JSON as text and a blank line
/// passed over; a line that is not UTF-8 ends them, its number named as
/// the command's usage error, after the values before it.
///
/// Covers: story:11-1130
#[test]
fn stdin_values_stop_at_a_line_that_is_not_utf8_naming_its_number() {
    let input: &[u8] =
        b"{\"url\":\"https://a.test/\"}\n\nplain\n\xff\xfe\n{\"url\":\"https://b.test/\"}\n";
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let refused = std::sync::OnceLock::new();

    super::read_values(&mut std::io::BufReader::new(input), &sender, &refused);
    drop(sender);

    let mut sent = Vec::new();
    while let Ok(value) = receiver.try_recv() {
        sent.push(value);
    }
    assert_eq!(
        (sent, refused.get().cloned()),
        (
            vec![json!({ "url": "https://a.test/" }), json!("plain")],
            Some(
                "line 4 of standard input is not UTF-8: give one value per line in UTF-8"
                    .to_owned()
            )
        )
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
    let matches = command(operation, None).try_get_matches_from(
        std::iter::once(member(operation)).chain(arguments.iter().copied()),
    )?;
    Ok(Value::Object(
        Call::of(Product::Collie, operation, None, &matches).params,
    ))
}

/// Covers: story:11-198
#[test]
fn an_entry_of_an_offered_kind_adds_its_command() {
    let group = members_of(
        Command::new("widget"),
        [&SPIN, &FRAMES, &WAIT_FOR_STOP].into_iter(),
        &[],
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

/// `run start` takes its Run's Limits as `--limit`, repeatable, and no
/// longer as raw JSON.
#[test]
fn run_start_takes_its_limits_as_repeated_limit_flags() {
    let run_start = collies("run.start");

    let params = parse(
        run_start,
        &["--limit", "requests=5", "--limit", "page_crashes=1"],
    )
    .unwrap();

    assert_eq!(
        params,
        json!({ "limits": [
            { "usage_kind": "requests", "amount": 5 },
            { "usage_kind": "page_crashes", "amount": 1 },
        ] })
    );
}

/// Collie's catalog entry `name`.
fn collies(name: &str) -> &'static Operation {
    operations_of(Product::Collie)
        .find(|operation| operation.name == name)
        .unwrap()
}

/// The call a command line gives `operation`'s command.
fn call(operation: &'static Operation, arguments: &[&str]) -> Result<Call, clap::Error> {
    let matches = command(operation, None).try_get_matches_from(
        std::iter::once(member(operation)).chain(arguments.iter().copied()),
    )?;
    Ok(Call::of(Product::Collie, operation, None, &matches))
}

/// An opener's new value and the value a value-bound command names outlive
/// the command, so they need the Daemon; nothing else does (ADR 0088).
///
/// Covers: story:11-199
#[rstest]
#[case::an_opener("run.start", &[], Lifetime::Lasting)]
#[case::a_value_bound_command("run.finish", &["first"], Lifetime::Lasting)]
#[case::a_page_bound_command("page.reload", &["page-1"], Lifetime::Lasting)]
#[case::a_command_with_no_value("engine.info", &[], Lifetime::Passing)]
#[case::a_run_s_store_named_by_flag("key_value_store.get", &["--run-id", "first", "--key", "k"], Lifetime::Passing)]
fn a_commands_lifetime_is_its_values(
    #[case] name: &str,
    #[case] arguments: &[&str],
    #[case] lifetime: Lifetime,
) {
    assert_eq!(call(collies(name), arguments).unwrap().lifetime(), lifetime);
}

/// `--attached` and `--daemon` force either engine, and together are a
/// usage error, before any engine call (ADR 0024).
#[rstest]
#[case::neither(&[], Choice::Auto)]
#[case::attached(&["--attached"], Choice::Attached)]
#[case::daemon(&["--daemon"], Choice::Daemon)]
fn attached_and_daemon_choose_the_engine(#[case] arguments: &[&str], #[case] choice: Choice) {
    assert_eq!(
        call(&SPIN, &[&["w1"], arguments].concat()).unwrap().choice,
        choice
    );
}

#[test]
fn attached_and_daemon_together_are_a_usage_error() {
    let refused = call(&SPIN, &["w1", "--attached", "--daemon"])
        .err()
        .unwrap();

    assert_eq!(
        (refused.kind(), refused.exit_code()),
        (ErrorKind::ArgumentConflict, 2)
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
