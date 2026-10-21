//! The commands generated from Collie's catalog (ADR 0099): every round trip
//! and opener of Collie whose namespace no hand-written command group
//! takes is `<namespace> <member>`, words joined by `-`, whose flags are its
//! params, `--<parameter>` in kebab-case, a value-bound command taking its
//! value's ID (`run_id` for `run`) as its first positional argument, and
//! `--json` printing the result as JSON. Help comes from the catalog's
//! descriptions. Foxhound's executable carries these commands unchanged
//! (ADR 0090).
//!
//! A command calls its operation on the engine [`engine_choice`] gives it
//! and prints the result: an opener prints the new value's ID, any other
//! the result's fields one per line, a point in time in the local zone,
//! and `--json` the result itself, in UTC. While the call runs, its words
//! show as progress on a terminal. A
//! failure prints its message, code, retryability and `fix` on stderr, or
//! under `--json` [`CommandFailed`] on stdout, and exits with its row of
//! [`exit_codes`]. A deprecated command warns on stderr.

use std::process::ExitCode;

use clap::{Arg, ArgAction, ArgMatches, Command};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::{ErrorGroup, Operation, OperationKind, Product};
use goodfetch_protocol::cli::{CommandError, CommandFailed};
use goodfetch_protocol::entries::operations_of;
use goodfetch_protocol::jsonrpc::{ErrorObject, ProtocolFailure};
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

use crate::engine_choice::{self, Answer};
use crate::output::Human;
use crate::{exit_codes, output};

/// The Product whose catalog the commands come from: Collie's, which both
/// executables carry (ADR 0090).
const PRODUCT: Product = Product::Collie;

/// The flag that prints the result as JSON.
const JSON: &str = "json";

/// The generated commands beside an executable's hand-written ones.
pub struct Generated {
    /// The namespaces they cover, as the catalog writes them.
    namespaces: Vec<&'static str>,
}

impl Generated {
    /// The commands of every namespace no command group of `hand_written`
    /// takes.
    pub fn beside(hand_written: &Command) -> Self {
        let mut namespaces: Vec<&'static str> = commands()
            .map(namespace)
            .filter(|namespace| hand_written.find_subcommand(spelled(namespace)).is_none())
            .collect();
        namespaces.dedup();
        Self { namespaces }
    }

    /// `command` with a command group per namespace, whose commands are
    /// built only when clap reaches the group.
    #[must_use]
    pub fn add_to(&self, command: Command) -> Command {
        command.subcommands(self.namespaces.iter().map(|namespace| {
            Command::new(spelled(namespace))
                .about(format!("Calls the catalog's `{namespace}` operations"))
                .subcommand_required(true)
                .defer(members)
        }))
    }

    /// The call a parse of the command line asks for, when its command is
    /// generated.
    pub fn call(&self, matches: &ArgMatches) -> Option<Call> {
        let (group, group_matches) = matches.subcommand()?;
        let namespace = self
            .namespaces
            .iter()
            .find(|namespace| spelled(namespace) == group)?;
        let (member, member_matches) = group_matches.subcommand()?;
        let name = format!("{namespace}.{}", member.replace('-', "_"));
        let operation = commands().find(|operation| operation.name == name)?;
        let params = parameters(operation)
            .into_iter()
            .filter_map(|parameter| {
                let value = parameter.value(member_matches)?;
                Some((parameter.name, value))
            })
            .collect();
        Some(Call {
            operation,
            params,
            json: member_matches.get_flag(JSON),
        })
    }
}

/// The operations that are commands, in the catalog's order.
fn commands() -> impl Iterator<Item = &'static Operation> {
    operations_of(PRODUCT).filter(|operation| {
        matches!(
            operation.kind,
            OperationKind::RoundTrip | OperationKind::Opener
        )
    })
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

/// The commands of the group `group`, one per operation of its namespace.
fn members(group: Command) -> Command {
    let namespace = group.get_name().replace('-', "_");
    group.subcommands(
        commands()
            .filter(|operation| self::namespace(operation) == namespace)
            .map(command),
    )
}

