//! Which catalog operations are commands, and how a command line becomes a
//! call, read from the catalog and from entries made up for one case each
//! (ADR 0099).

use clap::Command;
use clap::error::ErrorKind;
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::entries::operations_of;
use proptest::prelude::{prop, prop_assert_eq, proptest};
use rstest::rstest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Call, Generated, NAMESPACES, Namespace, Offer, OwnFlags, command, duration, member, members_of,
    offer,
};
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
    let commands: Vec<(&str, Offer)> = operations_of(Product::Collie)
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
            ("needs_you.list", Offer::Command),
            ("needs_you.dismiss", Offer::Command),
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
            ("session.pages", Offer::Command),
            ("page.close", Offer::Command),
            ("run.start", Offer::Command),
            ("run.finish", Offer::Command),
            ("run.stop", Offer::Command),
            ("run.abort", Offer::Command),
            ("run.usage", Offer::Command),
            ("run.progress", Offer::Command),
            ("run.report", Offer::Command),
            ("run.list", Offer::Command),
            ("run.checkpoint", Offer::Command),
            ("run.pause", Offer::Command),
            ("run.migrate", Offer::Command),
            ("run.resume", Offer::Command),
            ("key_value_store.get", Offer::Command),
            ("key_value_store.set", Offer::Command),
            ("key_value_store.get_bytes", Offer::Command),
            ("key_value_store.set_bytes", Offer::Command),
            ("key_value_store.delete", Offer::Command),
            ("key_value_store.keys", Offer::Follow),
            ("audit.export", Offer::Command),
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
    let names: Vec<&str> = NAMESPACES
        .iter()
        .filter(|namespace| namespace.product == Product::Collie)
        .map(|namespace| namespace.name)
        .collect();

    assert_eq!(
        names,
        [
            "daemon",
            "certs",
            "engine",
            "events",
            "needs_you",
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
        NAMESPACES[5],
        Namespace {
            name: "run",
            spelled: "run",
            about: "Calls the catalog's `run` operations",
            product: Product::Collie,
        }
    );
}

/// Foxhound's catalog adds its crawl's commands to Collie's, in the
/// catalog's order (ADRs 0095 and 0099): a sequence of what a crawl
/// committed prints it once the call ends unless `--follow` is given, a
/// round trip that answers pages of what a crawl committed follows them under
/// `--follow`, and a stream of seeds reads them from stdin. A handler's
/// own calls and `crawl.serve`, which serves a handler, stay the SDKs'
/// (ADR 0104).
///
/// Covers: story:11-1130
#[test]
fn foxhounds_catalog_adds_the_crawls_commands_to_collies() {
    let collie: Vec<&str> = operations_of(Product::Collie)
        .map(|operation| operation.name)
        .collect();
    let added: Vec<(&str, Offer)> = operations_of(Product::Foxhound)
        .filter(|operation| !collie.contains(&operation.name))
        .filter_map(|operation| Some((operation.name, offer(operation)?)))
        .collect();

    assert_eq!(
        added,
        [
            ("sample.crawl", Offer::Command),
            ("crawl.start", Offer::Command),
            ("run.seeds", Offer::Command),
            ("run.add_seed", Offer::Command),
            ("run.add_seeds", Offer::Input),
            ("run.records", Offer::Committed),
            ("dataset.list", Offer::Command),
            ("dataset.page", Offer::Command),
            ("run.export", Offer::Command),
            ("run.outcomes", Offer::Pages),
            ("run.failed_requests", Offer::Command),
            ("run.retry_failed", Offer::Command),
            ("run.rejected_records", Offer::Pages),
            ("dataset.query", Offer::Command),
        ]
    );
}

/// The generated namespaces hold Foxhound's commands' too, each marked by
/// the Product whose catalog first gives it a command: Collie's first, in
/// their order, then the two only Foxhound has.
#[test]
fn the_generated_namespaces_mark_the_two_only_foxhound_has() {
    let foxhounds: Vec<&str> = NAMESPACES
        .iter()
        .filter(|namespace| namespace.product == Product::Foxhound)
        .map(|namespace| namespace.name)
        .collect();

    assert_eq!(foxhounds, ["crawl", "dataset"]);
}

