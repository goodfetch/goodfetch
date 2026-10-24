//! Page functions (ADR 0085): JavaScript source run in a page with one
//! explicit JSON argument, its result crossing back as JSON. `evaluate`'s
//! expression calls the walker (`walk.js`) with the program's source in a
//! thunk of its own, so the source sees the page's globals and its argument
//! only; the walker answers the result's JSON, the first value outside JSON
//! by its path and kind (`page_value_invalid`), or what the page threw
//! (`page_function_failed`). An init script is the source called with its
//! argument when it is a function, or the script as it is. Each Session's
//! first main-world call and first init script are marked in its
//! Provenance (ADRs 0019 and 0021), and a navigation sent after an init
//! script's call waits until the script is registered, or removed once its
//! call was cancelled (`InitScripts`). `page.wait_for_function` runs the
//! same expression again at each interval until its result is truthy
//! ([`until_truthy`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::page::ScriptIdentifier;
use goodfetch_cdp::generated::runtime;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::page::functions::{
    Evaluated, PAGE_FUNCTION_FAILED, PAGE_VALUE_INVALID, PageFunctionFailedDetails,
    PageValueDirection, PageValueInvalidDetails, PageValueKind, World,
};
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_FUNCTION;
use goodfetch_protocol::page::{PageId, SessionId};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::driver::PageDriver;
use crate::trace;

/// The walker, a function expression the engine calls with the thunk of
/// the program's source, its argument and whether one was given.
const WALK: &str = include_str!("page_functions/walk.js");

/// The Provenance action that marks a main-world injection.
const MAIN_WORLD_INJECTION: &str = "main_world_injection";

