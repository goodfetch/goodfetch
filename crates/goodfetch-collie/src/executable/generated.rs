//! The commands generated from the executable's Product's catalog, Collie's
//! or Foxhound's, which holds Collie's (ADRs 0090 and 0099): every operation
//! [`offer`] gives a command, whose namespace no hand-written command group
//! takes, is `<namespace> <member>`, words joined by `-`, whose flags are
//! its params, `--<parameter>` in kebab-case, a value-bound command taking
//! its value's ID (`run_id` for `run`) as its first positional argument,
//! a `timeout` param being `--timeout <duration>` or `--timeout none`
//! (ADR 0080), and `--json` printing the result as JSON; an operation that
//! takes `settings` takes them as the [`SettingsFlags`]. A sequence's
//! command takes `--follow`. Help comes from the catalog's descriptions.
//! Foxhound's executable carries Collie's commands unchanged and adds its
//! crawl's (ADR 0090): a sequence of what a crawl committed, such as `run
//! records`, prints what it sent once it ended, one JSON array under
//! `--json`, unless `--follow` is given; a round trip that answers pages
//! of what a crawl committed, `run outcomes` or `run rejected-records`,
//! follows them page after page under `--follow`, one JSON object per
//! line, until the Run ends; and
//! `run add-seeds` reads its seeds from stdin, one per line. Two of its
//! commands take flags of their own in place of some params: `crawl start
//! <url>…`, which builds the code-free crawl's `extraction` and seeds from
//! them and follows its Run under `--follow` ([`crawl_start`]), and `run
//! export <run> <dataset> --out <path|->`, which writes the Export's file
//! ([`run_export`]).
//!
//! A command calls its operation on the engine [`engine_choice`] gives it
//! and prints the result: an opener prints the new value's ID, any other
//! the result's fields one per line, a point in time in the local zone,
//! and `--json` the result itself, in UTC. While the call runs, its words
//! show as progress on a terminal. Under `--follow` it prints each event or
//! chunk the call sends as it arrives, `name=value` fields on one line, or
//! one JSON object per line under `--json`, until the call ends. A failure
//! prints its message, code, retryability and `fix` on stderr, or under
//! `--json` [`CommandFailed`](goodfetch_protocol::cli::CommandFailed) on
//! stdout, and exits with its row of
//! [`exit_codes`]. A command whose operation answers a Run report exits by
//! how the Run ended ([`exit_codes::of_report`]): `20` for a Run that ended
//! `stopped`, and under its `--require-verified` `21` for a Verdict other
//! than `verified`. An interrupt exits with [`exit_codes::INTERRUPTED`],
//! `SIGTERM` with [`exit_codes::TERMINATED`]. A deprecated command warns on
//! stderr with ADR 0030's text, which its `--help` shows too.
//!
//! Size: one concern, a catalog entry turned into a command and its call:
//! the schema readers that give each command its flags, its offer and its
//! output share the entry's schemas, and splitting them would only pass
//! those schemas between modules.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::marker::PhantomData;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

use clap::error::ErrorKind;
use clap::{Arg, ArgAction, ArgMatches, Args as _, Command, FromArgMatches, Subcommand};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::{
    Deprecation, Operation, OperationKind, Product, Reach, SDK_ONLY,
};
use goodfetch_protocol::entries::operations_of;
use goodfetch_protocol::events::EVENTS;
use goodfetch_protocol::foxhound_names;
use goodfetch_protocol::jsonrpc::ErrorObject;
use goodfetch_protocol::run::RunReport;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use super::engine_choice::{
    self, Answer, Choice, CommandCall, Daemons, EngineChoice, Follower, Input, Lifetime, Next,
    Pager, ReadValues, Values,
};
use super::output::{ColourFlag, Human};
use super::settings_flags::{self, SettingsFlags};
use super::{exit_codes, output};

mod crawl_start;
mod run_export;

/// A command whose flags the CLI writes by hand in place of some of its
/// operation's params, beside the flags generated for the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Own {
    /// `crawl start <url>…` ([`crawl_start`]).
    CrawlStart,
    /// `run export <run> <dataset> --out <path|->` ([`run_export`]).
    RunExport,
}

impl Own {
    /// `operation`'s, when its command takes flags of its own.
    fn of(operation: &Operation) -> Option<Self> {
        match operation.name {
            crawl_start::OPERATION => Some(Self::CrawlStart),
            run_export::OPERATION => Some(Self::RunExport),
            _ => None,
        }
    }

    /// The params its own flags give, or it leaves out, which get no
    /// generated flag.
    fn replaces(self) -> &'static [&'static str] {
        match self {
            Self::CrawlStart => crawl_start::REPLACES,
            Self::RunExport => run_export::REPLACES,
        }
    }

    /// Its own help, in place of its operation's description, if it has
    /// one.
    fn about(self) -> Option<&'static str> {
        match self {
            Self::CrawlStart => Some(crawl_start::ABOUT),
            Self::RunExport => None,
        }
    }

    /// `command` with its own flags.
    fn args(self, command: Command, operation: &Operation) -> Command {
        match self {
            Self::CrawlStart => crawl_start::args(command, operation),
            Self::RunExport => run_export::args(command, operation),
        }
    }

    /// What its own flags gave in the parse `matches`: the params they
    /// give, added to `params`, and what its call does with the rest.
    fn read(self, matches: &ArgMatches, params: &mut Map<String, Value>) -> OwnFlags {
        match self {
            Self::CrawlStart => OwnFlags::CrawlStart(crawl_start::Flags::read(matches, params)),
            Self::RunExport => OwnFlags::RunExport(run_export::Flags::read(matches, params)),
        }
    }
}

/// What a command's own flags gave beside its params.
enum OwnFlags {
    CrawlStart(crawl_start::Flags),
    RunExport(run_export::Flags),
}

/// The Product whose catalog the commands of a hand-written group's
/// namespace come from ([`Joining`]): Collie's, since Foxhound adds no
/// operation to a namespace a hand-written group takes.
const JOINED_PRODUCT: Product = Product::Collie;