/// The command line of `product`'s executable, with no hand-written
/// command, and the generated commands it carries.
fn command_line(product: Product) -> (Command, Generated) {
    let generated = Generated::beside(product, &Command::new("goodfetch"));
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
/// refuses with `not_in_product` before clap reads it.
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
#[case::crawl_start(
    &["crawl", "start", "https://example.test/", "--run-id", "r1"],
    ("crawl.start", json!({
        "run_id": "r1",
        "seeds": [{ "url": "https://example.test/", "label": "page" }],
        "extraction": { "formats": { "markdown": true } },
    }))
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

/// `crawl start`'s flags give the code-free crawl's seeds and its
/// `extraction`: each URL a seed of its Label, each `--format` a format,
/// each `--field` a named field by a CSS selector or, after `xpath:`, an
/// `XPath` expression, and `--links` its `follow`; with neither a format
/// nor a field, each page's Markdown.
///
/// Covers: story:11-1132
#[rstest]
#[case::urls_formats_fields_and_links(
    &[
        "crawl", "start", "https://a.test/", "https://b.test/",
        "--format", "markdown", "--format", "links",
        "--field", "title=h1", "--field", "price=xpath://span[@class='price']",
        "--field", "name=css:.name", "--links", "none",
    ],
    json!({
        "seeds": [
            { "url": "https://a.test/", "label": "page" },
            { "url": "https://b.test/", "label": "page" },
        ],
        "extraction": {
            "formats": { "markdown": true, "links": true },
            "follow": "none",
            "fields": {
                "title": { "locator": { "kind": "css", "selector": "h1" } },
                "price": { "locator": { "kind": "xpath", "selector": "//span[@class='price']" } },
                "name": { "locator": { "kind": "css", "selector": ".name" } },
            },
        },
    })
)]
#[case::a_url_alone(
    &["crawl", "start", "https://a.test/"],
    json!({
        "seeds": [{ "url": "https://a.test/", "label": "page" }],
        "extraction": { "formats": { "markdown": true } },
    })
)]
#[case::a_field_alone(
    &["crawl", "start", "https://a.test/", "--field", "title=h1", "--links", "sitemap"],
    json!({
        "seeds": [{ "url": "https://a.test/", "label": "page" }],
        "extraction": {
            "formats": {},
            "follow": "sitemap",
            "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } },
        },
    })
)]
fn crawl_starts_flags_give_its_seeds_and_extraction(
    #[case] arguments: &[&str],
    #[case] params: Value,
) {
    let call = asked(Product::Foxhound, arguments).unwrap();

    assert_eq!(Value::Object(call.params), params);
}

/// `--seeds-file` adds a seed per line, a URL or a seed as JSON, after the
/// URLs; `--fields` reads named fields in the catalog's JSON form, which a
/// `--field` of the same name beats.
///
/// Covers: story:11-1132
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the files the flags read"
)]
fn crawl_start_reads_seeds_and_fields_from_files() {
    let folder = tempfile::tempdir().unwrap();
    let seeds = folder.path().join("seeds.txt");
    std::fs::write(
        &seeds,
        "https://b.test/\n\n{\"url\":\"https://c.test/\",\"label\":\"page\",\"priority\":2}\n",
    )
    .unwrap();
    let fields = folder.path().join("fields.json");
    std::fs::write(
        &fields,
        r#"{"price":{"locator":{"kind":"css","selector":".price"},"attribute":"data-price"},"title":{"locator":{"kind":"css","selector":"h2"}}}"#,
    )
    .unwrap();

    let call = asked(
        Product::Foxhound,
        &[
            "crawl",
            "start",
            "https://a.test/",
            "--seeds-file",
            seeds.to_str().unwrap(),
            "--fields",
            fields.to_str().unwrap(),
            "--field",
            "title=h1",
        ],
    )
    .unwrap();

    assert_eq!(
        (
            call.params["seeds"].clone(),
            call.params["extraction"].clone()
        ),
        (
            json!([
                { "url": "https://a.test/", "label": "page" },
                { "url": "https://b.test/", "label": "page" },
                { "url": "https://c.test/", "label": "page", "priority": 2 },
            ]),
            json!({
                "formats": {},
                "fields": {
                    "price": {
                        "locator": { "kind": "css", "selector": ".price" },
                        "attribute": "data-price",
                    },
                    "title": { "locator": { "kind": "css", "selector": "h1" } },
                },
            })
        )
    );
}

