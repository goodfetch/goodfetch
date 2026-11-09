//! Foxhound's own commands as its executable reads them (ADR 0099), and
//! Foxhound's help beside Collie's (ADRs 0090 and 0094).

use std::fmt::Write as _;

use clap::Command;
use clap::error::ErrorKind;
use goodfetch_collie::executable::catalog_commands::{Call, Generated};
use goodfetch_collie::executable::{OwnCommands, command};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound_names;
use goodfetch_test_support::{
    certificate_verification_switches, command_help, command_paths, long_flags,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::COMMANDS;
use super::crawl_start::Flags;

/// The commands `product`'s executable writes by hand.
fn own(product: Product) -> OwnCommands {
    match product {
        Product::Collie => OwnCommands::COLLIE,
        Product::Foxhound => COMMANDS,
    }
}

/// The command line of `product`'s executable, with no hand-written
/// command, and the generated commands it carries, its own among them.
fn command_line(product: Product) -> (Command, Generated) {
    let generated = Generated::beside(product, own(product), &Command::new("goodfetch"));
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

/// The params `crawl start`'s own flags give and what they give beside
/// them, for `arguments`, a `crawl start` command line Foxhound's executable
/// reads.
fn crawl_start(arguments: &[&str]) -> (Map<String, Value>, Flags) {
    let (command, _) = command_line(Product::Foxhound);
    let matches = command
        .try_get_matches_from(std::iter::once("goodfetch").chain(arguments.iter().copied()))
        .unwrap();
    let (_, crawl) = matches.subcommand().unwrap();
    let (_, start) = crawl.subcommand().unwrap();
    let mut params = Map::new();
    let flags = Flags::read(start, &mut params);
    (params, flags)
}

/// Foxhound's executable carries `crawl start`, its own command; Collie's
/// has no such command, which its executable refuses with `not_in_product`
/// before clap reads it.
///
/// Covers: story:11-1130, story:11-1131
#[rstest]
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
            (call.operation().name, Value::Object(call.params().clone())),
            refused.kind()
        ),
        (expected, ErrorKind::InvalidSubcommand)
    );
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

    assert_eq!(Value::Object(call.params().clone()), params);
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
            call.params()["seeds"].clone(),
            call.params()["extraction"].clone()
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

    let (params, flags) = crawl_start(&[
        "crawl",
        "start",
        "https://first.test/",
        "--seeds-file",
        seeds.to_str().unwrap(),
    ]);

    let started = params["seeds"].as_array().unwrap();
    let start_bytes = params["seeds"].to_string().len();
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
    let (_, flags) = crawl_start(&["crawl", "start", "https://a.test/"]);

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
    let (_, flags) = crawl_start(arguments);

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
            .map(|call| Value::Object(call.params().clone()))
            .map_err(|refused| refused.kind()),
        expected
    );
}

#[test]
fn the_command_line_holds_no_clap_misconfiguration() {
    command(Product::Foxhound, COMMANDS).debug_assert();
}

/// A start builds no group's commands, Foxhound's own among them: clap
/// builds them only for the group it parses or shows help for, so
/// `--version` and every command skip building the rest of the tree (ADR
/// 0071's startup figure).
#[test]
fn a_start_builds_no_command_of_any_group() {
    let (command_line, _) = command_line(Product::Foxhound);

    let built: Vec<(&str, Vec<&str>)> = command_line
        .get_subcommands()
        .map(|group| {
            let commands: Vec<&str> = group.get_subcommands().map(Command::get_name).collect();
            (group.get_name(), commands)
        })
        .filter(|(_, commands)| !commands.is_empty())
        .collect();

    assert_eq!(built, Vec::<(&str, Vec<&str>)>::new());
}