/// The operations whose call takes a stream of values from the Client
/// (ADR 0024), with the notification that carries them and the field that
/// lists them: a command reads them from stdin, one per line.
const INPUTS: [(&str, &str, &str); 1] = [("run.add_seeds", "$/seeds", "seeds")];

/// The flag that prints the result as JSON.
const JSON: &str = "json";

/// The flag that runs a sequence, printing what it sends as it arrives.
const FOLLOW: &str = "follow";

/// The flag that runs the call on an Attached engine in this process,
/// whatever runs (ADR 0024).
const ATTACHED: &str = "attached";

/// The flag that runs the call on the Daemon, started when none runs, whose
/// refusal ends the command (ADRs 0024 and 0098).
const DAEMON: &str = "daemon";

/// The flag of a command that answers a Run report that exits `21` when
/// the Run's Verdict is not `verified` (ADR 0069).
const REQUIRE_VERIFIED: &str = "require-verified";

/// The type in the bundle's `$defs` of a Run report, by which a command's
/// exit code follows how its Run ended.
const RUN_REPORT: &str = "RunReport";

/// The param that is a call's own timeout (ADR 0080): an operation that may
/// wait takes it, in milliseconds or `null` for none, and one that never
/// waits does not.
const TIMEOUT: &str = "timeout";

/// The command line's spelling of no timeout, the null form (ADR 0080).
const NO_TIMEOUT: &str = "none";

/// The param that takes the run layer's settings (ADR 0083), which a
/// command takes as its [`SettingsFlags`].
const SETTINGS: &str = "settings";

/// A namespace of the catalog with commands: the group of its commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Namespace {
    /// As the catalog writes it, such as `crawl_request`.
    name: &'static str,
    /// As the command line spells it (ADR 0099), such as `crawl-request`.
    spelled: &'static str,
    /// The group's help.
    about: &'static str,
    /// The Product whose catalog first gives it a command: Collie for a
    /// namespace both executables carry, Foxhound for one only Foxhound's
    /// carries.
    product: Product,
}

/// Every namespace of Foxhound's commands' operations, which hold Collie's,
/// once each, in the catalog's order, Collie's first: what
/// [`namespaces_source`] gives, which `xtask codegen` writes, so that a
/// start walks no catalog (ADR 0071's startup figure).
const NAMESPACES: &[Namespace] = &include!("../../generated/namespaces.rs");

/// The source of [`NAMESPACES`], from the catalog, as `xtask codegen` writes
/// it.
pub fn namespaces_source() -> String {
    let mut names: Vec<&'static str> = Vec::new();
    // A namespace's operations may sit in several modules of the catalog,
    // such as `run.new_page` among the page's: one group each.
    for name in commands(Product::Foxhound).map(namespace) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let mut source = String::from(
        "// Generated by `xtask codegen` from Foxhound's catalog, which holds Collie's:\n// the namespaces of its commands; never edit by hand.\n[\n",
    );
    for name in names {
        let (spelled, about) = (
            spelled(name),
            format!("Calls the catalog's `{name}` operations"),
        );
        let product = if commands(Product::Collie).any(|operation| namespace(operation) == name) {
            "Collie"
        } else {
            "Foxhound"
        };
        let _ = write!(
            source,
            "    Namespace {{\n        name: {name:?},\n        spelled: {spelled:?},\n        about: {about:?},\n        product: Product::{product},\n    }},\n"
        );
    }
    source.push_str("]\n");
    source
}

/// The generated commands beside an executable's hand-written ones.
pub struct Generated {
    /// The Product of the executable, whose catalog they come from.
    product: Product,
    /// The namespaces they cover: those no hand-written group takes.
    namespaces: Vec<&'static Namespace>,
}

impl Generated {
    /// The commands of `product`'s catalog in every namespace no command
    /// group of `hand_written` takes; a hand-written group takes the other
    /// commands of its namespace through [`Joining`]. Foxhound's executable
    /// carries Collie's commands unchanged, and its own beside them (ADR
    /// 0090).
    pub fn beside(product: Product, hand_written: &Command) -> Self {
        let namespaces = NAMESPACES
            .iter()
            .filter(|namespace| product.contains(namespace.product))
            .filter(|namespace| hand_written.find_subcommand(namespace.spelled).is_none())
            .collect();
        Self {
            product,
            namespaces,
        }
    }

    /// `command` with a command group per namespace, whose commands are
    /// built only when clap reaches the group.
    #[must_use]
    pub fn add_to(&self, command: Command) -> Command {
        let members: fn(Command) -> Command = match self.product {
            Product::Collie => collies_members,
            Product::Foxhound => foxhounds_members,
        };
        command.subcommands(self.namespaces.iter().map(|namespace| {
            Command::new(namespace.spelled)
                .about(namespace.about)
                .subcommand_required(true)
                .defer(members)
        }))
    }

    /// The call a parse of the command line asks for, when its command is
    /// generated in a group of its own.
    pub fn call(&self, matches: &ArgMatches) -> Option<Call> {
        let (group, group_matches) = matches.subcommand()?;
        let namespace = self
            .namespaces
            .iter()
            .find(|namespace| namespace.spelled == group)?;
        let (member, member_matches) = group_matches.subcommand()?;
        call_in(self.product, namespace.name, member, member_matches)
    }
}

/// The call of `namespace`'s command `member` of `product`'s catalog, from
/// its parse `matches`, when that command is generated.
fn call_in(product: Product, namespace: &str, member: &str, matches: &ArgMatches) -> Option<Call> {
    let name = format!("{namespace}.{}", member.replace('-', "_"));
    let operation = commands(product).find(|operation| operation.name == name)?;
    Some(Call::of(product, operation, matches))
}

/// The hand-written commands of a command group that takes a namespace of
/// the catalog, which the namespace's generated commands join through
/// [`Joining`].
pub trait Joined: clap::Subcommand {
    /// The namespace, as the catalog writes it, such as `usage`.
    const NAMESPACE: &'static str;
}