/// A seeds file whose seeds would pass the engine's message limit in
/// `crawl.start` leaves the seeds past half of it to `run.add_seeds`,
/// after the URLs given: no message carries every seed.
///
/// Covers: story:11-1132
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the seeds file the flag reads"
)]
fn a_seeds_file_past_half_the_message_limit_leaves_its_last_seeds_to_run_add_seeds() {
    let folder = tempfile::tempdir().unwrap();
    let seeds = folder.path().join("seeds.txt");
    let url = |n: usize| format!("https://a.test/{n}?{}", "q".repeat(1000));
    let lines: Vec<String> = (0..20_000).map(url).collect();
    std::fs::write(&seeds, lines.join("\n")).unwrap();

    let call = asked(
        Product::Foxhound,
        &[
            "crawl",
            "start",
            "https://first.test/",
            "--seeds-file",
            seeds.to_str().unwrap(),
        ],
    )
    .unwrap();

    let Some(OwnFlags::CrawlStart(flags)) = &call.own else {
        panic!("crawl start takes flags of its own");
    };
    let started = call.params["seeds"].as_array().unwrap();
    let start_bytes = call.params["seeds"].to_string().len();
    let urls: Vec<&str> = started
        .iter()
        .chain(&flags.added)
        .map(|seed| seed["url"].as_str().unwrap())
        .collect();
    let mut expected = vec!["https://first.test/".to_owned()];
    expected.extend(lines);
    // Compared whole but shown short: the seeds hold 20 MB.
    assert_eq!(
        (
            start_bytes <= goodfetch_engine::rpc::framing::MAX_MESSAGE_BYTES / 2,
            !flags.added.is_empty(),
            urls == expected.iter().map(String::as_str).collect::<Vec<_>>(),
        ),
        (true, true, true),
        "{start_bytes} bytes in crawl.start, {} seeds left to run.add_seeds",
        flags.added.len()
    );
}

/// A seeds file within half the message limit goes whole into
/// `crawl.start`, which then leaves nothing to `run.add_seeds`.
///
/// Covers: story:11-1132
#[test]
fn a_small_seeds_list_goes_whole_into_crawl_start() {
    let call = asked(Product::Foxhound, &["crawl", "start", "https://a.test/"]).unwrap();

    let Some(OwnFlags::CrawlStart(flags)) = &call.own else {
        panic!("crawl start takes flags of its own");
    };
    assert_eq!(flags.added, Vec::<Value>::new());
}

