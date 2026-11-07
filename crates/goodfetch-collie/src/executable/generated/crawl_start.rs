//! `crawl start <url>…`, the command line's code-free crawl (ADRs 0099 and
//! 0103; spec #611's CLI): it calls `crawl.start` with the seeds and the
//! `extraction` its flags give in place of those two params, on the Daemon,
//! and prints the Run ID, the one JSON value under `--json`. Each URL is a
//! seed of the code-free crawl's Label, and so is each line of
//! `--seeds-file`, where a line holding a JSON object is a seed as
//! `crawl.start` takes it. `--format` names a format of `page.formats`,
//! repeatable; `--field <name>=<locator>` reads a named field by a CSS
//! selector, or an `XPath` expression after `xpath:`, given by the `text`
//! Value transform; `--fields` reads named fields in the catalog's JSON form;
//! `--links` says which pages the crawl fetches beyond its seeds. A crawl
//! given neither a format nor a field keeps each page's Markdown. Its help
//! is its own, and it takes no `--datasets`: a code-free crawl declares
//! its one Dataset, `pages`, itself.
//!
//! A seeds file whose seeds would pass the engine's message limit
//! (`MAX_MESSAGE_BYTES`) in `crawl.start` starts the crawl with the URLs
//! given and as many of the file's seeds as fill half of it, then, once the
//! Run ID is printed, sends the others through `run.add_seeds` on the same
//! Connection, a batch per credits it grants, so no message carries every
//! seed (ADR 0069).
//!
//! Under `--follow` it then reads the Run's progress on the same
//! Connection until the Run ends, and exits by how it ended, as a command
//! that answers a Run report does: by the row of the error a `failed` Run
//! ended with, `20` for a Run that ended `stopped`, and under
//! `--require-verified` `21` for a Verdict other than `verified`. A
//! failure once the Run ID is printed goes to stderr alone, naming the
//! call that failed, so stdout under `--json` holds the Run ID alone (ADR
//! 0069). A first interrupt then stops the Run with a Checkpoint,
//! `run.stop`, and exits `130` once it is answered; a second exits at
//! once.

use std::cell::RefCell;
use std::process::ExitCode;

use clap::builder::{PossibleValue, PossibleValuesParser};
use clap::{Arg, ArgAction, ArgMatches, Command};
use goodfetch_engine::rpc::framing::MAX_MESSAGE_BYTES;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::foxhound::frontier::{RUN_ADD_SEEDS, SEEDS as SEEDS_NOTIFICATION};
use goodfetch_protocol::page::FormatsRequest;
use goodfetch_protocol::run::{Lifecycle, RUN_PROGRESS_READ, RUN_REPORT, RUN_STOP, RunReport};
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use super::{Call, FOLLOW, REQUIRE_VERIFIED, resolved};
use crate::executable::engine_choice::{self, Answer, CommandCall, Daemons, Input, Next, Values};
use crate::executable::exit_codes;
use crate::executable::output::{self, Human, Progress};

/// The operation whose command this is.
pub(super) const OPERATION: &str = "crawl.start";

/// The params its own flags give, and `datasets`, which a crawl with an
/// `extraction` refuses, so its command offers no flag for it.
pub(super) const REPLACES: &[&str] = &[SEEDS, EXTRACTION, DATASETS];

/// Its help, in place of `crawl.start`'s, which tells of a handler's crawl.
pub(super) const ABOUT: &str = "Starts a code-free crawl of its seeds, the URLs given and those of `--seeds-file`, whose Records of the Dataset `pages` hold what its flags read from each page, and prints its Run ID";

/// The param that declares a crawl's Datasets.
const DATASETS: &str = "datasets";

/// How many bytes of the seeds file's seeds `crawl.start` carries at most:
/// half the engine's message limit, the rest of its params' room.
const START_SEEDS_BYTES: usize = MAX_MESSAGE_BYTES / 2;

/// The param that lists the crawl's seeds.
const SEEDS: &str = "seeds";

/// The param that says what a code-free crawl reads from each page.
const EXTRACTION: &str = "extraction";