/// The generated commands of the namespace of the hand-written commands
/// `Own` that `Own` lacks, in their group: `#[command(flatten)]` it before
/// `Own` among the group's commands, so that they list first. `Own`'s
/// commands win over a generated one of the same name, such as a
/// person-only prompt like `usage raise` beside `usage list`. Like the
/// group's own commands, they are built only when clap reaches the group,
/// so a start builds none (ADR 0071's startup figure).
pub struct Joining<Own> {
    call: Call,
    own: PhantomData<Own>,
}

impl<Own: Joined> Joining<Own> {
    /// The call its command asks for.
    #[must_use]
    pub fn call(self) -> Call {
        self.call
    }
}

// Each generic function only passes `Own`'s namespace and own commands on
// to one that is not: a generic one reading the catalog would copy its
// tables into the executable's crate, about 700 more relocations for the
// loader to apply at every start (ADR 0071's startup figure).

impl<Own: Joined> Subcommand for Joining<Own> {
    fn augment_subcommands(group: Command) -> Command {
        joined_members(group, Own::NAMESPACE, Own::has_subcommand)
    }

    fn augment_subcommands_for_update(group: Command) -> Command {
        Self::augment_subcommands(group)
    }

    fn has_subcommand(name: &str) -> bool {
        joins(Own::NAMESPACE, Own::has_subcommand, name)
    }
}

impl<Own: Joined> FromArgMatches for Joining<Own> {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        let call = joined_call(Own::NAMESPACE, Own::has_subcommand, matches)
            .ok_or_else(|| clap::Error::new(ErrorKind::MissingSubcommand))?;
        Ok(Self {
            call,
            own: PhantomData,
        })
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

/// The operations of `namespace` whose commands join its hand-written
/// group, whose own commands `own` names.
fn joining(
    namespace: &'static str,
    own: fn(&str) -> bool,
) -> impl Iterator<Item = &'static Operation> {
    commands(JOINED_PRODUCT).filter(move |operation| {
        self::namespace(operation) == namespace && !own(&spelled(member(operation)))
    })
}

/// `group`, the hand-written group of `namespace` whose own commands `own`
/// names, with the generated commands that join it.
fn joined_members(group: Command, namespace: &'static str, own: fn(&str) -> bool) -> Command {
    members_of(group, joining(namespace, own))
}

/// Whether `name` is a generated command that joins the hand-written group
/// of `namespace`, whose own commands `own` names.
fn joins(namespace: &'static str, own: fn(&str) -> bool, name: &str) -> bool {
    joining(namespace, own).any(|operation| spelled(member(operation)) == name)
}

/// The call of the generated command that the parse `matches` of the
/// hand-written group of `namespace`, whose own commands `own` names, asks
/// for.
fn joined_call(
    namespace: &'static str,
    own: fn(&str) -> bool,
    matches: &ArgMatches,
) -> Option<Call> {
    let (member, matches) = matches.subcommand()?;
    if !joins(namespace, own, member) {
        return None;
    }
    call_in(JOINED_PRODUCT, namespace, member, matches)
}

/// How the CLI offers an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// A command that prints the call's result.
    Command,
    /// A command that takes `--follow` and prints what the call sends as
    /// it arrives, until the call ends: a sequence.
    Follow,
    /// A sequence of what a crawl committed (ADR 0099): its command prints
    /// what the call sent once it ends, one JSON array under `--json`, or
    /// under `--follow` as it arrives.
    Committed,
    /// A round trip that answers one page of what a crawl committed at a time,
    /// each starting `after` the last's `next`, the last `ended`: under
    /// `--follow` its command asks for page after page, printing each
    /// item as it arrives, until the page that says `ended`.
    Pages,
    /// A sequence whose call takes a stream of values from the Client
    /// ([`INPUTS`]): its command reads them from stdin, one per line, and
    /// prints what the call answers each batch with as it arrives.
    Input,
}

/// How the CLI offers `operation`, if it does (ADR 0099): every round trip,
/// query, read and opener is a command, a round trip that answers pages
/// ([`Offer::Pages`]) one that follows them under `--follow`, and every
/// sequence one with `--follow`, a sequence of Foxhound's, which reads
/// what a crawl committed, printing it once it ends without one
/// ([`Offer::Committed`]), and one taking a stream reading it from stdin
/// ([`Offer::Input`]), unless the catalog keeps it off the CLI, to the
/// SDKs or to a person; a waiter or an event is none. An operation the catalog's
/// [`SDK_ONLY`] list names is none whatever its marker, so an entry added
/// without one never becomes a command.
pub fn offer(operation: &Operation) -> Option<Offer> {
    if operation.reach != Reach::AllSurfaces || SDK_ONLY.contains(&operation.name) {
        return None;
    }
    match operation.kind {
        OperationKind::RoundTrip if answers_pages(operation) => Some(Offer::Pages),
        OperationKind::RoundTrip
        | OperationKind::Query
        | OperationKind::Read
        | OperationKind::Opener => Some(Offer::Command),
        OperationKind::Sequence if INPUTS.iter().any(|(name, _, _)| *name == operation.name) => {
            Some(Offer::Input)
        }
        // Foxhound's sequences read what a crawl committed, so each ends
        // once it sent the Records of a Run that ended.
        OperationKind::Sequence if foxhound_names::OPERATIONS.contains(&operation.name) => {
            Some(Offer::Committed)
        }
        OperationKind::Sequence => Some(Offer::Follow),
        OperationKind::Waiter | OperationKind::Event => None,
    }
}

/// The operations of `product`'s catalog that are commands, in the
/// catalog's order.
fn commands(product: Product) -> impl Iterator<Item = &'static Operation> {
    operations_of(product).filter(|operation| offer(operation).is_some())
}

fn namespace(operation: &Operation) -> &'static str {
    operation
        .name
        .split_once('.')
        .map_or(operation.name, |(namespace, _)| namespace)
}

fn member(operation: &Operation) -> &'static str {
    operation
        .name
        .split_once('.')
        .map_or(operation.name, |(_, member)| member)
}