/// The expression `Runtime.evaluate` runs for the page function `function`
/// with `arg`: the walker called with a thunk of the source, kept outside
/// the walker's scope, and the argument parsed from its JSON text, so that
/// a key such as `__proto__` stays a key.
pub(crate) fn expression(function: &str, arg: Option<&Value>) -> String {
    let text = arg.map_or_else(|| "null".to_owned(), Value::to_string);
    let literal = Value::String(text).to_string();
    let given = arg.is_some();
    format!("({WALK})(() => (\n{function}\n), JSON.parse({literal}), {given})")
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
/// answers that result; its errors are spelled for `surface`.
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
    expression: &str,
    world: World,
    (interval, clock): (Duration, &Clock),
    surface: Surface,
) -> Result<Evaluated, CallError> {
    loop {
        let returns = driver
            .evaluate(expression, world, PAGE_WAIT_FOR_FUNCTION.name)
            .await?;
        let evaluated = answered(returns, surface)?;
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
/// built it, its errors spelled for `surface`.
pub(crate) fn answered(
    returns: runtime::EvaluateReturns,
    surface: Surface,
) -> Result<Evaluated, CallError> {
    if let Some(details) = returns.exception_details {
        return Err(failed(not_run(&details), surface));
    }
    let walked: Walked = serde_json::from_value(returns.result.value.unwrap_or(Value::Null))
        .map_err(|error| {
            failed(
                Thrown {
                    name: None,
                    message: format!("the page's answer was not the walker's: {error}"),
                    stack: None,
                },
                surface,
            )
        })?;
    if let Some(invalid) = walked.invalid {
        return Err(value_invalid(invalid));
    }
    if let Some(thrown) = walked.thrown {
        return Err(failed(thrown, surface));
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
    let message = description
        .as_deref()
        .and_then(|description| description.lines().next())
        .map_or_else(
            || details.text.clone(),
            |line| {
                name.as_deref()
                    .and_then(|name| line.strip_prefix(name))
                    .and_then(|rest| rest.strip_prefix(": "))
                    .unwrap_or(line)
                    .to_owned()
            },
        );
    Thrown {
        name,
        message,
        stack: description,
    }
}

/// `page_function_failed` for what the page threw; a `ReferenceError`'s
/// fix names the argument, spelled for `surface`.
fn failed(thrown: Thrown, surface: Surface) -> CallError {
    let fix = (thrown.name.as_deref() == Some("ReferenceError"))
        .then(|| thrown.message.strip_suffix(" is not defined"))
        .flatten()
        .map(|missing| {
            let call = match surface {
                Surface::SdkTypescript => format!("page.evaluate(({missing}) => …, {missing})"),
                Surface::SdkPython
                | Surface::Rust
                | Surface::Cli
                | Surface::Mcp
                | Surface::Console
                | Surface::Cloud
                | Surface::BrowserHost
                | Surface::Worker => format!("page.evaluate(\"({missing}) => …\", {missing})"),
            };
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
}

impl Injection {
    /// The operation that injected, as Provenance names it.
    fn by(self) -> &'static str {
        match self {
            Self::Evaluate => goodfetch_protocol::page::functions::PAGE_EVALUATE.name,
            Self::InitScript => goodfetch_protocol::page::functions::PAGE_ADD_INIT_SCRIPT.name,
            Self::WaitForFunction => PAGE_WAIT_FOR_FUNCTION.name,
        }
    }
}

/// The Sessions already marked in Provenance, each once per kind of
/// injection.
#[derive(Clone)]
pub(crate) struct Injections {
    marked: Arc<watch::Sender<HashSet<(SessionId, Injection)>>>,
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
    /// Session `session` made a main-world injection by `injection`, the
    /// first time its Session does.
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
            .send_if_modified(|marked| marked.insert((session.clone(), injection)));
        if first {
            trace::provenance(
                run,
                &json!({
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

/// The init scripts each page has on their way in or out: a navigation
/// waits for none to be left, so that one sent after `page.add_init_script`
/// runs the script, and one sent after its call was cancelled runs without
/// it.
#[derive(Clone)]
pub(crate) struct InitScripts {
    pending: Arc<watch::Sender<HashMap<PageId, usize>>>,
}

impl Default for InitScripts {
    fn default() -> Self {
        Self {
            pending: Arc::new(watch::Sender::new(HashMap::new())),
        }
    }
}

impl InitScripts {
    /// Counts one script of the page `page_id` on its way, until the
    /// answer is dropped.
    pub(crate) fn begin(&self, page_id: &PageId) -> Pending {
        self.pending.send_modify(|pending| {
            *pending.entry(page_id.clone()).or_default() += 1;
        });
        Pending {
            pending: Arc::clone(&self.pending),
            page_id: page_id.clone(),
        }
    }

    /// Waits until no script of the page `page_id` is on its way.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn settled(&self, page_id: &PageId) {
        let mut pending = self.pending.subscribe();
        let _ = pending
            .wait_for(|pending| !pending.contains_key(page_id))
            .await;
    }
}

/// One init script on its way in or out of its page.
pub(crate) struct Pending {
    pending: Arc<watch::Sender<HashMap<PageId, usize>>>,
    page_id: PageId,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.pending.send_modify(|pending| {
            if let Some(count) = pending.get_mut(&self.page_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    pending.remove(&self.page_id);
                }
            }
        });
    }
}

/// An init script registered on its page, removed once dropped unless its
/// page ended: the removal is counted on its way out, so a navigation sent
/// after the drop runs without the script.
pub(crate) struct Registered {
    driver: PageDriver,
    identifier: Option<ScriptIdentifier>,
    run: Run,
    scripts: InitScripts,
    page_id: PageId,
}

impl Registered {
    /// The script `identifier`, registered on the page `page_id` that
    /// `driver` drives for `run`, counted among `scripts` on its way out.
    pub(crate) fn new(
        driver: PageDriver,
        identifier: ScriptIdentifier,
        run: Run,
        scripts: InitScripts,
        page_id: PageId,
    ) -> Self {
        Self {
            driver,
            identifier: Some(identifier),
            run,
            scripts,
            page_id,
        }
    }

    /// Keeps the script, whose page ended with it: nothing is left to
    /// remove.
    pub(crate) fn ended(mut self) {
        self.identifier = None;
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        let Some(identifier) = self.identifier.take() else {
            return;
        };
        let leaving = self.scripts.begin(&self.page_id);
        let driver = self.driver.clone();
        self.run.spawn(async move {
            driver.remove_init_script(identifier).await;
            drop(leaving);
        });
    }
}

#[cfg(test)]
mod tests;