/// The Label of a code-free crawl, which its seeds take (ADR 0103).
const CODE_FREE_LABEL: &str = "page";

/// The format a crawl given neither a format nor a field keeps.
const DEFAULT_FORMAT: &str = "markdown";

const URL: &str = "url";
const FORMAT: &str = "format";
const FIELD: &str = "field";
const FIELDS: &str = "fields";
const LINKS: &str = "links";
const SEEDS_FILE: &str = "seeds-file";

/// `command` with the flags of `crawl start` that give its seeds and its
/// `extraction`, and `--follow`; the values `--format` and `--links` take
/// are read from the catalog's schemas of `operation`'s params.
pub(super) fn args(command: Command, operation: &Operation) -> Command {
    command
        .arg(
            Arg::new(URL)
                .value_name("URL")
                .num_args(1..)
                .action(ArgAction::Append)
                .required_unless_present(SEEDS_FILE)
                .help("A seed of the crawl, its URL; repeatable"),
        )
        .arg(
            Arg::new(FORMAT)
                .long(FORMAT)
                .value_name("NAME")
                .action(ArgAction::Append)
                .value_parser(PossibleValuesParser::new(
                    format_names().into_iter().map(PossibleValue::new),
                ))
                .help("A format each page's Record holds, as `page.formats` gives it; repeatable; `markdown` when neither a format nor a field is given"),
        )
        .arg(
            Arg::new(FIELD)
                .long(FIELD)
                .value_name("NAME=LOCATOR")
                .action(ArgAction::Append)
                .value_parser(field)
                .help("A named field each page's Record holds: the text of the first element a CSS selector, or an XPath expression after `xpath:`, finds; repeatable"),
        )
        .arg(
            Arg::new(FIELDS)
                .long(FIELDS)
                .value_name("FILE|-")
                .value_parser(fields)
                .help("Reads named fields, each with its locator and its Value transform, as one JSON object by name in FILE, or from standard input for `-`; `--field` beats it"),
        )
        .arg(
            Arg::new(LINKS)
                .long(LINKS)
                .value_name("FOLLOW")
                .value_parser(PossibleValuesParser::new(
                    follow_values(operation).into_iter().map(PossibleValue::new),
                ))
                .help("Which pages the crawl fetches beyond its seeds: every link its scope admits, its sites' sitemaps, or none; `scope` by default"),
        )
        .arg(
            Arg::new(SEEDS_FILE)
                .long(SEEDS_FILE)
                .value_name("FILE|-")
                .value_parser(seeds_file)
                .help("Reads more seeds from FILE, or from standard input for `-`, one per line: a URL, or a seed as JSON"),
        )
        .arg(
            Arg::new(FOLLOW)
                .long(FOLLOW)
                .action(ArgAction::SetTrue)
                .help("Follows the Run's progress until it ends, and exits by how it ended; a first interrupt checkpoints the Run"),
        )
        .arg(
            Arg::new(REQUIRE_VERIFIED)
                .long(REQUIRE_VERIFIED)
                .action(ArgAction::SetTrue)
                .requires(FOLLOW)
                .help("Exits 21 when the Run ended with a Verdict other than `verified`"),
        )
}

/// The formats `page.formats` takes, by name: the fields of its request.
fn format_names() -> Vec<String> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let schema = Value::from(generator.subschema_for::<FormatsRequest>());
    let definitions = Value::Object(generator.take_definitions(true));
    resolved(&schema, &definitions)["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .collect()
}

/// The values `extraction.follow` takes, from `operation`'s params.
fn follow_values(operation: &Operation) -> Vec<String> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let params = Value::from((operation.params)(&mut generator));
    let definitions = Value::Object(generator.take_definitions(true));
    let extraction = resolved(
        &resolved(&params, &definitions)["properties"][EXTRACTION],
        &definitions,
    );
    let follow = resolved(&extraction["properties"]["follow"], &definitions);
    choices(follow, &definitions)
}