/// A canonical name in the CLI's form: words joined by `-` (ADR 0099).
fn spelled(name: &str) -> String {
    name.replace('_', "-")
}

/// The commands of the group `group` of Collie's executable.
fn collies_members(group: Command) -> Command {
    members(group, Product::Collie)
}

/// The commands of the group `group` of Foxhound's executable.
fn foxhounds_members(group: Command) -> Command {
    members(group, Product::Foxhound)
}

/// The commands of the group `group`, one per operation of its namespace
/// in `product`'s catalog, and the group's `--color`, which clap copies
/// into them (see [`ColourFlag`]).
fn members(group: Command, product: Product) -> Command {
    members_of(ColourFlag::augment_args(group), operations_of(product))
}

/// The commands of the group `group`, one per operation of `operations` in
/// its namespace that the CLI offers.
fn members_of(group: Command, operations: impl Iterator<Item = &'static Operation>) -> Command {
    let namespace = group.get_name().replace('-', "_");
    group.subcommands(
        operations
            .filter(|operation| offer(operation).is_some())
            .filter(|operation| self::namespace(operation) == namespace)
            .map(command),
    )
}

/// The command of `operation`, its help in the form of the hand-written
/// commands': the description with no closing period.
fn command(operation: &'static Operation) -> Command {
    let mut command = Command::new(spelled(member(operation)));
    let json_help = match offer(operation) {
        Some(Offer::Follow) => {
            command = command.arg(
                Arg::new(FOLLOW)
                    .long(FOLLOW)
                    .action(ArgAction::SetTrue)
                    .required(true)
                    .help("Prints what the call sends as it arrives, until it ends"),
            );
            "Prints what the call sends as one JSON object per line"
        }
        Some(Offer::Committed) => {
            command = command.arg(
                Arg::new(FOLLOW)
                    .long(FOLLOW)
                    .action(ArgAction::SetTrue)
                    .help("Prints what the call sends as it arrives, until it ends, rather than once it ended"),
            );
            "Prints what the call sent as one JSON array, or under `--follow` one JSON object per line"
        }
        Some(Offer::Pages) => {
            command = command.arg(
                Arg::new(FOLLOW)
                    .long(FOLLOW)
                    .action(ArgAction::SetTrue)
                    .help("Asks for page after page, printing each item as it arrives, until the Run ends"),
            );
            "Prints the result as JSON, or under `--follow` each item as one JSON object per line"
        }
        Some(Offer::Input) => {
            "Prints what the call answers each batch of stdin's lines with as one JSON object per line"
        }
        Some(Offer::Command) | None => "Prints the result as JSON",
    };
    let own = Own::of(operation);
    let command = generated_parameters(operation, own).fold(command, |command, parameter| {
        match parameter.form {
            Form::Settings => SettingsFlags::augment_args(command),
            _ => command.arg(parameter.arg()),
        }
    });
    let command = match own {
        Some(own) => own.args(command, operation),
        None => command,
    };
    let command = command
        .arg(
            Arg::new(JSON)
                .long(JSON)
                .action(ArgAction::SetTrue)
                .help(json_help),
        )
        .arg(
            Arg::new(ATTACHED)
                .long(ATTACHED)
                .action(ArgAction::SetTrue)
                .conflicts_with(DAEMON)
                .help("Runs the call on an Attached engine in this process, even when a Daemon runs"),
        )
        .arg(
            Arg::new(DAEMON)
                .long(DAEMON)
                .action(ArgAction::SetTrue)
                .help("Runs the call on the Daemon of the major, starting it if none runs; its refusal ends the command"),
        );
    let command = if answers_a_run_report(operation) {
        command.arg(
            Arg::new(REQUIRE_VERIFIED)
                .long(REQUIRE_VERIFIED)
                .action(ArgAction::SetTrue)
                .help("Exits 21 when the Run ended with a Verdict other than `verified`"),
        )
    } else {
        command
    };
    let about = own
        .and_then(Own::about)
        .map_or_else(|| sentence(operation.description), str::to_owned);
    let command = command
        // After the settings flags, whose derived arguments set an `about`
        // of their own.
        .about(about);
    match operation.deprecated {
        Some(deprecated) => command.after_help(deprecation(deprecated)),
        None => command,
    }
}

/// ADR 0030's text for a deprecation, which the warning and `--help` share:
/// since when, the earliest removal, and the note naming the replacement.
fn deprecation(deprecated: Deprecation) -> String {
    format!(
        "Deprecated since protocol {}, and removed no earlier than the next major: {}",
        deprecated.since, deprecated.note
    )
}

/// A call's timeout as the command line gives it (ADR 0080): `none` for no
/// timeout, or a positive duration in milliseconds or with a unit (`250ms`,
/// `45s`, `2m`, `1h`), as milliseconds. `0` reads as "at once", so it is
/// refused, as is anything that is not a positive duration.
fn duration(text: &str) -> Result<Value, String> {
    if text == NO_TIMEOUT {
        return Ok(Value::Null);
    }
    match settings_flags::milliseconds(text) {
        Some(0) => Err(format!(
            "a timeout of 0 would end the call at once: give `--timeout {NO_TIMEOUT}` for no timeout"
        )),
        Some(milliseconds) => Ok(Value::from(milliseconds)),
        None => Err(format!(
            "`{text}` is no duration: give milliseconds, or a whole number with `ms`, `s`, `m` or `h`, such as `45s`, `2m` or `1h`, or `{NO_TIMEOUT}`"
        )),
    }
}

/// `text` on one line, with no closing period, as clap shows doc comments.
fn sentence(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_owned()
}

/// One param of an operation, as the command line takes it.
struct Parameter {
    /// Its canonical name.
    name: String,
    /// How it is given.
    form: Form,
    /// Whether the call needs it.
    required: bool,
    /// Its description.
    help: String,
}

