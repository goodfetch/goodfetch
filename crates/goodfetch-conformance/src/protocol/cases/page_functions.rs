//! The page suite's page function cases (ADR 0085, ticket #704; pitfall
//! B16, pain P16), on the Proving ground's page functions page, whose own
//! script defines a global and records what it found of an init script's
//! mark. `page.evaluate` answers a function's JSON in the main world, where
//! the page's global is, and in an isolated world, where it is not; a
//! value outside JSON raises `page_value_invalid` naming its path and its
//! kind, and a function reading a variable of the program raises
//! `page_function_failed` whose fix names the argument. An init script
//! registered by `page.add_init_script` runs before the page's own script
//! on the next navigation and in a frame added afterwards, until its call
//! is cancelled, and the Session's Provenance marks the main-world
//! injection, while a page with none records none. `locator.evaluate_all`
//! hands a function every element its locator matches, inside open shadow
//! roots too, with the argument after them, and refuses what
//! `page.evaluate` refuses (#1597).

use goodfetch_protocol::connection::{CALL_CANCELLED, CANCEL};
use goodfetch_protocol::page::functions::{
    LOCATOR_EVALUATE_ALL, PAGE_ADD_INIT_SCRIPT, PAGE_EVALUATE, PAGE_FUNCTION_FAILED,
    PAGE_VALUE_INVALID, PageFunctionFailedDetails, PageValueInvalidDetails,
};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_FUNCTION;
use goodfetch_protocol::page::{PAGE_GOTO, PAGE_RELOAD};
use serde_json::{Value, json};

use super::clean_up::{FRAME_HOST, provenance};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The page functions page's path on the access family.
const PAGE_FUNCTIONS: &str = "/page-functions";

/// The Provenance action that marks a main-world injection.
const MAIN_WORLD_INJECTION: &str = "main_world_injection";

/// The mark the init script sets, which the page's own script reads.
const MARK: &str = "set by the init script";

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Json,
    OutsideJson,
    Thrown,
    InitScript,
    EvaluateAll,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "evaluate-answers-json-in-the-main-world-and-an-isolated-world-sees-no-page-global" => {
                Self::Json
            }
            "a-value-outside-json-raises-page-value-invalid-naming-its-path-and-kind" => {
                Self::OutsideJson
            }
            "a-function-reading-a-program-variable-raises-page-function-failed-whose-fix-names-the-argument" => {
                Self::Thrown
            }
            "an-init-script-runs-before-the-pages-scripts-on-the-next-navigation-and-in-new-frames-and-provenance-marks-it" => {
                Self::InitScript
            }
            "evaluate-all-hands-every-match-and-one-json-argument-and-refuses-what-evaluate-refuses" => {
                Self::EvaluateAll
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Json => answers_json(connection, site, checks),
            Self::OutsideJson => outside_json(connection, site, checks),
            Self::Thrown => thrown(connection, site, checks),
            Self::InitScript => init_script(connection, target, site, checks),
            Self::EvaluateAll => evaluate_all(connection, target, site, checks),
        }
    }
}

/// The page functions page of `site`, opened on a new page of the Run
/// `run_id` once `before` ran with its page's ID: the page's ID.
fn on_the_page(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
    before: impl FnOnce(&mut Connection, &Value) -> Result<Option<i64>, Gone>,
) -> Result<Option<(Value, Option<i64>)>, Gone> {
    checks.check(
        "the Proving ground serves the access family",
        site.access_origin.is_some(),
    );
    let Some(origin) = site.access_origin.clone() else {
        return Ok(None);
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(None);
    };
    let page_id = json!(page.page_id);
    let registered = before(connection, &page_id)?;
    let id = connection.request(
        PAGE_GOTO.name,
        &json!({ "page_id": page_id, "url": format!("{origin}{PAGE_FUNCTIONS}") }),
    )?;
    let went = connection.browser_answer(id)?.message;
    checks.check(
        "the page functions page loads",
        went["result"]["response"]["status"] == json!(200),
    );
    Ok(Some((page_id, registered)))
}