/// The strings an enumeration's schema allows: its `enum`, or each of its
/// `oneOf` alternatives' `const` or `enum`.
fn choices(schema: &Value, definitions: &Value) -> Vec<String> {
    let alternatives = schema["oneOf"].as_array().map_or_else(
        || vec![schema],
        |alternatives| alternatives.iter().collect(),
    );
    alternatives
        .into_iter()
        .map(|alternative| resolved(alternative, definitions))
        .flat_map(|alternative| {
            alternative["enum"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| alternative.get("const").cloned().into_iter().collect())
        })
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// Reads `--field`'s `name=locator`: a CSS selector, or one after `css:`,
/// or an `XPath` expression after `xpath:`, as one named field of the
/// catalog's JSON form, given by the `text` Value transform.
fn field(text: &str) -> Result<(String, Value), String> {
    let (name, locator) = text
        .split_once('=')
        .ok_or_else(|| "expected `name=locator`, such as `title=h1`".to_owned())?;
    if name.is_empty() {
        return Err("a field needs a name before `=`".to_owned());
    }
    let (kind, selector) = if let Some(expression) = locator.strip_prefix("xpath:") {
        ("xpath", expression)
    } else {
        ("css", locator.strip_prefix("css:").unwrap_or(locator))
    };
    if selector.is_empty() {
        return Err(format!("the field `{name}` needs a selector after `=`"));
    }
    Ok((
        name.to_owned(),
        json!({ "locator": { "kind": kind, "selector": selector } }),
    ))
}

/// Reads `--fields`' JSON object of named fields from the file `source`
/// names, or from standard input for `-`.
fn fields(source: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str(&read(source)?) {
        Ok(Value::Object(fields)) => Ok(fields),
        _ => Err("expected a JSON object of named fields, each with its `locator`".to_owned()),
    }
}

/// Reads `--seeds-file`'s seeds from the file `source` names, or from
/// standard input for `-`: one per line, a line holding a JSON object a
/// seed as it is, any other a URL; a blank line passed over.
fn seeds_file(source: &str) -> Result<Vec<Value>, String> {
    Ok(read(source)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| match serde_json::from_str(line) {
            Ok(Value::Object(seed)) => Value::Object(seed),
            _ => url_seed(line),
        })
        .collect())
}

/// The text of the file `source` names, or of standard input for `-`,
/// read while clap reads the command line, before any engine starts.
fn read(source: &str) -> Result<String, String> {
    if source == "-" {
        let mut text = String::new();
        #[expect(
            clippy::disallowed_methods,
            reason = "a flag reads standard input while clap reads the command line, before any engine or transport starts"
        )]
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|error| format!("standard input does not read: {error}"))?;
        return Ok(text);
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "the file is read while clap reads the command line, before any runtime starts"
    )]
    std::fs::read_to_string(source).map_err(|error| format!("`{source}` does not read: {error}"))
}

/// The seed of the code-free crawl's Label for `url`.
fn url_seed(url: &str) -> Value {
    json!({ "url": url, "label": CODE_FREE_LABEL })
}

/// What `crawl start`'s own flags gave beside its params.
pub(super) struct Flags {
    /// `--follow`.
    pub(super) follow: bool,
    /// `--require-verified`.
    pub(super) require_verified: bool,
    /// The seeds file's seeds `crawl.start` leaves to `run.add_seeds`.
    pub(super) added: Vec<Value>,
}

/// `seeds` split in two: the first, which `crawl.start` carries, as many
/// as fill `room` bytes once encoded, and at least one when `one_at_least`;
/// and the others.
fn split_seeds(mut seeds: Vec<Value>, room: usize, one_at_least: bool) -> (Vec<Value>, Vec<Value>) {
    let mut used = 0_usize;
    let mut taken = 0;
    for seed in &seeds {
        // Its bytes in the request's array, its comma included.
        let size = seed.to_string().len() + 1;
        if used + size > room && !(one_at_least && taken == 0) {
            break;
        }
        used += size;
        taken += 1;
    }
    let others = seeds.split_off(taken);
    (seeds, others)
}