/// How a param is given on the command line.
enum Form {
    /// The value's ID, first and positional.
    Value,
    /// A flag with no value: `true` when given.
    Switch,
    /// A flag of an optional boolean, which takes `true` or `false`.
    Boolean,
    /// A flag with a text.
    Text,
    /// A flag with a value read as JSON when it parses as JSON, as text
    /// otherwise.
    Json,
    /// A repeatable flag, each value read as `Text` or `Json`.
    Repeated { json: bool },
    /// The call's timeout (ADR 0080), read by [`duration`].
    Timeout,
    /// The run layer's `settings`: `--set` and `--settings`, the
    /// [`SettingsFlags`].
    Settings,
}

impl Parameter {
    fn arg(&self) -> Arg {
        let arg = Arg::new(self.name.clone())
            .help(self.help.clone())
            .value_name(self.name.to_uppercase());
        match self.form {
            Form::Value => arg.required(true),
            Form::Switch => arg.long(spelled(&self.name)).action(ArgAction::SetTrue),
            Form::Boolean => arg
                .long(spelled(&self.name))
                .value_name("true|false")
                .required(self.required)
                .action(ArgAction::Set)
                .value_parser(clap::builder::BoolValueParser::new()),
            Form::Text | Form::Json => arg
                .long(spelled(&self.name))
                .required(self.required)
                .action(ArgAction::Set),
            Form::Repeated { .. } | Form::Settings => arg
                .long(spelled(&self.name))
                .required(self.required)
                .action(ArgAction::Append),
            Form::Timeout => arg
                .long(spelled(&self.name))
                .value_name("DURATION")
                .required(self.required)
                .action(ArgAction::Set)
                .value_parser(duration),
        }
    }

    /// Its value in a parse, when given.
    fn value(&self, matches: &ArgMatches) -> Option<Value> {
        let read = |text: &String, json: bool| {
            if json {
                serde_json::from_str(text).unwrap_or_else(|_| Value::from(text.as_str()))
            } else {
                Value::from(text.as_str())
            }
        };
        match self.form {
            Form::Switch => matches.get_flag(&self.name).then_some(Value::Bool(true)),
            Form::Boolean => matches
                .get_one::<bool>(&self.name)
                .copied()
                .map(Value::Bool),
            Form::Value | Form::Text => matches
                .get_one::<String>(&self.name)
                .map(|text| read(text, false)),
            Form::Json => matches
                .get_one::<String>(&self.name)
                .map(|text| read(text, true)),
            Form::Repeated { json } => matches
                .get_many::<String>(&self.name)
                .map(|values| Value::Array(values.map(|text| read(text, json)).collect())),
            Form::Timeout => matches.get_one::<Value>(&self.name).cloned(),
            Form::Settings => {
                let run_layer = SettingsFlags::from_arg_matches(matches).ok()?.run_layer();
                (!run_layer.is_empty()).then_some(Value::Object(run_layer))
            }
        }
    }
}

/// The params of `operation`, from its params' JSON Schema.
fn parameters(operation: &Operation) -> Vec<Parameter> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let params = Value::from((operation.params)(&mut generator));
    let definitions = Value::Object(generator.take_definitions(true));
    let params = resolved(&params, &definitions);
    let required: Vec<&str> = params["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let value_id = format!("{}_id", namespace(operation));
    let Some(properties) = params["properties"].as_object() else {
        return Vec::new();
    };
    let mut parameters: Vec<Parameter> = properties
        .iter()
        .map(|(name, schema)| {
            let required = required.contains(&name.as_str());
            let type_of = resolved(schema, &definitions);
            let form = if *name == value_id && required {
                Form::Value
            } else if name == TIMEOUT {
                Form::Timeout
            } else if name == SETTINGS {
                Form::Settings
            } else {
                match type_name(type_of) {
                    // An optional boolean has three states, so its flag
                    // takes `true` or `false`, and its absence leaves the
                    // param out.
                    Some("boolean") if nullable(schema, &definitions) => Form::Boolean,
                    Some("boolean") => Form::Switch,
                    Some("string") => Form::Text,
                    Some("array") => Form::Repeated {
                        json: type_name(resolved(&type_of["items"], &definitions))
                            != Some("string"),
                    },
                    _ => Form::Json,
                }
            };
            let help = schema["description"]
                .as_str()
                .or_else(|| type_of["description"].as_str())
                .unwrap_or_default();
            Parameter {
                name: name.clone(),
                form,
                required,
                help: sentence(help),
            }
        })
        .collect();
    // The value's ID comes first.
    parameters.sort_by_key(|parameter| !matches!(parameter.form, Form::Value));
    parameters
}

/// The params of `operation` its command takes as generated flags: all
/// but those its own flags give.
fn generated_parameters(
    operation: &Operation,
    own: Option<Own>,
) -> impl Iterator<Item = Parameter> {
    let replaced = own.map_or(&[][..], Own::replaces);
    parameters(operation)
        .into_iter()
        .filter(move |parameter| !replaced.contains(&parameter.name.as_str()))
}

/// Whether `operation` answers a Run report, by which its command exits.
fn answers_a_run_report(operation: &Operation) -> bool {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let result = Value::from((operation.result)(&mut generator));
    result["$ref"].as_str() == Some(&format!("#/$defs/{RUN_REPORT}"))
}

/// The fields of `operation`'s result that are points in time, from its
/// result's JSON Schema (`format: date-time`).
fn times(operation: &Operation) -> Vec<String> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let result = Value::from((operation.result)(&mut generator));
    let definitions = Value::Object(generator.take_definitions(true));
    let result = resolved(&result, &definitions);
    result["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, schema)| resolved(schema, &definitions)["format"] == "date-time")
        .map(|(name, _)| name.clone())
        .collect()
}

/// `schema` with its references followed and an optional value's `null`
/// alternative dropped.
fn resolved<'a>(schema: &'a Value, definitions: &'a Value) -> &'a Value {
    if let Some(reference) = schema["$ref"].as_str()
        && let Some(name) = reference.strip_prefix("#/$defs/")
    {
        return resolved(&definitions[name], definitions);
    }
    if let Some(alternatives) = schema["anyOf"].as_array() {
        let mut values = alternatives
            .iter()
            .filter(|alternative| alternative["type"] != "null");
        if let (Some(value), None) = (values.next(), values.next()) {
            return resolved(value, definitions);
        }
    }
    schema
}

