//! The page suite's scroll collection cases (#807, ADR 0121), on lists a
//! case builds on the Proving ground's page functions page and on the
//! `rendered` family's infinite feed. `locator.scroll_collect` reads a
//! virtualized list of 1,000 rows whole, each row once, ends `end_of_list`
//! and says the 1,000 rows it declares, or ends `max_rows` at its cap; it
//! reads the feed to every item the feed serves; a list that stops loading
//! short of what it declares ends `end_of_list` with what it read. A call
//! past its timeout raises `deadline_exceeded` naming the rows it kept and
//! the last key, and a throw fails as `page.evaluate` fails. Each scroll is
//! a gesture through the guard, whose refusal stops the call with the
//! guard's code, and no other read scrolls.

use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails, WaitedFor};
use goodfetch_protocol::page::functions::{
    LOCATOR_EVALUATE_ALL, LOCATOR_SCROLL_COLLECT, PAGE_FUNCTION_FAILED, PageFunctionFailedDetails,
};
use goodfetch_protocol::page::read::PAGE_READ;
use goodfetch_protocol::page::waits::LOCATOR_WAIT_FOR;
use goodfetch_protocol::page::{PAGE_FORMATS, PAGE_GOTO};
use goodfetch_protocol::settings::TIMEOUT_ACTION;
use serde_json::{Value, json};

use super::{Checks, Site, details, evaluate, finish_run, new_page, on_the_page_under, start_run};
use crate::protocol::connection::{Connection, Gone};

/// The `rendered` task whose start page is the infinite feed.
const FEED_TASK: &str = "rendered/1";

/// How long a case's step waits for a new key: 1 s, short of the 2 s
/// default so a list's end costs less.
const STEP_WAIT: u64 = 1_000;

/// The whole call's timeout where a case expects it to end on its own:
/// 30 s.
const LONG_TIMEOUT: u64 = 30_000;

/// The whole call's timeout where a case expects it to pass mid-list.
const SHORT_TIMEOUT: u64 = 1_500;

/// `timeout.action` where a case expects the guard to refuse: 1 s.
const SHORT_ACTION: u64 = 1_000;

/// A virtualized list the page builds in its main world, with the
/// argument `{ declared, total, first, batch }`: a grid declaring
/// `declared` rows through `aria-rowcount`, holding `first` rows and
/// loading `batch` more each time the window reaches its end, up to
/// `total`, of which only the rows near the view are in the document, each
/// a `.row` keyed `r-<n>` with its `aria-rowindex`. `window.renderList`
/// renders the window again.
const LIST: &str = "({ declared, total, first, batch }) => {
  const HEIGHT = 20;
  const list = document.createElement('div');
  list.setAttribute('role', 'grid');
  list.setAttribute('aria-rowcount', String(declared));
  list.style.position = 'relative';
  document.body.append(list);
  let loaded = first;
  const render = () => {
    const top = list.getBoundingClientRect().top + window.scrollY;
    const from = Math.max(0, Math.floor((window.scrollY - top) / HEIGHT) - 5);
    const reach = from + Math.ceil(window.innerHeight / HEIGHT) + 10;
    while (loaded < Math.min(reach, total)) loaded = Math.min(total, loaded + batch);
    list.style.height = `${loaded * HEIGHT}px`;
    const rows = [];
    for (let index = from; index < Math.min(reach, loaded); index += 1) {
      const row = document.createElement('div');
      row.className = 'row';
      row.dataset.key = `r-${index + 1}`;
      row.setAttribute('aria-rowindex', String(index + 1));
      row.style.cssText = `position: absolute; top: ${index * HEIGHT}px; height: ${HEIGHT}px;`;
      row.textContent = `Row ${index + 1}`;
      rows.push(row);
    }
    list.replaceChildren(...rows);
  };
  window.addEventListener('scroll', render);
  window.renderList = render;
  render();
}";

/// The rows of [`LIST`].
const ROWS: &str = "[role=grid] .row";