/// Story 4-31: no flag of Foxhound's executable turns certificate
/// verification off, those of `crawl start` and `run export`, the commands
/// it writes by hand, among them. No flag reads a variable either: clap's
/// `env` feature is off.
/// Covers: story:4-31
#[test]
fn no_flag_switches_certificate_verification_off() {
    let flags = long_flags(&command(Product::Foxhound, COMMANDS));

    assert_eq!(
        (
            flags.iter().any(|flag| flag == "links"),
            flags.iter().any(|flag| flag == "out"),
        ),
        (true, true),
        "the enumeration reaches `crawl start`'s and `run export`'s flags"
    );
    assert_eq!(
        certificate_verification_switches(&flags),
        Vec::<String>::new()
    );
}

/// Whether Foxhound's help `foxhounds` of the command path `path` shows
/// Collie's help `collies` unchanged: each line of Collie's, in its order,
/// its executable's name aside, and between them only the lines of a
/// group's listing that name one of Foxhound's own commands of the group
/// (`foxhound_names::COMMANDS`).
fn shows_unchanged(collies: &str, foxhounds: &str, path: &[String]) -> bool {
    // A group's list aligns its commands' help to its longest name, so a
    // line is compared with its runs of spaces as one.
    let words = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");
    let foxhounds_own = |line: &String| {
        line.split(' ').next().is_some_and(|name| {
            let command = path
                .iter()
                .map(String::as_str)
                .chain([name])
                .collect::<Vec<_>>()
                .join(".");
            foxhound_names::COMMANDS.contains(&command.as_str())
        })
    };
    let theirs: Vec<String> = foxhounds
        .replace("goodfetch-foxhound", "goodfetch-collie")
        .lines()
        .map(words)
        .filter(|line| !foxhounds_own(line))
        .collect();
    let ours: Vec<String> = collies.lines().map(words).collect();
    theirs == ours
}

/// The containment check refuses a line Foxhound's help adds to a command
/// Collie carries, such as an option of `run start` only Foxhound's
/// executable has.
#[test]
fn the_containment_check_refuses_an_option_line_foxhound_adds_to_a_command_collie_carries() {
    let (collie, foxhound) = (
        command(Product::Collie, OwnCommands::COLLIE),
        command(Product::Foxhound, COMMANDS),
    );
    let path = ["run".to_owned(), "start".to_owned()];
    let foxhounds = command_help(&foxhound, &path);
    let planted = foxhounds.replacen(
        "Options:\n",
        "Options:\n      --extra  An option only Foxhound's executable has\n",
        1,
    );
    assert_ne!(planted, foxhounds, "the plant is in Foxhound's help");

    assert!(!shows_unchanged(
        &command_help(&collie, &path),
        &planted,
        &path
    ));
}

/// The containment check (ADRs 0090 and 0094): Foxhound's executable
/// carries every command of Collie's unchanged, so each command path of
/// Collie's renders the same help in Foxhound's, its executable's name
/// aside, a group's listing Foxhound's own commands beside Collie's (`run
/// outcomes` in `run`); the snapshot is Foxhound's whole help tree.
///
/// Covers: story:11-1130
#[test]
fn foxhounds_help_shows_each_command_collies_carries_unchanged() {
    let (collie, foxhound) = (
        command(Product::Collie, OwnCommands::COLLIE),
        command(Product::Foxhound, COMMANDS),
    );

    let changed: Vec<String> = command_paths(&collie)
        .into_iter()
        .filter(|path| {
            !shows_unchanged(
                &command_help(&collie, path),
                &command_help(&foxhound, path),
                path,
            )
        })
        .map(|path| path.join(" "))
        .collect();

    assert_eq!(changed, Vec::<String>::new());
    let tree = std::iter::once(Vec::new())
        .chain(command_paths(&foxhound))
        .fold(String::new(), |mut tree, path| {
            let words: Vec<&str> = std::iter::once("goodfetch-foxhound")
                .chain(path.iter().map(String::as_str))
                .chain(std::iter::once("--help"))
                .collect();
            let _ = writeln!(
                tree,
                "$ {}\n{}",
                words.join(" "),
                command_help(&foxhound, &path)
            );
            tree
        });
    insta::assert_snapshot!("help_tree_foxhound", tree);
}