/// Whether `schema`, followed through its references, allows `null`
/// beside its value: an optional value's.
fn nullable(schema: &Value, definitions: &Value) -> bool {
    if let Some(reference) = schema["$ref"].as_str()
        && let Some(name) = reference.strip_prefix("#/$defs/")
    {
        return nullable(&definitions[name], definitions);
    }
    let named_null = match &schema["type"] {
        Value::Array(names) => names.iter().any(|name| name == "null"),
        _ => false,
    };
    named_null
        || schema["anyOf"].as_array().is_some_and(|alternatives| {
            alternatives
                .iter()
                .any(|alternative| alternative["type"] == "null")
        })
}

/// The JSON type a schema names, an optional value's `null` aside.
fn type_name(schema: &Value) -> Option<&str> {
    match &schema["type"] {
        Value::String(name) => Some(name),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null"),
        _ => None,
    }
}

/// A generated command's call, as the command line gave it.
pub struct Call {
    /// The Product of the executable, which its `hello` needs.
    product: Product,
    operation: &'static Operation,
    params: Map<String, Value>,
    json: bool,
    follow: bool,
    /// Whether it answers a Run report, and if so whether
    /// `--require-verified` was given.
    report: Option<bool>,
    /// The engine `--attached` or `--daemon` asks for.
    choice: Choice,
    /// What its command's own flags gave, when it takes some.
    own: Option<OwnFlags>,
}

impl Call {
    /// The call of `operation` that its command's parse `matches` gives, on
    /// `product`'s executable.
    fn of(product: Product, operation: &'static Operation, matches: &ArgMatches) -> Self {
        let own = Own::of(operation);
        let mut params = generated_parameters(operation, own)
            .filter_map(|parameter| {
                let value = parameter.value(matches)?;
                Some((parameter.name, value))
            })
            .collect();
        let own = own.map(|own| own.read(matches, &mut params));
        let follows = matches!(
            offer(operation),
            Some(Offer::Follow | Offer::Committed | Offer::Pages)
        );
        Self {
            product,
            operation,
            params,
            json: matches.get_flag(JSON),
            follow: follows && matches.get_flag(FOLLOW),
            report: answers_a_run_report(operation).then(|| matches.get_flag(REQUIRE_VERIFIED)),
            choice: if matches.get_flag(ATTACHED) {
                Choice::Attached
            } else if matches.get_flag(DAEMON) {
                Choice::Daemon
            } else {
                Choice::Auto
            },
            own,
        }
    }

    /// The engine its call runs on: as its command line and its value
    /// choose, its refusals warned of through `warn`.
    fn reach<'a>(&self, daemons: &'a dyn Daemons, warn: &'a dyn Fn(&str)) -> EngineChoice<'a> {
        EngineChoice {
            choice: self.choice,
            lifetime: self.lifetime(),
            daemons,
            warn,
        }
    }

    /// What its call needs of its engine: a long-lived one when its value
    /// outlives the command, as an opener's new value or the value a
    /// value-bound command names does (ADR 0088).
    fn lifetime(&self) -> Lifetime {
        let value_bound = parameters(self.operation)
            .iter()
            .any(|parameter| matches!(parameter.form, Form::Value));
        if self.operation.kind == OperationKind::Opener || value_bound {
            Lifetime::Lasting
        } else {
            Lifetime::Passing
        }
    }

