//! The page formats cases (spec #580; ADR 0007): a page's main content as
//! Markdown, on the Proving ground's Markdown pages of the `extraction`
//! family, each judged by the oracle; only the formats a request names in
//! its result; and a page with no main content answered as a value.

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::{PAGE_FORMATS, PAGE_GOTO, PageFormats, PageId};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::Submission;

/// The task whose pages and oracle judge a Markdown format.
const MARKDOWN_TASK: &str = "extraction/7";

/// The page of the task with no main content.
const NO_MAIN_CONTENT: &str = "/markdown/none/navigation-only";

/// Takes `page` to `url`, checking the navigation is answered.
fn goto(
    connection: &mut Connection,
    page: &PageId,
    url: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let id = connection.request(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    let answer = connection.browser_answer(id)?.message;
    checks.check(
        format!("`{}` to {url} is answered", PAGE_GOTO.name),
        answer.get("result").is_some(),
    );
    Ok(())
}

/// `page.formats` of `page` for `formats`: its result, as JSON.
fn formats(
    connection: &mut Connection,
    page: &PageId,
    formats: &Value,
) -> Result<Option<Value>, Gone> {
    let id = connection.request(
        PAGE_FORMATS.name,
        &json!({ "page_id": page, "formats": formats }),
    )?;
    Ok(connection
        .browser_answer(id)?
        .message
        .get("result")
        .cloned())
}

/// The task's start page and the paths of the pages it lists.
fn markdown_pages(site: &Site, checks: &mut Checks) -> Option<(String, Vec<String>)> {
    let index = site.start_url(MARKDOWN_TASK).map(ToOwned::to_owned);
    checks.check(
        format!("the Proving ground serves task `{MARKDOWN_TASK}`"),
        index.is_some(),
    );
    let index = index?;
    let listing = site.proving_ground.page(&index).ok();
    checks.check("the task's start page is served", listing.is_some());
    let paths = listing?
        .split("href=\"")
        .skip(1)
        .filter_map(|link| link.split('"').next())
        .map(ToOwned::to_owned)
        .collect();
    Some((index, paths))
}

/// The origin of an `http` URL: its scheme, host and port.
fn origin_of(url: &str) -> &str {
    let host = url.find("://").map_or(0, |at| at + "://".len());
    url[host..].find('/').map_or(url, |at| &url[..host + at])
}

/// Each Markdown page's main content, read through `page.formats`, is the
/// Markdown the oracle holds for it, judged by structure: the lab's 30
/// hand-written fixtures and a page with no main content (ADR 0007).
pub(super) fn markdown_matches_the_oracle(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, paths)) = markdown_pages(site, checks) else {
        return Ok(());
    };
    let origin = origin_of(&index).to_owned();
    let run_id = "conformance-formats-markdown";
    start_run(connection, site, run_id, checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let mut records = Vec::new();
        for path in &paths {
            goto(
                connection,
                &page.page_id,
                &format!("{origin}{path}"),
                checks,
            )?;
            let read = formats(connection, &page.page_id, &json!({ "markdown": true }))?
                .and_then(|result| serde_json::from_value::<PageFormats>(result).ok());
            checks.check(
                format!("`{}` of {path} answers its result", PAGE_FORMATS.name),
                read.is_some(),
            );
            if let Some(markdown) = read.and_then(|read| read.markdown) {
                records.push(json!({ "id": path, "markdown": markdown }));
            }
        }
        let task = TaskId::try_from(MARKDOWN_TASK.to_owned())
            .expect("invariant: the Markdown task's ID is a task ID");
        let answer = site.proving_ground.judge(&Submission {
            task: &task,
            status: "success",
            records: &records,
        });
        checks.check(
            "the oracle verifies the Markdown of every page",
            answer
                .as_ref()
                .is_ok_and(|answer| answer.outcome == OracleOutcome::Verified),
        );
        if let Ok(answer) = answer {
            checks.oracle_answers.push(answer);
        }
    }
    finish_run(connection, run_id)
}

/// A request that names no format answers the envelope alone.
pub(super) fn a_format_not_named_is_left_out(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, paths)) = markdown_pages(site, checks) else {
        return Ok(());
    };
    let Some(path) = paths.first() else {
        checks.check("the task lists a page", false);
        return Ok(());
    };
    let url = format!("{}{path}", origin_of(&index));
    let run_id = "conformance-formats-envelope";
    start_run(connection, site, run_id, checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        goto(connection, &page.page_id, &url, checks)?;
        let read = formats(connection, &page.page_id, &json!({}))?;
        let keys: Option<Vec<String>> = read
            .as_ref()
            .and_then(Value::as_object)
            .map(|result| result.keys().cloned().collect());
        checks.check(
            "the result holds the URL, the base URL and whether there is main content, and no format",
            keys.as_deref()
                == Some(&["base_url".to_owned(), "main_content".to_owned(), "url".to_owned()][..]),
        );
    }
    finish_run(connection, run_id)
}

/// A page with no main content answers empty Markdown and `none`, a
/// value, never an error nor the page's navigation (pitfall D02).
pub(super) fn no_main_content_is_none(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, _)) = markdown_pages(site, checks) else {
        return Ok(());
    };
    let url = format!("{}{NO_MAIN_CONTENT}", origin_of(&index));
    let run_id = "conformance-formats-none";
    start_run(connection, site, run_id, checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        goto(connection, &page.page_id, &url, checks)?;
        let read = formats(connection, &page.page_id, &json!({ "markdown": true }))?;
        checks.check("the call answers a result, not an error", read.is_some());
        checks.check(
            "its main content is `none` and its Markdown empty",
            read.as_ref().is_some_and(|read| {
                read["main_content"] == json!("none") && read["markdown"] == json!("")
            }),
        );
    }
    finish_run(connection, run_id)
}