/// A page function reading a [`LIST`] row: its key and its row index.
const ROW: &str =
    "(row) => ({ key: row.dataset.key, value: Number(row.getAttribute('aria-rowindex')) })";

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Virtualized,
    Feed,
    StoppedShort,
    TimeoutAndThrow,
    Guard,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "scroll-collect-reads-a-virtualized-list-whole-each-row-once-and-what-it-declares" => {
                Self::Virtualized
            }
            "scroll-collect-reads-an-infinite-feed-to-every-item-it-serves" => Self::Feed,
            "a-list-that-stops-loading-short-of-what-it-declares-ends-end-of-list-with-what-it-read" => {
                Self::StoppedShort
            }
            "a-scroll-collection-past-its-timeout-names-what-it-kept-and-a-throw-fails-as-evaluate-fails" => {
                Self::TimeoutAndThrow
            }
            "each-scroll-goes-through-the-guard-and-no-other-read-scrolls" => Self::Guard,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Virtualized => virtualized(connection, site, checks),
            Self::Feed => feed(connection, site, checks),
            Self::StoppedShort => stopped_short(connection, site, checks),
            Self::TimeoutAndThrow => timeout_and_throw(connection, site, checks),
            Self::Guard => guard(connection, site, checks),
        }
    }
}

/// A new page of the Run `run_id`, started with `settings`, on the page
/// functions page holding a [`LIST`] built with `list`: the page's ID.
fn on_a_list(
    connection: &mut Connection,
    site: &Site,
    (run_id, settings): (&str, &Value),
    list: &Value,
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    let Some((page, _)) =
        on_the_page_under(connection, site, (run_id, settings), checks, |_, _| {
            Ok(None)
        })?
    else {
        return Ok(None);
    };
    let built = evaluate(connection, &page, LIST, Some(list.clone()), "main")?;
    checks.check("the page builds its list", built.get("result").is_some());
    Ok(Some(page))
}