/// `crawl start` offers no `--datasets`, which a code-free crawl refuses,
/// and has a help of its own, not `crawl.start`'s of a handler's crawl.
///
/// Covers: story:11-1132
#[test]
fn crawl_start_offers_no_datasets_flag_and_tells_of_a_code_free_crawl() {
    let (mut command, _) = command_line(Product::Foxhound);
    command.build();
    let start = command
        .find_subcommand_mut("crawl")
        .and_then(|crawl| {
            crawl.build();
            crawl.find_subcommand("start").cloned()
        })
        .unwrap();

    let flags: Vec<String> = start
        .get_arguments()
        .filter_map(|argument| argument.get_long().map(str::to_owned))
        .collect();
    let about = start
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert_eq!(
        (
            flags.contains(&"datasets".to_owned()),
            about.contains("code-free"),
            about.contains("handler")
        ),
        (false, true, false),
        "{flags:?}\n{about}"
    );
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

/// What `crawl start` cannot read is a usage error before any engine call:
/// no seed, a format `page.formats` does not take, a `--links` value
/// `extraction.follow` does not take, a field with no name or locator, and
/// `--require-verified` without `--follow`.
///
/// Covers: story:11-1132
#[rstest]
#[case::no_seed(&["crawl", "start", "--format", "markdown"], ErrorKind::MissingRequiredArgument)]
#[case::unknown_format(&["crawl", "start", "https://a.test/", "--format", "pdf"], ErrorKind::InvalidValue)]
#[case::unknown_links(&["crawl", "start", "https://a.test/", "--links", "everywhere"], ErrorKind::InvalidValue)]
#[case::field_without_a_locator(&["crawl", "start", "https://a.test/", "--field", "title"], ErrorKind::ValueValidation)]
#[case::field_without_a_name(&["crawl", "start", "https://a.test/", "--field", "=h1"], ErrorKind::ValueValidation)]
#[case::require_verified_without_follow(&["crawl", "start", "https://a.test/", "--require-verified"], ErrorKind::MissingRequiredArgument)]
fn crawl_start_refuses_what_it_cannot_read(#[case] arguments: &[&str], #[case] kind: ErrorKind) {
    let refused = asked(Product::Foxhound, arguments).err().unwrap();

    assert_eq!(refused.kind(), kind, "{refused}");
}

/// `crawl start` takes `--follow`, and under it `--require-verified`.
///
/// Covers: story:11-1132
#[rstest]
#[case::not_followed(&["crawl", "start", "https://a.test/"], (false, false))]
#[case::followed(&["crawl", "start", "https://a.test/", "--follow"], (true, false))]
#[case::verified(
    &["crawl", "start", "https://a.test/", "--follow", "--require-verified"],
    (true, true)
)]
fn crawl_start_follows_its_run_under_follow(
    #[case] arguments: &[&str],
    #[case] expected: (bool, bool),
) {
    let call = asked(Product::Foxhound, arguments).unwrap();

    let Some(OwnFlags::CrawlStart(flags)) = call.own else {
        panic!("crawl start takes flags of its own");
    };
    assert_eq!((flags.follow, flags.require_verified), expected);
}

/// `run export` takes the Dataset as its second positional argument, after
/// the Run ID, and needs `--out`.
///
/// Covers: story:11-1132
#[rstest]
#[case::to_standard_output(
    &["run", "export", "r1", "pages", "--format", "jsonl", "--out", "-"],
    Ok(json!({ "run_id": "r1", "dataset": "pages", "format": "jsonl" }))
)]
#[case::compressed(
    &["run", "export", "r1", "pages", "--format", "csv", "--compression", "zstd", "--out", "pages.csv.zst"],
    Ok(json!({ "run_id": "r1", "dataset": "pages", "format": "csv", "compression": "zstd" }))
)]
#[case::without_out(
    &["run", "export", "r1", "pages", "--format", "jsonl"],
    Err(ErrorKind::MissingRequiredArgument)
)]
#[case::without_a_dataset(
    &["run", "export", "r1", "--format", "jsonl", "--out", "-"],
    Err(ErrorKind::MissingRequiredArgument)
)]
fn run_export_takes_its_dataset_second_and_out(
    #[case] arguments: &[&str],
    #[case] expected: Result<Value, ErrorKind>,
) {
    let asked = asked(Product::Foxhound, arguments);

    assert_eq!(
        asked
            .map(|call| Value::Object(call.params))
            .map_err(|refused| refused.kind()),
        expected
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
    Ok(Value::Object(
        Call::of(Product::Collie, operation, &matches).params,
    ))
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

/// Collie's catalog entry `name`.
fn collies(name: &str) -> &'static Operation {
    operations_of(Product::Collie)
        .find(|operation| operation.name == name)
        .unwrap()
}

/// The call a command line gives `operation`'s command.
fn call(operation: &'static Operation, arguments: &[&str]) -> Result<Call, clap::Error> {
    let matches = command(operation).try_get_matches_from(
        std::iter::once(member(operation)).chain(arguments.iter().copied()),
    )?;
    Ok(Call::of(Product::Collie, operation, &matches))
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
