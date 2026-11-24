//! Page functions (ADR 0085): JavaScript source run in a page with one
//! explicit JSON argument, its result crossing back as JSON. `evaluate`'s
//! expression calls the walker (`walk.js`) with the program's source in a
//! thunk of its own, so the source sees the page's globals and its argument
//! only; the walker answers the result's JSON, the first value outside JSON
//! by its path and kind (`page_value_invalid`), or what the page threw
//! (`page_function_failed`). An init script is the source called with its
//! argument when it is a function, or the script as it is, which the page's
//! owner adds (`init_scripts`). Each Session's first main-world call and
//! first init script are marked in its Provenance (ADRs 0019 and 0021).
//! `page.wait_for_function` runs the same expression again at each interval
//! until its result is truthy ([`until_truthy`]). A locator's page function,
//! `locator.evaluate_all`'s and `locator.wait_for_function`'s, takes the
//! locator's matches first, found in its world by `targets.js`
//! ([`on_locator`]): every match, or the one match, a check while there are
//! none or several answering no value, which is falsy.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::runtime;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::functions::{
    Evaluated, PAGE_FUNCTION_FAILED, PAGE_VALUE_INVALID, PageFunctionFailedDetails,
    PageValueDirection, PageValueInvalidDetails, PageValueKind, World,
};
use goodfetch_protocol::page::functions::{LOCATOR_EVALUATE_ALL, PAGE_EVALUATE};
use goodfetch_protocol::page::waits::{LOCATOR_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_FUNCTION};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::{connection::Form, naming};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::driver::PageDriver;
use crate::trace;

/// The walker, a function expression the engine calls with the thunk of
/// the program's source, its argument and whether one was given.
const WALK: &str = include_str!("page_functions/walk.js");

/// The finder of a locator's matches, a function the engine calls with the
/// selector and whether the page function takes one match, which answers
/// the thunk the walker calls.
const TARGETS: &str = include_str!("page_functions/targets.js");

/// The Provenance action that marks a main-world injection.
const MAIN_WORLD_INJECTION: &str = "main_world_injection";

/// The expression `Runtime.evaluate` runs for the page function `function`
/// with `arg`: the walker called with a thunk of the source, kept outside
/// the walker's scope, and the argument parsed from its JSON text, so that
/// a key such as `__proto__` stays a key.
pub(crate) fn expression(function: &str, arg: Option<&Value>) -> String {
    called(function, arg, "undefined")
}

/// Which matches of its locator a page function takes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Matches {
    /// The array of every match, none included (`locator.evaluate_all`).
    All,
    /// The one match (`locator.wait_for_function`); none or several make
    /// the check answer no value.
    One,
}

/// The expression `Runtime.evaluate` runs for the page function `function`
/// of a locator whose CSS selector is `selector`, with `arg`: as
/// [`expression`]'s, the locator's `matches` found in the same world and
/// handed to the function before its argument.
pub(crate) fn on_locator(
    function: &str,
    arg: Option<&Value>,
    (selector, matches): (&str, Matches),
) -> String {
    let selector = Value::String(selector.to_owned()).to_string();
    let one = matches == Matches::One;
    called(function, arg, &format!("({TARGETS})({selector}, {one})"))
}

/// The walker's call for `function` with `arg`, `targets` the expression
/// of the thunk that finds a locator's matches, or `undefined`.
fn called(function: &str, arg: Option<&Value>, targets: &str) -> String {
    let text = arg.map_or_else(|| "null".to_owned(), Value::to_string);
    let literal = Value::String(text).to_string();
    let given = arg.is_some();
    format!("({WALK})(() => (\n{function}\n), JSON.parse({literal}), {given}, {targets})")
}