/// The answer to `locator.scroll_collect` of `function` on the matches of
/// `selector` on `page`, with the call's other `options`.
fn scroll_collected(
    connection: &mut Connection,
    (page, selector): (&Value, &str),
    function: &str,
    options: &Value,
) -> Result<Value, Gone> {
    let mut params = json!({
        "locator": { "page_id": page, "selector": selector },
        "function": function,
        "step_wait": STEP_WAIT,
        "timeout": LONG_TIMEOUT,
    });
    if let (Some(params), Some(options)) = (params.as_object_mut(), options.as_object()) {
        params.extend(options.clone());
    }
    let id = connection.request(LOCATOR_SCROLL_COLLECT.name, &params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// The page's vertical scroll position.
fn scrolled_to(connection: &mut Connection, page: &Value) -> Result<Value, Gone> {
    Ok(
        evaluate(connection, page, "() => window.scrollY", None, "main")?["result"]["value"]
            .clone(),
    )
}

/// The row indexes 1 to `rows`, as [`ROW`] reads them.
fn indexes(rows: u64) -> Value {
    json!((1..=rows).collect::<Vec<_>>())
}

/// On a virtualized list of 1,000 rows, `max_rows: 50` ends `max_rows` with
/// the first 50 rows, and from the top again the call reads the 1,000
/// rows whole, each once and in order, ends `end_of_list` and says the
/// 1,000 rows the list declares.
fn virtualized(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-scroll-collect-virtualized";
    let list = json!({ "declared": 1000, "total": 1000, "first": 1000, "batch": 1000 });
    let Some(page) = on_a_list(connection, site, (run_id, &json!({})), &list, checks)? else {
        return Ok(());
    };
    let answer = scroll_collected(connection, (&page, ROWS), ROW, &json!({ "max_rows": 50 }))?;
    checks.check(
        "`max_rows: 50` ends `max_rows` with the list's first 50 rows",
        answer["result"] == json!({ "rows": indexes(50), "ended": "max_rows", "declared": 1000 }),
    );
    evaluate(
        connection,
        &page,
        "() => { window.scrollTo(0, 0); window.renderList(); }",
        None,
        "main",
    )?;
    let answer = scroll_collected(connection, (&page, ROWS), ROW, &json!({}))?;
    checks.check(
        "the 1,000 rows come back whole, each once and in order, ending `end_of_list` with the 1,000 the list declares",
        answer["result"]
            == json!({ "rows": indexes(1000), "ended": "end_of_list", "declared": 1000 }),
    );
    finish_run(connection, run_id)
}

/// The feed's script loads its next page each time its end comes into
/// view: the call reads every item the feed serves, in its order, ends
/// `end_of_list` and says the feed declares nothing.
fn feed(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let Some(url) = site.start_url(FEED_TASK).map(str::to_owned) else {
        checks.check(
            format!("the Proving ground serves the task `{FEED_TASK}`"),
            false,
        );
        return Ok(());
    };
    let run_id = "conformance-scroll-collect-feed";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(opened) = new_page(connection, run_id, checks)? else {
        return Ok(());
    };
    let page = json!(opened.page_id);
    let went = connection.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    checks.check(
        "the feed loads",
        went["result"]["response"]["status"] == json!(200),
    );
    let answer = scroll_collected(
        connection,
        (&page, "li.item"),
        "(item) => ({ key: item.dataset.id, value: item.dataset.id })",
        &json!({}),
    )?;
    let served = evaluate(
        connection,
        &page,
        "async () => { const ids = []; let next = '/api/feed/1'; while (next !== null) { const page = await (await fetch(next)).json(); ids.push(...page.items.map((item) => item.id)); next = page.next; } return ids; }",
        None,
        "isolated",
    )?["result"]["value"]
        .clone();
    checks.check(
        "the feed serves items",
        served.as_array().is_some_and(|ids| !ids.is_empty()),
    );
    checks.check(
        "every item the feed serves comes back once, in its order, ending `end_of_list` with nothing declared",
        answer["result"] == json!({ "rows": served, "ended": "end_of_list", "declared": null }),
    );
    finish_run(connection, run_id)
}

/// A list declaring 1,000 rows that stops loading at 400 ends
/// `end_of_list` with its 400 rows and the 1,000 it declares, and raises
/// nothing.
fn stopped_short(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-scroll-collect-stopped-short";
    let list = json!({ "declared": 1000, "total": 400, "first": 50, "batch": 50 });
    let Some(page) = on_a_list(connection, site, (run_id, &json!({})), &list, checks)? else {
        return Ok(());
    };
    let answer = scroll_collected(connection, (&page, ROWS), ROW, &json!({}))?;
    checks.check(
        "the 400 rows loaded come back, ending `end_of_list` with the 1,000 the list declares and no error",
        answer["result"]
            == json!({ "rows": indexes(400), "ended": "end_of_list", "declared": 1000 }),
    );
    finish_run(connection, run_id)
}

/// A call whose timeout passes mid-list raises `deadline_exceeded` for the
/// list's end, whose `observed` names the rows it kept and the last key; a
/// page function that throws raises `page_function_failed` with its
/// error's name, as `page.evaluate` does.
fn timeout_and_throw(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-scroll-collect-timeout";
    let list = json!({ "declared": 1000, "total": 1000, "first": 1000, "batch": 1000 });
    let Some(page) = on_a_list(connection, site, (run_id, &json!({})), &list, checks)? else {
        return Ok(());
    };
    let answer = scroll_collected(
        connection,
        (&page, ROWS),
        ROW,
        &json!({ "timeout": SHORT_TIMEOUT }),
    )?;
    checks.check(
        format!(
            "a timeout passed mid-list raises `{}` for the list's end, naming the rows kept and the last key",
            DEADLINE_EXCEEDED.name
        ),
        details::<DeadlineExceededDetails>(&answer, DEADLINE_EXCEEDED.name).is_some_and(
            |details| {
                details.waited_for == WaitedFor::LIST_END
                    && details
                        .observed
                        .is_some_and(|observed| observed.contains("rows kept, the last key `r-"))
            },
        ),
    );
    let answer = scroll_collected(
        connection,
        (&page, ROWS),
        "(row) => { throw new TypeError('the row said no'); }",
        &json!({}),
    )?;
    checks.check(
        format!(
            "a page function that throws raises `{}` with its error's name",
            PAGE_FUNCTION_FAILED.name
        ),
        details::<PageFunctionFailedDetails>(&answer, PAGE_FUNCTION_FAILED.name)
            .is_some_and(|details| details.name.as_deref() == Some("TypeError")),
    );
    finish_run(connection, run_id)
}

/// Builds two rows, the last one hidden, which the guard never sees
/// visible.
const HIDDEN_LAST: &str = "() => {
  const list = document.createElement('ul');
  list.className = 'refused';
  for (const hidden of [false, true]) {
    const row = document.createElement('li');
    row.textContent = hidden ? 'hidden' : 'shown';
    row.hidden = hidden;
    list.append(row);
  }
  document.body.append(list);
}";

/// `page.read`, `page.formats`, `locator.evaluate_all` and
/// `locator.wait_for` leave the page where it is, which a scroll
/// collection then scrolls; and a last match the guard never sees visible
/// stops the call with the guard's `deadline_exceeded` under
/// `timeout.action`.
fn guard(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-scroll-collect-guard";
    let list = json!({ "declared": 1000, "total": 1000, "first": 1000, "batch": 1000 });
    let settings = json!({ "timeout": { "action": SHORT_ACTION } });
    let Some(page) = on_a_list(connection, site, (run_id, &settings), &list, checks)? else {
        return Ok(());
    };
    let locator = json!({ "page_id": page, "selector": ROWS });
    let reads = [
        (
            PAGE_READ.name,
            json!({ "page_id": page, "fields": { "row": { "locator": { "kind": "css", "selector": ROWS } } } }),
        ),
        (
            PAGE_FORMATS.name,
            json!({ "page_id": page, "formats": { "markdown": true } }),
        ),
        (
            LOCATOR_EVALUATE_ALL.name,
            json!({ "locator": locator, "function": "(rows) => rows.length" }),
        ),
        (
            LOCATOR_WAIT_FOR.name,
            json!({ "locator": { "page_id": page, "selector": "[role=grid]" }, "state": "attached" }),
        ),
    ];
    for (operation, params) in reads {
        let answer = connection.call(operation, &params)?;
        checks.check(
            format!("`{operation}` answers ({answer})"),
            answer.get("result").is_some(),
        );
    }
    let still = scrolled_to(connection, &page)?;
    checks.check(
        format!("no other read scrolls the page (`scrollY` {still})"),
        still == json!(0),
    );
    let answer = scroll_collected(connection, (&page, ROWS), ROW, &json!({ "max_rows": 100 }))?;
    let moved = scrolled_to(connection, &page)?;
    checks.check(
        format!("a scroll collection scrolls it (`scrollY` {moved}, {answer})"),
        answer.get("result").is_some() && moved.as_f64().is_some_and(|y| y > 0.0),
    );
    evaluate(connection, &page, HIDDEN_LAST, None, "main")?;
    let answer = scroll_collected(
        connection,
        (&page, ".refused li"),
        "(row) => ({ key: row.textContent, value: row.textContent })",
        &json!({}),
    )?;
    checks.check(
        format!(
            "a last match never visible stops the call with the guard's `{}` under `{}`",
            DEADLINE_EXCEEDED.name, TIMEOUT_ACTION.name
        ),
        details::<DeadlineExceededDetails>(&answer, DEADLINE_EXCEEDED.name).is_some_and(
            |details| {
                details.waited_for == WaitedFor::TARGET_VISIBLE
                    && details.limit.key == TIMEOUT_ACTION.name
            },
        ),
    );
    finish_run(connection, run_id)
}