impl Flags {
    /// Reads the parse `matches`, adding the seeds and the `extraction`
    /// they give to `params`.
    pub(super) fn read(matches: &ArgMatches, params: &mut Map<String, Value>) -> Self {
        let mut seeds: Vec<Value> = matches
            .get_many::<String>(URL)
            .into_iter()
            .flatten()
            .map(|url| url_seed(url))
            .collect();
        let filed = matches
            .get_one::<Vec<Value>>(SEEDS_FILE)
            .cloned()
            .unwrap_or_default();
        // The URLs given take room before the file's seeds, and a crawl
        // starts with one seed at least.
        let given: usize = seeds.iter().map(|seed| seed.to_string().len() + 1).sum();
        let (started, added) = split_seeds(
            filed,
            START_SEEDS_BYTES.saturating_sub(given),
            seeds.is_empty(),
        );
        seeds.extend(started);
        let mut named = matches
            .get_one::<Map<String, Value>>(FIELDS)
            .cloned()
            .unwrap_or_default();
        for (name, field) in matches
            .get_many::<(String, Value)>(FIELD)
            .into_iter()
            .flatten()
        {
            named.insert(name.clone(), field.clone());
        }
        let mut formats: Map<String, Value> = matches
            .get_many::<String>(FORMAT)
            .into_iter()
            .flatten()
            .map(|format| (format.clone(), Value::Bool(true)))
            .collect();
        if formats.is_empty() && named.is_empty() {
            formats.insert(DEFAULT_FORMAT.to_owned(), Value::Bool(true));
        }
        let mut extraction = Map::new();
        extraction.insert("formats".to_owned(), Value::Object(formats));
        if let Some(links) = matches.get_one::<String>(LINKS) {
            extraction.insert("follow".to_owned(), Value::from(links.as_str()));
        }
        if !named.is_empty() {
            extraction.insert("fields".to_owned(), Value::Object(named));
        }
        params.insert(SEEDS.to_owned(), Value::Array(seeds));
        params.insert(EXTRACTION.to_owned(), Value::Object(extraction));
        Self {
            follow: matches.get_flag(FOLLOW),
            require_verified: matches.get_flag(REQUIRE_VERIFIED),
            added,
        }
    }
}

/// Where a `crawl start` is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `crawl.start` is in flight.
    Starting,
    /// `run.add_seeds` is, with the seeds `crawl.start` left out.
    Seeding,
    /// `run.progress` is, until its last value.
    Following,
    /// `run.report` is, once the Run ended.
    Reporting,
}