    /// Calls the operation on the engine its command line and its value
    /// choose ([`engine_choice`]): the executable's Daemon, `daemons`, or
    /// an Attached engine in this process, `engine` on `dependencies`;
    /// prints what it answered as `human` shows it, or under `--follow`
    /// what it sent, and answers the exit code.
    #[expect(
        clippy::too_many_lines,
        reason = "the call's follower, pager and input are closures over the state its end reads; splitting them would pass that state between functions"
    )]
    pub fn run(
        mut self,
        engine: &Engine,
        dependencies: Dependencies,
        daemons: &dyn Daemons,
        human: &Human,
    ) -> ExitCode {
        let words = self.operation.name.replace('.', " ").replace('_', "-");
        if let Some(deprecated) = self.operation.deprecated {
            human.warning(&format!("`{words}`: {}", deprecation(deprecated)));
        }
        match self.own.take() {
            Some(OwnFlags::CrawlStart(flags)) => {
                return crawl_start::run(&self, &flags, engine, dependencies, daemons, human);
            }
            Some(OwnFlags::RunExport(flags)) => {
                return run_export::run(&self, &flags, engine, dependencies, daemons, human);
            }
            None => {}
        }
        let json = self.json;
        let offered = offer(self.operation);
        let mut params = Value::Object(std::mem::take(&mut self.params));
        // Without `--follow` or `--through`, a sequence of what a crawl
        // committed reads through the last Record committed when it began,
        // which its first call reads; it prints each Record as it arrives.
        let bounded =
            offered == Some(Offer::Committed) && !self.follow && params.get(THROUGH).is_none();
        let array = RefCell::new(JsonArray::default());
        let mut print_sent = |method: &str, sent: &Value| Some(follow(json, method, sent));
        let mut print_committed = |method: &str, sent: &Value| {
            let items = items(method, sent);
            for item in &items {
                if json {
                    array.borrow_mut().item(item);
                } else {
                    output::line(&fields(item));
                }
            }
            Some(u32::try_from(items.len()).unwrap_or(u32::MAX))
        };
        let mut print_answers = |method: &str, sent: &Value| {
            follow(json, method, sent);
            Some(0)
        };
        let follower: Option<Follower<'_>> = match (offered, self.follow) {
            (Some(Offer::Follow | Offer::Committed), true) => Some(&mut print_sent),
            (Some(Offer::Committed), false) => Some(&mut print_committed),
            (Some(Offer::Input), _) => Some(&mut print_answers),
            _ => None,
        };
        let field = page_items(self.operation);
        let mut asked = params.clone();
        let mut next_page = |page: &Value| {
            for item in page[&field].as_array().into_iter().flatten() {
                print_item(json, item);
            }
            if page[ENDED] == true {
                return None;
            }
            asked[AFTER] = page[NEXT].clone();
            Some(Next {
                method: self.operation.name,
                params: asked.clone(),
                cancellable: self.operation.cancellable,
                followed: false,
            })
        };
        // The bound's read is the first call; the sequence, through its
        // `through`, the next.
        let bounding = Cell::new(bounded);
        let mut sequence = params.clone();
        let mut read_through = |page: &Value| {
            if !bounding.replace(false) {
                return None;
            }
            sequence[THROUGH] = page[THROUGH].clone();
            Some(Next {
                method: self.operation.name,
                params: sequence.clone(),
                cancellable: self.operation.cancellable,
                followed: true,
            })
        };
        let (method, cancellable) = if bounded {
            params = json!({
                "run_id": params["run_id"],
                "dataset": params["dataset"],
                "limit": 1,
                "columns": [],
            });
            (BOUND.name, BOUND.cancellable)
        } else {
            (self.operation.name, self.operation.cancellable)
        };
        let pager: Option<Pager<'_>> = if bounded {
            Some(&mut read_through)
        } else {
            (offered == Some(Offer::Pages) && self.follow).then_some(&mut next_page)
        };
        let refused = Arc::new(OnceLock::new());
        let mut stdin = ReadValues::new({
            let refused = Arc::clone(&refused);
            move |sender| read_stdin(&sender, &refused)
        });
        let input = INPUTS
            .iter()
            .find(|(name, _, _)| *name == self.operation.name)
            .map(|(_, method, field)| Input {
                method,
                field,
                values: Values::Read(&mut stdin),
            });
        let call = CommandCall {
            method,
            params,
            cancellable,
            follower,
            pager,
            input,
            stopper: None,
        };
        // A followed call prints as it runs, so only a call that prints at
        // its end shows its words as progress until then.
        let prints_at_its_end =
            !self.follow && !matches!(offered, Some(Offer::Input | Offer::Committed));
        let progress = prints_at_its_end.then(|| human.progress(&words));
        let warn = |text: &str| human.warning(text);
        let reach = self.reach(daemons, &warn);
        let answer = engine_choice::call(engine, dependencies, self.product, &reach, call);
        drop(progress);
        // Once the array of Records is open on stdout, it closes whatever
        // ends the call, and a failure goes to stderr alone (ADR 0069).
        let opened = array.borrow_mut().close();
        if let Some(problem) = refused.get() {
            human.failure(problem, &[]);
            return ExitCode::from(goodfetch_protocol::cli::USAGE.code);
        }
        let failing = if bounding.get() {
            BOUND.name
        } else {
            self.operation.name
        };
        self.answered(answer, opened, failing, human)
    }

    /// Prints what the call answered, `answer`, and answers the exit code:
    /// a failure of the call `failing` printed on stderr alone once
    /// stdout's array of Records was `opened`.
    fn answered(&self, answer: Answer, opened: bool, failing: &str, human: &Human) -> ExitCode {
        let (json, offered) = (self.json, offer(self.operation));
        match answer {
            Answer::Result(result) => {
                match offered {
                    _ if self.follow => {}
                    // What a crawl committed is what such a command
                    // printed, an empty array under `--json` when nothing.
                    Some(Offer::Committed) if json && !opened => output::raw(b"[]\n"),
                    Some(Offer::Committed) => {}
                    Some(Offer::Input) if json => {}
                    _ => self.print(&result, human),
                }
                ExitCode::from(self.exit_of(result))
            }
            Answer::Error(error) if opened => {
                let exit = exit_codes::of_error(&error.data.code, exit_codes::group(&error));
                output::failed_after(&output::command_failed(error, Some(failing)), json, human);
                ExitCode::from(exit)
            }
            Answer::Error(error) => output::fail(error, Some(failing), json, human),
            Answer::Refused(error) => self.fail_exiting(error, exit_codes::DAEMON_REFUSED, human),
            Answer::Unanswered(error) => output::fail(error, None, json, human),
            Answer::Interrupted(interrupt) => ExitCode::from(interrupt.exit_code()),
        }
    }

    /// The exit code of a call that answered `result`: by how the Run ended
    /// when it is a Run report, else success.
    fn exit_of(&self, result: Value) -> u8 {
        let Some(require_verified) = self.report else {
            return exit_codes::SUCCESS;
        };
        match serde_json::from_value::<RunReport>(result) {
            Ok(report) => exit_codes::of_report(&report, require_verified),
            Err(problem) => {
                tracing::error!(%problem, "the engine answered no Run report");
                exit_codes::unanswered()
            }
        }
    }

    /// Prints `result`: as JSON under `--json`, which keeps its times in
    /// UTC; else an opener's new ID, or each field, a point in time in the
    /// local zone.
    fn print(&self, result: &Value, human: &Human) {
        if self.json {
            output::json(result);
            return;
        }
        if self.operation.kind == OperationKind::Opener {
            let id = format!("{}_id", namespace(self.operation));
            if let Some(id) = result[&id].as_str() {
                output::line(id);
                return;
            }
        }
        match result {
            Value::Object(fields) => {
                let times = times(self.operation);
                for (name, value) in fields {
                    let shown = match value {
                        Value::String(text) if times.contains(name) => human.time(text),
                        _ => None,
                    };
                    human.field(name, &shown.unwrap_or_else(|| self::text(value)));
                }
            }
            value => output::line(&text(value)),
        }
    }

    /// Prints the failure `error` and answers its exit code, its Error
    /// code's row of the table.
    fn fail(&self, error: ErrorObject, human: &Human) -> ExitCode {
        output::fail(error, Some(self.operation.name), self.json, human)
    }

    /// Prints the failure `error` and answers `exit`.
    fn fail_exiting(&self, error: ErrorObject, exit: u8, human: &Human) -> ExitCode {
        output::fail_exiting(error, Some(self.operation.name), exit, self.json, human)
    }
}