/// What the walker answers.
#[derive(Deserialize)]
struct Walked {
    /// The result's JSON, `null` included; absent for `undefined`.
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
    #[serde(default)]
    invalid: Option<Invalid>,
    #[serde(default)]
    thrown: Option<Thrown>,
    #[serde(default)]
    expression: bool,
    /// A locator's selector the page does not parse.
    #[serde(default)]
    unparsed: Option<Unparsed>,
    /// How many elements a locator matched where its page function takes
    /// one, when they were not one.
    #[serde(default)]
    missing: Option<u64>,
}

/// A locator's selector the page does not parse, and what it said.
#[derive(Deserialize)]
struct Unparsed {
    selector: String,
    message: String,
}

/// A value that is there, `null` included, which `Option<Value>` alone
/// would read as absent.
fn present<'de, D: Deserializer<'de>>(value: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(value).map(Some)
}

/// The first value outside JSON the walker met.
#[derive(Deserialize)]
struct Invalid {
    path: String,
    kind: PageValueKind,
    constructor: Option<String>,
}

/// What the page threw.
#[derive(Deserialize)]
struct Thrown {
    name: Option<String>,
    message: String,
    stack: Option<String>,
}

/// Runs `expression`, as [`expression`] built it, in `world` of the page
/// `driver` drives, again every `interval` on `clock`, and in each new
/// document, until its result is truthy as JavaScript judges it, and
/// answers that result; its errors are spelled in `form`.
///
/// # Errors
///
/// What [`answered`] raises for a result outside JSON or a function that
/// threw, and the browser's failures as their codes.
///
/// # Cancel safety
///
/// Dropping the future runs the function no more.
pub(crate) async fn until_truthy(
    driver: &PageDriver,
    (expression, operation): (&str, &'static str),
    world: World,
    (interval, clock): (Duration, &Clock),
    form: Form,
) -> Result<Evaluated, CallError> {
    loop {
        let returns = driver.evaluate(expression, world, operation).await?;
        let evaluated = answered(returns, (form, operation))?;
        if truthy(evaluated.value.as_ref()) {
            return Ok(evaluated);
        }
        clock.sleep(interval).await;
    }
}

/// Whether a page function's result is truthy, as JavaScript judges it:
/// not `undefined`, `null`, `false`, `0` or the empty string, which is all
/// JSON carries of the falsy values.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// The answer to a page function `Runtime.evaluate` ran as [`expression`]
/// built it for the operation `operation`, its errors spelled in
/// `form`.
pub(crate) fn answered(
    returns: runtime::EvaluateReturns,
    spelled: (Form, &'static str),
) -> Result<Evaluated, CallError> {
    if let Some(details) = returns.exception_details {
        return Err(failed(not_run(&details), spelled));
    }
    let walked: Walked = serde_json::from_value(returns.result.value.unwrap_or(Value::Null))
        .map_err(|error| {
            failed(
                Thrown {
                    name: None,
                    message: format!("the page's answer was not the walker's: {error}"),
                    stack: None,
                },
                spelled,
            )
        })?;
    if let Some(Unparsed { selector, message }) = walked.unparsed {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{selector}` is no CSS selector the page parses: {message}"),
        ));
    }
    if walked.missing.is_some() {
        return Ok(Evaluated { value: None });
    }
    if let Some(invalid) = walked.invalid {
        return Err(value_invalid(invalid));
    }
    if let Some(thrown) = walked.thrown {
        return Err(failed(thrown, spelled));
    }
    if walked.expression {
        return Err(argument_to_expression());
    }
    Ok(Evaluated {
        value: walked.value,
    })
}

/// What an exception the evaluation itself raised, such as the source's
/// `SyntaxError`, says.
fn not_run(details: &runtime::ExceptionDetails) -> Thrown {
    let exception = details.exception.as_ref();
    let description = exception.and_then(|exception| exception.description.clone());
    let name = exception.and_then(|exception| exception.class_name.clone());
    let message = description.as_deref().map_or_else(
        || details.text.clone(),
        |description| error_message(name.as_deref(), description),
    );
    Thrown {
        name,
        message,
        stack: description,
    }
}

/// An error's message from its V8 description, `<name>: <message>` then
/// its stack: the first line, past its name `name` when it starts so, or
/// none when it is the name alone.
pub(crate) fn error_message(name: Option<&str>, description: &str) -> String {
    let first = description.lines().next().unwrap_or_default();
    name.and_then(|name| first.strip_prefix(name))
        .and_then(|rest| {
            rest.strip_prefix(": ")
                .or_else(|| rest.is_empty().then_some(""))
        })
        .unwrap_or(first)
        .to_owned()
}

/// The call of the operation `operation` that passes `missing` as its
/// argument, spelled in `form`: a locator's page function takes its
/// matches first and the argument second.
fn passing(missing: &str, (form, operation): (Form, &'static str)) -> String {
    let (operation, matches) = match operation {
        name if name == LOCATOR_EVALUATE_ALL.name => (name, Some("elements")),
        name if name == LOCATOR_WAIT_FOR_FUNCTION.name => (name, Some("element")),
        name if name == PAGE_WAIT_FOR_FUNCTION.name => (name, None),
        _ => (PAGE_EVALUATE.name, None),
    };
    let parameters = matches.map_or_else(
        || missing.to_owned(),
        |matches| format!("{matches}, {missing}"),
    );
    let call = naming::operation(form, operation);
    match form {
        Form::Typescript => format!("{call}(({parameters}) => …, {missing})"),
        Form::Python | Form::Rust | Form::Cli | Form::Tool | Form::Catalog => {
            format!("{call}(\"({parameters}) => …\", {missing})")
        }
    }
}

/// `page_function_failed` for what the page threw in the operation
/// `operation`; a `ReferenceError`'s fix names the argument, its call
/// spelled in `form`.
fn failed(thrown: Thrown, (form, operation): (Form, &'static str)) -> CallError {
    let fix = (thrown.name.as_deref() == Some("ReferenceError"))
        .then(|| thrown.message.strip_suffix(" is not defined"))
        .flatten()
        .map(|missing| {
            let call = passing(missing, (form, operation));
            format!(
                "A page function sees only the page and its one argument, never the program's variables: pass `{missing}` as the argument, as `{call}` does."
            )
        });
    let name = thrown.name.as_deref().unwrap_or("a value");
    CallError::new(
        &PAGE_FUNCTION_FAILED,
        format!(
            "the page function threw {name} in the page: {}",
            thrown.message
        ),
        &PageFunctionFailedDetails {
            name: thrown.name,
            message: thrown.message,
            stack: thrown.stack,
            fix,
        },
    )
}

/// `page_function_failed` for an expression given an argument, which only a
/// function takes.
fn argument_to_expression() -> CallError {
    let message = "the page function is an expression, which takes no argument: only a function, such as `(arg) => …`, is called with one";
    CallError::new(
        &PAGE_FUNCTION_FAILED,
        message,
        &PageFunctionFailedDetails {
            name: None,
            message: message.to_owned(),
            stack: None,
            fix: Some(
                "Pass a function that takes the argument, or leave the argument out.".to_owned(),
            ),
        },
    )
}

/// `page_value_invalid` for the result's first value outside JSON.
fn value_invalid(invalid: Invalid) -> CallError {
    let fix = match invalid.kind {
        PageValueKind::Node | PageValueKind::Window => {
            "Return what the program needs of it as JSON, such as an element's `textContent` or an attribute, instead of the object itself."
        }
        PageValueKind::NotANumber | PageValueKind::Infinity => {
            "Return a finite number, or `null` or a string where the value is no number."
        }
        PageValueKind::Bigint => "Return it as a string, or as a number when it fits.",
        PageValueKind::Symbol => "Return its `description`, a string.",
        PageValueKind::Function => "Call it in the page and return its result.",
        PageValueKind::Cycle => "Return a copy without the reference back to the object.",
        PageValueKind::Map => "Return `Object.fromEntries(map)` or `[...map]`.",
        PageValueKind::Set => "Return `[...set]`.",
        PageValueKind::Date => "Return `date.toISOString()` or `date.getTime()`.",
        PageValueKind::Regexp => "Return its `source`, a string.",
        PageValueKind::Error => "Return its `message`, a string, or throw it.",
        PageValueKind::Object => "Return a plain object or an array of the fields it holds.",
    };
    let kind = serde_json::to_value(invalid.kind)
        .ok()
        .and_then(|kind| kind.as_str().map(str::to_owned))
        .unwrap_or_default();
    CallError::new(
        &PAGE_VALUE_INVALID,
        format!(
            "the page function's result holds a value outside JSON at `{}`: {kind}",
            invalid.path
        ),
        &PageValueInvalidDetails {
            direction: PageValueDirection::Result,
            path: invalid.path,
            kind: invalid.kind,
            constructor: invalid.constructor,
            fix: fix.to_owned(),
        },
    )
}

/// The init script's source for `function` with `arg`: a function called
/// with its argument, or the script as it is; a script given an argument
/// is refused before anything is sent.
pub(crate) fn init_script(function: &str, arg: Option<&Value>) -> Result<String, CallError> {
    if let Some(function) = only_function(function) {
        let text = arg.map_or_else(|| "null".to_owned(), Value::to_string);
        let literal = Value::String(text).to_string();
        return Ok(format!(
            "(() => {{ const goodfetchInit = (\n{function}\n); if (typeof goodfetchInit === \"function\") {{ goodfetchInit(JSON.parse({literal})); }} }})();"
        ));
    }
    if arg.is_some() {
        return Err(argument_to_expression());
    }
    Ok(function.to_owned())
}

/// The function `source` is, when it is one function and nothing else:
/// `function …` or an arrow function, either maybe `async`, comments around
/// it and a closing `;` allowed, which the function answered leaves out. A
/// statement before or after it makes the source a script, such as a
/// declaration followed by its call.
fn only_function(source: &str) -> Option<&str> {
    let source = after_trivia(source);
    let after_async = source
        .strip_prefix("async")
        .filter(|rest| rest.starts_with(|c: char| c.is_whitespace() || c == '(' || c == '/'))
        .map_or(source, after_trivia);
    let rest = match after_async.strip_prefix("function") {
        Some(rest) => function_end(rest)?,
        None => arrow_end(after_async)?,
    };
    let after = after_trivia(rest);
    after_trivia(after.strip_prefix(';').unwrap_or(after))
        .is_empty()
        .then(|| &source[..source.len() - rest.len()])
}

/// What follows the function whose `function` keyword `rest` follows, when
/// it reads as one: an optional `*` and name, its parameters, its body.
fn function_end(rest: &str) -> Option<&str> {
    if !rest.starts_with(|c: char| c.is_whitespace() || c == '(' || c == '*' || c == '/') {
        return None;
    }
    let rest = &rest[rest.find('(')?..];
    let rest = after_trivia(&rest[closing(rest)?..]);
    if !rest.starts_with('{') {
        return None;
    }
    Some(&rest[closing(rest)?..])
}

/// What follows the arrow function `source` starts with, when it is one:
/// its parameters, `=>`, and its body, a block or an expression running to
/// the first `;` outside brackets, strings and comments.
fn arrow_end(source: &str) -> Option<&str> {
    let rest = if source.starts_with('(') {
        &source[closing(source)?..]
    } else {
        let end = source
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(source.len());
        if end == 0 {
            return None;
        }
        &source[end..]
    };
    let body = after_trivia(after_trivia(rest).strip_prefix("=>")?);
    if body.starts_with('{') {
        return Some(&body[closing(body)?..]);
    }
    let mut depth = 0_usize;
    for (at, c) in code(body) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            ';' if depth == 0 => return Some(&body[at..]),
            _ => {}
        }
    }
    Some(&body[body.trim_end().len()..])
}

/// `source` past its leading white space and comments.
fn after_trivia(source: &str) -> &str {
    let mut rest = source.trim_start();
    loop {
        if let Some(comment) = rest.strip_prefix("//") {
            rest = comment
                .find('\n')
                .map_or("", |end| &comment[end..])
                .trim_start();
        } else if let Some(comment) = rest.strip_prefix("/*") {
            rest = comment
                .find("*/")
                .map_or("", |end| &comment[end + 2..])
                .trim_start();
        } else {
            return rest;
        }
    }
}

/// The byte offset just past the bracket that closes the one `source`
/// starts with, strings, template literals and comments skipped.
fn closing(source: &str) -> Option<usize> {
    let mut depth = 0_usize;
    for (at, c) in code(source) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at + c.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// The characters of `source` outside its strings, template literals and
/// comments, with their byte offsets. A regular expression literal is read
/// as code, so one holding an unbalanced bracket misreads the source.
fn code(source: &str) -> impl Iterator<Item = (usize, char)> + '_ {
    let mut characters = source.char_indices().peekable();
    std::iter::from_fn(move || {
        while let Some((at, c)) = characters.next() {
            let next = characters.peek().map(|(_, next)| *next);
            match (c, next) {
                ('\'' | '"' | '`', _) => {
                    let mut escaped = false;
                    for (_, inner) in characters.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if inner == '\\' {
                            escaped = true;
                        } else if inner == c {
                            break;
                        }
                    }
                }
                ('/', Some('/')) => {
                    let _ = characters.by_ref().find(|(_, inner)| *inner == '\n');
                }
                ('/', Some('*')) => {
                    characters.next();
                    let mut star = false;
                    for (_, inner) in characters.by_ref() {
                        if star && inner == '/' {
                            break;
                        }
                        star = inner == '*';
                    }
                }
                _ => return Some((at, c)),
            }
        }
        None
    })
}