/// Runs `call`, the command `crawl start`, whose own flags gave `flags`, on
/// the engine its command line chooses, and answers its exit code.
pub(super) fn run(
    call: &Call,
    flags: &Flags,
    engine: &Engine,
    dependencies: Dependencies,
    daemons: &dyn Daemons,
    human: &Human,
) -> ExitCode {
    let json = call.json;
    let run_id: RefCell<Option<Value>> = RefCell::new(None);
    let mut phase = Phase::Starting;
    let mut shown: Option<Progress> = Some(human.progress("crawl start"));
    let started_id: RefCell<Value> = RefCell::new(Value::Null);
    let follow = |started: &Value, phase: &mut Phase| -> Option<Next> {
        if !flags.follow {
            return None;
        }
        *phase = Phase::Following;
        Some(next_call(&RUN_PROGRESS_READ, json!({ "run_id": started })))
    };
    let mut next = |answer: &Value| -> Option<Next> {
        match phase {
            Phase::Starting => {
                shown = None;
                let started = answer["run_id"].clone();
                if json {
                    output::json(&started);
                } else {
                    output::line(started.as_str().unwrap_or_default());
                }
                if flags.follow {
                    *run_id.borrow_mut() = Some(started.clone());
                }
                started.clone_into(&mut started_id.borrow_mut());
                if !flags.added.is_empty() {
                    phase = Phase::Seeding;
                    return Some(next_call(&RUN_ADD_SEEDS, json!({ "run_id": started })));
                }
                follow(&started, &mut phase)
            }
            Phase::Seeding => {
                let started = started_id.borrow().clone();
                follow(&started, &mut phase)
            }
            Phase::Following => {
                let started = answer["run_id"].clone();
                // The last progress shown is erased before the next shows.
                drop(shown.take());
                shown = Some(human.progress(&format!(
                    "crawl {}: {}",
                    started.as_str().unwrap_or_default(),
                    answer["lifecycle"].as_str().unwrap_or_default()
                )));
                if answer["last"] == true {
                    phase = Phase::Reporting;
                    return Some(next_call(&RUN_REPORT, json!({ "run_id": started })));
                }
                Some(next_call(
                    &RUN_PROGRESS_READ,
                    json!({ "run_id": started, "after": answer["position"] }),
                ))
            }
            Phase::Reporting => None,
        }
    };
    let mut stop = || {
        let started = run_id.borrow().clone()?;
        Some(next_call(&RUN_STOP, json!({ "run_id": started })))
    };
    let params = Value::Object(call.params.clone());
    let mut added = flags.added.iter().cloned();
    let command_call = CommandCall {
        method: call.operation.name,
        params,
        cancellable: call.operation.cancellable,
        follower: None,
        pager: Some(&mut next),
        input: Some(Input {
            method: SEEDS_NOTIFICATION,
            field: SEEDS,
            values: Values::Held(&mut added),
        }),
        stopper: Some(&mut stop),
    };
    let warn = |text: &str| human.warning(text);
    let reach = call.reach(daemons, &warn);
    let answer = engine_choice::call(engine, dependencies, call.product, &reach, command_call);
    drop(shown);
    // Once the Run ID is out, a failure goes to stderr alone, naming the
    // call in flight.
    let failing = match phase {
        Phase::Starting => None,
        Phase::Seeding => Some(RUN_ADD_SEEDS.name),
        Phase::Following => Some(RUN_PROGRESS_READ.name),
        Phase::Reporting => Some(RUN_REPORT.name),
    };
    match (answer, failing) {
        (Answer::Result(_), _) if !flags.follow => ExitCode::from(exit_codes::SUCCESS),
        (Answer::Result(report), _) => ended(report, flags.require_verified, json, human),
        (Answer::Error(error), None) => call.fail(error, human),
        (Answer::Error(error), Some(operation)) => {
            let exit = exit_codes::of_error(&error.data.code, exit_codes::group(&error));
            output::failed_after(&output::command_failed(error, Some(operation)), json, human);
            ExitCode::from(exit)
        }
        (Answer::Refused(error), _) => call.fail_exiting(error, exit_codes::DAEMON_REFUSED, human),
        (Answer::Unanswered(error), _) => output::fail(error, None, json, human),
        (Answer::Interrupted(interrupt), _) => ExitCode::from(interrupt.exit_code()),
    }
}

/// The exit code of a followed crawl whose `run.report` answered `report`:
/// by how its Run ended (ADR 0069), and for a Run whose ending raised an
/// error, that error, printed on stderr as `run.report`'s, as `run finish`
/// fails with it.
fn ended(report: Value, require_verified: bool, json: bool, human: &Human) -> ExitCode {
    let report = match serde_json::from_value::<RunReport>(report) {
        Ok(report) => report,
        Err(problem) => {
            let error = exit_codes::internal(format!(
                "The engine answered `{}` with no Run report: {problem}.",
                RUN_REPORT.name
            ));
            output::failed_after(
                &output::command_failed(error, Some(RUN_REPORT.name)),
                json,
                human,
            );
            return ExitCode::from(exit_codes::unanswered());
        }
    };
    let error = match (&report.error, report.lifecycle) {
        (Some(error), _) => Some(exit_codes::of_ending(error)),
        (None, Lifecycle::Failed) => Some(exit_codes::internal(format!(
            "The crawl `{}` ended `failed`, and its report names no error.",
            report.run_id
        ))),
        (None, _) => None,
    };
    if let Some(error) = error {
        output::failed_after(
            &output::command_failed(error, Some(RUN_REPORT.name)),
            json,
            human,
        );
    }
    ExitCode::from(exit_codes::of_report(&report, require_verified))
}

/// The call of `operation`, a Run's read or its stop, with `params`.
fn next_call(operation: &Operation, params: Value) -> Next {
    Next {
        method: operation.name,
        params,
        cancellable: operation.cancellable,
        followed: false,
    }
}