/// The answer to `page.evaluate` of `function` with `arg` in `world` on
/// `page`.
fn evaluate(
    connection: &mut Connection,
    page: &Value,
    function: &str,
    arg: Option<Value>,
    world: &str,
) -> Result<Value, Gone> {
    let mut params = json!({ "page_id": page, "function": function, "world": world });
    if let Some(arg) = arg {
        params["arg"] = arg;
    }
    let id = connection.request(PAGE_EVALUATE.name, &params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// The answer to `locator.evaluate_all` of `function` with `arg` in
/// `world` on the matches of `selector` on `page`.
fn evaluated_all(
    connection: &mut Connection,
    (page, selector): (&Value, &str),
    function: &str,
    arg: Option<Value>,
    world: &str,
) -> Result<Value, Gone> {
    let mut params = json!({
        "locator": { "page_id": page, "selector": selector },
        "function": function,
        "world": world,
    });
    if let Some(arg) = arg {
        params["arg"] = arg;
    }
    let id = connection.request(LOCATOR_EVALUATE_ALL.name, &params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// The details of the Error code `code` an answer raised, if it raised it.
fn details<T: serde::de::DeserializeOwned>(answer: &Value, code: &str) -> Option<T> {
    (answer["error"]["data"]["code"] == json!(code))
        .then(|| serde_json::from_value(answer["error"]["data"].clone()).ok())
        .flatten()
}

/// A function's JSON comes back as `JSON.stringify` gives it, from the main
/// world, where the page's global is; an expression answers its value; a
/// function that returns nothing answers no value; an isolated world reads
/// the page's document but not its global; and a page that replaced the
/// built-ins a walk could call still gets its result whole.
fn answers_json(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-function-json";
    let Some((page, _)) = on_the_page(connection, site, run_id, checks, |_, _| Ok(None))? else {
        return Ok(());
    };
    let answer = evaluate(
        connection,
        &page,
        "async ({ n }) => ({ global: window.pageGlobal, doubled: n * 2, list: [1, undefined, -0], gone: undefined })",
        Some(json!({ "n": 21 })),
        "main",
    )?;
    checks.check(
        "a main-world function answers its awaited result as JSON, the page's global in it, `undefined` dropped or `null` and `-0` as `0`",
        answer["result"]
            == json!({ "value": { "global": "from the page", "doubled": 42, "list": [1, null, 0] } }),
    );
    let answer = evaluate(connection, &page, "document.title", None, "main")?;
    checks.check(
        "an expression answers its value",
        answer["result"] == json!({ "value": "page-functions" }),
    );
    let answer = evaluate(
        connection,
        &page,
        "() => { window.touched = true; }",
        None,
        "main",
    )?;
    checks.check(
        "a function that returns nothing answers no value",
        answer["result"] == json!({}),
    );
    let answer = evaluate(
        connection,
        &page,
        "(selector) => [typeof window.pageGlobal, document.querySelector(selector).dataset.id]",
        Some(json!("article")),
        "isolated",
    )?;
    checks.check(
        "an isolated-world function reads the page's document but not its global",
        answer["result"] == json!({ "value": ["undefined", "page-functions"] }),
    );
    evaluate(connection, &page, REPLACED_BUILT_INS, None, "main")?;
    let answer = evaluate(
        connection,
        &page,
        "() => ({ rows: [1, 2, 3], nested: [{ cell: 'a', gone: undefined }], ['__proto__']: 'kept' })",
        None,
        "main",
    )?;
    checks.check(
        "a main-world result keeps its arrays and objects on a page that replaced `Array.from` as Prototype.js does, and `Object.fromEntries`, `Set` and `Array.prototype.push`",
        answer["result"]
            == json!({ "value": { "rows": [1, 2, 3], "nested": [{ "cell": "a" }], "__proto__": "kept" } }),
    );
    finish_run(connection, run_id)
}

/// A page script replacing built-ins a walk of a result could call:
/// `Array.from` as Prototype.js 1.7.3 does (its `$A`, which ignores a map
/// function), and `Object.fromEntries`, `Set` and `Array.prototype.push`
/// with ones that lose what they are given.
const REPLACED_BUILT_INS: &str = "() => {
  Array.from = function $A(iterable) {
    if (!iterable) return [];
    if ('toArray' in Object(iterable)) return iterable.toArray();
    var length = iterable.length || 0, results = new Array(length);
    while (length--) results[length] = iterable[length];
    return results;
  };
  Object.fromEntries = () => ({});
  window.Set = function Set() { return { add() {}, delete() {}, has() { return true; } }; };
  Array.prototype.push = function push() { return 0; };
}";

/// A DOM node, `NaN` and a cycle each raise `page_value_invalid` naming
/// the first such value's path and its kind, in either world.
fn outside_json(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-function-outside-json";
    let Some((page, _)) = on_the_page(connection, site, run_id, checks, |_, _| Ok(None))? else {
        return Ok(());
    };
    let cases = [
        (
            "() => ({ rows: [{ cell: document.body }] })",
            "main",
            "$.rows[0].cell",
            "node",
        ),
        ("() => document.body", "isolated", "$", "node"),
        ("() => [1, NaN]", "main", "$[1]", "not_a_number"),
        (
            "() => { const a = { b: {} }; a.b.a = a; return a; }",
            "main",
            "$.b.a",
            "cycle",
        ),
    ];
    for (function, world, path, kind) in cases {
        let answer = evaluate(connection, &page, function, None, world)?;
        let raised = details::<PageValueInvalidDetails>(&answer, PAGE_VALUE_INVALID.name);
        checks.check(
            format!(
                "`{function}` in the {world} world raises `{}` naming `{path}` and `{kind}`",
                PAGE_VALUE_INVALID.name
            ),
            raised.is_some_and(|details| {
                details.path == path
                    && serde_json::to_value(details.kind).ok() == Some(json!(kind))
                    && !details.fix.is_empty()
            }),
        );
    }
    finish_run(connection, run_id)
}

/// A function reading a variable of the program raises
/// `page_function_failed` with the page's `ReferenceError`, whose fix names
/// the argument; a thrown error keeps its name and message; and an
/// expression given an argument is refused.
fn thrown(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-function-thrown";
    let Some((page, _)) = on_the_page(connection, site, run_id, checks, |_, _| Ok(None))? else {
        return Ok(());
    };
    let answer = evaluate(connection, &page, "() => programVariable * 2", None, "main")?;
    checks.check(
        format!(
            "reading a variable of the program raises `{}` with a `ReferenceError` whose fix names the argument",
            PAGE_FUNCTION_FAILED.name
        ),
        details::<PageFunctionFailedDetails>(&answer, PAGE_FUNCTION_FAILED.name).is_some_and(
            |details| {
                details.name.as_deref() == Some("ReferenceError")
                    && details.fix.is_some_and(|fix| {
                        fix.contains("programVariable") && fix.contains("argument")
                    })
            },
        ),
    );
    let answer = evaluate(
        connection,
        &page,
        "() => { throw new TypeError('the page said no'); }",
        None,
        "isolated",
    )?;
    checks.check(
        "a thrown error keeps its name and its message",
        details::<PageFunctionFailedDetails>(&answer, PAGE_FUNCTION_FAILED.name).is_some_and(
            |details| {
                details.name.as_deref() == Some("TypeError")
                    && details.message == "the page said no"
                    && details.stack.is_some()
            },
        ),
    );
    let answer = evaluate(
        connection,
        &page,
        "() => { const frame = document.createElement('iframe'); document.body.append(frame); const Thrown = frame.contentWindow.RangeError; return { get read() { throw new Thrown('another realm said no'); } }; }",
        None,
        "main",
    )?;
    checks.check(
        "an error of another realm thrown while the result is read keeps its name",
        details::<PageFunctionFailedDetails>(&answer, PAGE_FUNCTION_FAILED.name).is_some_and(
            |details| {
                details.name.as_deref() == Some("RangeError")
                    && details.message == "another realm said no"
            },
        ),
    );
    let answer = evaluate(connection, &page, "document.title", Some(json!(1)), "main")?;
    checks.check(
        format!(
            "an expression given an argument raises `{}`",
            PAGE_FUNCTION_FAILED.name
        ),
        details::<PageFunctionFailedDetails>(&answer, PAGE_FUNCTION_FAILED.name).is_some(),
    );
    finish_run(connection, run_id)
}

/// The Provenance entries of the Run `run_id` that mark a main-world
/// injection, each as `(by, session_id)`.
fn injections(target: &EngineTarget, run_id: &str) -> Vec<(Value, Value)> {
    provenance(target, run_id)
        .into_iter()
        .filter(|line| line["action"] == json!(MAIN_WORLD_INJECTION))
        .map(|line| (line["by"].clone(), line["session_id"].clone()))
        .collect()
}

/// An init script registered before a navigation runs before the page's
/// own script and in a frame added afterwards, of the page's site or of
/// another site in a process of its own, its call staying in flight,
/// and Provenance marks the Session; once its call is cancelled, the next
/// document runs without it; and a second Session of the Run, whose page
/// has no init script and only isolated calls, records no injection.
fn init_script(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-function-init-script";
    let Some((page, Some(script))) = on_the_page(
        connection,
        site,
        run_id,
        checks,
        |connection, page| {
            connection
                .request(
                    PAGE_ADD_INIT_SCRIPT.name,
                    &json!({
                        "page_id": page,
                        "function": "(mark) => { window.initScriptMark = mark; if (window !== window.top) { window.top.postMessage({ initScriptMark: mark }, '*'); } }",
                        "arg": MARK,
                    }),
                )
                .map(Some)
        },
    )?
    else {
        return Ok(());
    };
    let answer = evaluate(
        connection,
        &page,
        "() => window.initScriptSeenByPage",
        None,
        "main",
    )?;
    checks.check(
        "the init script ran before the page's own script",
        answer["result"] == json!({ "value": MARK }),
    );
    let answer = evaluate(
        connection,
        &page,
        "async (path) => { const frame = document.createElement('iframe'); const loaded = new Promise((resolve) => frame.addEventListener('load', resolve)); frame.src = path; document.body.append(frame); await loaded; return frame.contentWindow.initScriptSeenByPage; }",
        Some(json!(PAGE_FUNCTIONS)),
        "main",
    )?;
    checks.check(
        "the init script ran in a frame added afterwards, before its document's own script",
        answer["result"] == json!({ "value": MARK }),
    );
    let answer = evaluate(
        connection,
        &page,
        "async (src) => { const marked = new Promise((resolve) => { window.addEventListener('message', (event) => { if (event.data && event.data.initScriptMark !== undefined) { resolve(event.data.initScriptMark); } }); setTimeout(() => resolve(null), 5000); }); const frame = document.createElement('iframe'); frame.src = src; document.body.append(frame); return await marked; }",
        Some(json!(
            site.named_url(FRAME_HOST, "/framed").unwrap_or_default()
        )),
        "main",
    )?;
    checks.check(
        "the init script ran in a frame of another site, which the browser loads in a process of its own",
        answer["result"] == json!({ "value": MARK }),
    );
    connection.notify(CANCEL, &json!({ "id": script }))?;
    let ended = connection.answer(script)?.message;
    checks.check(
        "cancelling the init script's call ends it with `call_cancelled`",
        ended["error"]["data"]["code"] == json!(CALL_CANCELLED.name),
    );
    let id = connection.request(PAGE_RELOAD.name, &json!({ "page_id": page }))?;
    connection.browser_answer(id)?;
    let answer = evaluate(
        connection,
        &page,
        "() => window.initScriptSeenByPage",
        None,
        "main",
    )?;
    checks.check(
        "a document loaded after the call was cancelled runs without the init script",
        answer["result"] == json!({ "value": null }),
    );
    registered_or_not(connection, &page, checks)?;
    let plain = plain_session(connection, site, run_id, checks)?;
    finish_run(connection, run_id)?;
    let marked = injections(target, run_id);
    checks.check(
        "the Session's Provenance marks the init script and the main-world calls as main-world injections",
        marked.iter().any(|(by, _)| *by == json!(PAGE_ADD_INIT_SCRIPT.name))
            && marked.iter().any(|(by, _)| *by == json!(PAGE_EVALUATE.name))
            && marked.iter().all(|(_, session)| session.is_string()),
    );
    checks.check(
        "a Session whose page had no init script and no main-world call records no injection",
        plain.is_some_and(|plain| marked.iter().all(|(_, session)| *session != plain)),
    );
    Ok(())
}

/// An init script whose call is cancelled as soon as it is sent is never
/// left registered: the next document runs without it; and a page function
/// sent right after another init script's call, which reloads the page,
/// meets the script registered.
fn registered_or_not(
    connection: &mut Connection,
    page: &Value,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let script = |mark: &str| {
        json!({
            "page_id": page,
            "function": "(mark) => { window.initScriptMark = mark; }",
            "arg": mark,
        })
    };
    let cancelled = connection.request(PAGE_ADD_INIT_SCRIPT.name, &script("cancelled at once"))?;
    connection.notify(CANCEL, &json!({ "id": cancelled }))?;
    connection.answer(cancelled)?;
    let id = connection.request(PAGE_RELOAD.name, &json!({ "page_id": page }))?;
    connection.browser_answer(id)?;
    let answer = evaluate(
        connection,
        page,
        "() => window.initScriptSeenByPage",
        None,
        "main",
    )?;
    checks.check(
        "an init script whose call was cancelled as it was sent is not left registered",
        answer["result"] == json!({ "value": null }),
    );
    let kept = connection.request(PAGE_ADD_INIT_SCRIPT.name, &script("before the reload"))?;
    let reloaded = evaluate(
        connection,
        page,
        "() => { window.beforeTheReload = true; location.reload(); return true; }",
        None,
        "main",
    )?;
    let id = connection.request(
        PAGE_WAIT_FOR_FUNCTION.name,
        &json!({
            "page_id": page,
            "function": "() => window.beforeTheReload === undefined && document.readyState === 'complete' && (window.initScriptSeenByPage ?? 'none')",
        }),
    )?;
    let seen = connection.browser_answer(id)?.message;
    checks.check(
        "a page function sent right after an init script's call meets the script registered",
        reloaded["result"] == json!({ "value": true })
            && seen["result"] == json!({ "value": "before the reload" }),
    );
    connection.notify(CANCEL, &json!({ "id": kept }))?;
    connection.answer(kept)?;
    Ok(())
}

/// Opens a second Session of the Run `run_id`, loads the page functions
/// page in a page of it and calls only an isolated-world function there:
/// the Session's ID.
fn plain_session(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    let id = connection.request(RUN_NEW_SESSION.name, &json!({ "run_id": run_id }))?;
    let session = connection.browser_answer(id)?.message["result"]["session_id"].clone();
    let id = connection.request(SESSION_NEW_PAGE.name, &json!({ "session_id": session }))?;
    let page = connection.browser_answer(id)?.message["result"]["page_id"].clone();
    let origin = site.access_origin.clone().unwrap_or_default();
    let id = connection.request(
        PAGE_GOTO.name,
        &json!({ "page_id": page, "url": format!("{origin}{PAGE_FUNCTIONS}") }),
    )?;
    connection.browser_answer(id)?;
    let answer = evaluate(connection, &page, "() => document.title", None, "isolated")?;
    checks.check(
        "an isolated-world call answers in a second Session",
        answer["result"] == json!({ "value": "page-functions" }),
    );
    Ok(session.is_string().then_some(session))
}

/// Rows the page lists, one of them inside an open shadow root, which
/// `locator.evaluate_all` reaches as the guard's CSS does.
const ROWS: &str = "() => {
  const list = document.createElement('ul');
  for (const [index, name] of ['a', 'b', 'c'].entries()) {
    const row = document.createElement('li');
    row.dataset.price = String(index + 1);
    row.textContent = name;
    list.append(row);
  }
  const host = document.createElement('div');
  host.attachShadow({ mode: 'open' }).innerHTML = '<li data-price=\"4\">d</li>';
  document.body.append(list, host);
}";

/// `locator.evaluate_all` hands its function every match, an open shadow
/// root's included, and its JSON argument after them, in the main world,
/// where the page's global is, and in an isolated world; no match is an
/// empty array; a match in the result raises `page_value_invalid`, a
/// program variable `page_function_failed` whose fix names the argument,
/// and a selector the page does not parse `invalid_params`; and the
/// Session's Provenance marks the main-world call.
fn evaluate_all(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-function-evaluate-all";
    let Some((page, _)) = on_the_page(connection, site, run_id, checks, |_, _| Ok(None))? else {
        return Ok(());
    };
    evaluate(connection, &page, ROWS, None, "main")?;
    let answer = evaluated_all(
        connection,
        (&page, "li"),
        "(rows, { factor }) => ({ global: window.pageGlobal, rows: rows.map((row) => [row.textContent, Number(row.dataset.price) * factor]) })",
        Some(json!({ "factor": 10 })),
        "main",
    )?;
    checks.check(
        "a main-world function takes every match, an open shadow root's included, then its argument, and answers JSON",
        answer["result"]
            == json!({ "value": { "global": "from the page", "rows": [["a", 10], ["b", 20], ["c", 30], ["d", 40]] } }),
    );
    let answer = evaluated_all(
        connection,
        (&page, "li"),
        "(rows) => [typeof window.pageGlobal, rows.length]",
        None,
        "isolated",
    )?;
    checks.check(
        "an isolated-world function takes the matches but sees no page global",
        answer["result"] == json!({ "value": ["undefined", 4] }),
    );
    let answer = evaluated_all(
        connection,
        (&page, "#none"),
        "(rows) => rows.length",
        None,
        "main",
    )?;
    checks.check(
        "a locator that matches nothing hands an empty array",
        answer["result"] == json!({ "value": 0 }),
    );
    let answer = evaluated_all(
        connection,
        (&page, "li"),
        "(rows) => ({ first: rows[0] })",
        None,
        "isolated",
    )?;
    checks.check(
        format!(
            "a match in the result raises `{}` naming `$.first` and `node`",
            PAGE_VALUE_INVALID.name
        ),
        details::<PageValueInvalidDetails>(&answer, PAGE_VALUE_INVALID.name).is_some_and(
            |details| {
                details.path == "$.first"
                    && serde_json::to_value(details.kind).ok() == Some(json!("node"))
            },
        ),
    );
    let answer = evaluated_all(
        connection,
        (&page, "li"),
        "(rows) => rows.map(() => programVariable)",
        None,
        "main",
    )?;
    checks.check(
        format!(
            "reading a variable of the program raises `{}` whose fix names the argument in a `{}` call, after the matches",
            PAGE_FUNCTION_FAILED.name, LOCATOR_EVALUATE_ALL.name
        ),
        details::<PageFunctionFailedDetails>(&answer, PAGE_FUNCTION_FAILED.name).is_some_and(
            |details| {
                details.name.as_deref() == Some("ReferenceError")
                    && details.fix.is_some_and(|fix| {
                        fix.contains("locator.evaluate_all(\"(elements, programVariable)")
                            || fix.contains("locator.evaluateAll((elements, programVariable)")
                    })
            },
        ),
    );
    let answer = evaluated_all(
        connection,
        (&page, "li["),
        "(rows) => rows.length",
        None,
        "main",
    )?;
    checks.check(
        "a selector the page does not parse raises `invalid_params`",
        answer["error"]["code"] == json!(-32602),
    );
    finish_run(connection, run_id)?;
    checks.check(
        "the Session's Provenance marks the main-world `locator.evaluate_all`",
        injections(target, run_id)
            .iter()
            .any(|(by, _)| *by == json!(LOCATOR_EVALUATE_ALL.name)),
    );
    Ok(())
}