/// What marked a Session as departing from the no-injection identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Injection {
    /// A main-world `page.evaluate`.
    Evaluate,
    /// An init script.
    InitScript,
    /// A main-world `page.wait_for_function`.
    WaitForFunction,
    /// A main-world `locator.evaluate_all`.
    EvaluateAll,
    /// A main-world `locator.wait_for_function`.
    LocatorWaitForFunction,
}

impl Injection {
    /// The operation that injected, as Provenance names it.
    fn by(self) -> &'static str {
        match self {
            Self::Evaluate => PAGE_EVALUATE.name,
            Self::InitScript => goodfetch_protocol::page::functions::PAGE_ADD_INIT_SCRIPT.name,
            Self::WaitForFunction => PAGE_WAIT_FOR_FUNCTION.name,
            Self::EvaluateAll => LOCATOR_EVALUATE_ALL.name,
            Self::LocatorWaitForFunction => LOCATOR_WAIT_FOR_FUNCTION.name,
        }
    }
}

/// The kinds of injection a Session already marked in Provenance, each
/// once; it lives as long as the Session. Clones share them.
#[derive(Clone)]
pub(crate) struct Injections {
    marked: Arc<watch::Sender<HashSet<Injection>>>,
}

impl Default for Injections {
    fn default() -> Self {
        Self {
            marked: Arc::new(watch::Sender::new(HashSet::new())),
        }
    }
}

impl Injections {
    /// Records in `run`'s Provenance that the page `page_id` of the
    /// Session `session`, whose marks these are, made a main-world
    /// injection by `injection`, the first time its Session does.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the entry.
    pub(crate) async fn mark(
        &self,
        run: &Run,
        session: &SessionId,
        page_id: &PageId,
        injection: Injection,
    ) {
        let first = self
            .marked
            .send_if_modified(|marked| marked.insert(injection));
        if first {
            trace::provenance(
                run,
                json!({
                    "action": MAIN_WORLD_INJECTION,
                    "page_id": page_id,
                    "session_id": session,
                    "by": injection.by(),
                }),
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests;