/// Prints what a followed call sent in the notification `method`, one item
/// per line, as JSON under `--json`, and answers how many items, each the
/// credit it took: each event of an event batch, or the notification's
/// params as one item, such as a chunk of a body.
fn follow(json: bool, method: &str, sent: &Value) -> u32 {
    let items = items(method, sent);
    for item in &items {
        print_item(json, item);
    }
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

/// The items of what a followed call sent in the notification `method`:
/// each event of an event batch, or the notification's params as one item.
fn items<'a>(method: &str, sent: &'a Value) -> Vec<&'a Value> {
    match &sent["events"] {
        Value::Array(events) if method == EVENTS => events.iter().collect(),
        _ => vec![sent],
    }
}

/// Prints one item a call sent or a page held, on one line: as JSON under
/// `--json`, else its `name=value` fields.
fn print_item(json: bool, item: &Value) {
    if json {
        output::json(item);
    } else {
        output::line(&fields(item));
    }
}

/// The JSON array a sequence of what a crawl committed prints under
/// `--json`, written as its items arrive, so the command keeps none: `[`
/// before the first, each item on a line of its own, `]` once its call
/// ended, however it ended.
#[derive(Default)]
struct JsonArray {
    /// Whether `[` is out.
    opened: bool,
}

impl JsonArray {
    /// Writes `item`, opening the array first.
    fn item(&mut self, item: &Value) {
        let mut bytes = if self.opened {
            b",\n".to_vec()
        } else {
            b"[\n".to_vec()
        };
        self.opened = true;
        bytes.extend(serde_json::to_vec(item).expect("invariant: a JSON value serializes"));
        output::raw(&bytes);
    }

    /// Ends the array once it is open, and answers whether it was.
    fn close(&mut self) -> bool {
        if self.opened {
            output::raw(b"\n]\n");
        }
        self.opened
    }
}

/// The read whose `through` bounds a sequence of what a crawl committed
/// without `--follow`: the last Record committed when the command began,
/// which `dataset.page` answers.
const BOUND: Operation = goodfetch_protocol::foxhound::dataset_view::DATASET_PAGE;

/// The param of such a sequence, and the field of [`BOUND`]'s answer, that
/// names the last Record it reads.
const THROUGH: &str = "through";

/// The param of a call that answers page by page that names where its
/// page starts.
const AFTER: &str = "after";

/// The field of such a page that the next page's `after` takes.
const NEXT: &str = "next";

/// The field of such a page that says it is the last.
const ENDED: &str = "ended";

/// Whether `operation` answers page by page: it takes [`AFTER`], and its
/// result says [`NEXT`] and [`ENDED`] beside one list of items.
fn answers_pages(operation: &Operation) -> bool {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let params = Value::from((operation.params)(&mut generator));
    let definitions = Value::Object(generator.take_definitions(true));
    let params = resolved(&params, &definitions);
    params["properties"].get(AFTER).is_some()
        && result_fields(operation).contains(&NEXT.to_owned())
        && result_fields(operation).contains(&ENDED.to_owned())
        && !page_items(operation).is_empty()
}

/// The names of `operation`'s result's fields.
fn result_fields(operation: &Operation) -> Vec<String> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let result = Value::from((operation.result)(&mut generator));
    let definitions = Value::Object(generator.take_definitions(true));
    resolved(&result, &definitions)["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .collect()
}

/// The one field of `operation`'s result that lists items, or nothing
/// when it has none or several.
fn page_items(operation: &Operation) -> String {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let result = Value::from((operation.result)(&mut generator));
    let definitions = Value::Object(generator.take_definitions(true));
    let lists: Vec<&String> = resolved(&result, &definitions)["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, schema)| type_name(resolved(schema, &definitions)) == Some("array"))
        .map(|(name, _)| name)
        .collect();
    match lists.as_slice() {
        [list] => (*list).clone(),
        _ => String::new(),
    }
}

/// Reads the values a command takes from stdin for a call that takes a
/// stream, on the blocking thread the call starts it on, sending each to
/// `sender` as soon as its line is read: one per line, a line read as JSON
/// when it parses as JSON, as text otherwise, a blank line passed over. It
/// stops at stdin's end, or at a line that is not UTF-8 or does not read,
/// which it names, with its number, in `refused`: the stream ends there,
/// and the command then fails as a usage error.
fn read_stdin(sender: &tokio::sync::mpsc::Sender<Value>, refused: &OnceLock<String>) {
    #[expect(
        clippy::disallowed_methods,
        reason = "a command reads its stream's values from stdin on a tracked blocking thread, never an async worker (STYLE.md)"
    )]
    let stdin = std::io::stdin();
    read_values(&mut stdin.lock(), sender, refused);
}

/// Reads `input`'s values as [`read_stdin`] reads stdin's.
fn read_values(
    input: &mut impl std::io::BufRead,
    sender: &tokio::sync::mpsc::Sender<Value>,
    refused: &OnceLock<String>,
) {
    let mut line = Vec::new();
    for number in 1_u64.. {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => {
                let _ = refused.set(format!(
                    "line {number} of standard input does not read: {error}"
                ));
                return;
            }
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            let _ = refused.set(format!(
                "line {number} of standard input is not UTF-8: give one value per line in UTF-8"
            ));
            return;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let value = serde_json::from_str(text).unwrap_or_else(|_| Value::from(text));
        if sender.blocking_send(value).is_err() {
            return;
        }
    }
}

/// An item a followed call sent, as human output shows it on one line:
/// `name=value` for each field but the call's own `id`.
fn fields(item: &Value) -> String {
    match item {
        Value::Object(fields) => fields
            .iter()
            .filter(|(name, _)| name.as_str() != "id")
            .map(|(name, value)| format!("{name}={}", text(value)))
            .collect::<Vec<_>>()
            .join(" "),
        value => text(value),
    }
}

/// A value as human output shows it: text as it is, anything else as JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}

#[cfg(test)]
mod tests;