/// The command of `operation`, its help in the form of the hand-written
/// commands': the description with no closing period.
fn command(operation: &'static Operation) -> Command {
    let mut command =
        Command::new(spelled(member(operation))).about(sentence(operation.description));
    if let Some(deprecated) = operation.deprecated {
        command = command.after_help(format!(
            "Deprecated since {}: {}",
            deprecated.since, deprecated.note
        ));
    }
    parameters(operation)
        .into_iter()
        .fold(command, |command, parameter| command.arg(parameter.arg()))
        .arg(
            Arg::new(JSON)
                .long(JSON)
                .action(ArgAction::SetTrue)
                .help("Prints the result as JSON"),
        )
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
    /// A flag with a text.
    Text,
    /// A flag with a value read as JSON when it parses as JSON, as text
    /// otherwise.
    Json,
    /// A repeatable flag, each value read as `Text` or `Json`.
    Repeated { json: bool },
}

impl Parameter {
    fn arg(&self) -> Arg {
        let arg = Arg::new(self.name.clone())
            .help(self.help.clone())
            .value_name(self.name.to_uppercase());
        match self.form {
            Form::Value => arg.required(true),
            Form::Switch => arg.long(spelled(&self.name)).action(ArgAction::SetTrue),
            Form::Text | Form::Json => arg
                .long(spelled(&self.name))
                .required(self.required)
                .action(ArgAction::Set),
            Form::Repeated { .. } => arg
                .long(spelled(&self.name))
                .required(self.required)
                .action(ArgAction::Append),
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
            Form::Value | Form::Text => matches
                .get_one::<String>(&self.name)
                .map(|text| read(text, false)),
            Form::Json => matches
                .get_one::<String>(&self.name)
                .map(|text| read(text, true)),
            Form::Repeated { json } => matches
                .get_many::<String>(&self.name)
                .map(|values| Value::Array(values.map(|text| read(text, json)).collect())),
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
            } else {
                match type_name(type_of) {
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
    operation: &'static Operation,
    params: Map<String, Value>,
    json: bool,
}

impl Call {
    /// Calls the operation on an Attached engine in this process, `engine`
    /// on `dependencies`, prints what it answered as `human` shows it, and
    /// answers the exit code.
    pub fn run(mut self, engine: &Engine, dependencies: Dependencies, human: &Human) -> ExitCode {
        let words = self.operation.name.replace('.', " ").replace('_', "-");
        if let Some(deprecated) = self.operation.deprecated {
            human.warning(&format!(
                "`{words}` is deprecated since {}: {}",
                deprecated.since, deprecated.note
            ));
        }
        let progress = human.progress(&words);
        let answer = engine_choice::call_attached(
            engine,
            dependencies,
            PRODUCT,
            self.operation.name,
            Value::Object(std::mem::take(&mut self.params)),
        );
        drop(progress);
        match answer {
            Answer::Result(result) => {
                self.print(&result, human);
                ExitCode::from(exit_codes::SUCCESS)
            }
            Answer::Error(error) => self.fail(error, human),
            Answer::Unanswered(problem) => {
                tracing::error!("{problem}");
                ExitCode::from(exit_codes::unanswered())
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

    fn fail(&self, error: ErrorObject, human: &Human) -> ExitCode {
        let group = group(&error);
        let exit = exit_codes::of_error(&error.data.code, group);
        let failed = CommandFailed {
            error: CommandError {
                code: error.data.code,
                group,
                retryable: error.data.retryable,
                message: error.message,
                operation: self.operation.name.to_owned(),
                details: error.data.details,
            },
        };
        if self.json {
            output::json(&failed);
        } else {
            let error = &failed.error;
            let mut details = vec![
                ("code", error.code.clone()),
                ("retryable", error.retryable.to_string()),
            ];
            if let Some(fix) = error.details.get("fix").and_then(Value::as_str) {
                details.push(("fix", fix.to_owned()));
            }
            human.failure(&error.message, &details);
        }
        ExitCode::from(exit)
    }
}

/// A value as human output shows it: text as it is, anything else as JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}

/// The group of `error`: its numeric class's, or for a failure of the
/// protocol itself, the one ADR 0079 gives it.
fn group(error: &ErrorObject) -> ErrorGroup {
    ErrorGroup::ALL
        .into_iter()
        .find(|group| group.json_rpc_class() == error.code)
        .or_else(|| {
            [
                ProtocolFailure::ParseError,
                ProtocolFailure::InvalidRequest,
                ProtocolFailure::MessageTooLarge,
                ProtocolFailure::MessageTooDeep,
                ProtocolFailure::MethodNotFound,
                ProtocolFailure::InvalidParams,
                ProtocolFailure::InternalError,
            ]
            .into_iter()
            .find(|failure| failure.name() == error.data.code)
            .map(ProtocolFailure::group)
        })
        .unwrap_or(ErrorGroup::Internal)
}
