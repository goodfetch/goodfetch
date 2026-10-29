//! The page formats cases (spec #580; ADRs 0007, 0047 and 0121): a page's
//! main content as Markdown, on the Proving ground's Markdown pages of the
//! `extraction` family, each judged by the oracle; only the formats a
//! request names in its result; a page with no main content answered as a
//! value; the document read as it stands, after its scripts ran, with its
//! open and closed shadow roots; the Clean-up rules' cosmetic rules applied
//! to the Markdown only; and each call in the Run trace with the rules'
//! version and what they removed. How much of the page a call reads is
//! `content_modes`', the links, images, media and tables `references`', and
//! the metadata and contacts `structured`'.

pub(super) mod content_modes;
pub(super) mod extractor;
pub(super) mod references;
pub(super) mod structured;

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::waits::{LOCATOR_WAIT_FOR, PAGE_WAIT_FOR_FUNCTION};
use goodfetch_protocol::page::{PAGE_FORMATS, PAGE_GOTO, PageFormats, PageId};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::{Value, json};

use super::navigation::{answered, call};
use super::{Checks, Site, clean_up, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;
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

/// The Markdown task's start page and the paths of the pages it lists.
fn markdown_pages(site: &Site, checks: &mut Checks) -> Option<(String, Vec<String>)> {
    task_pages(site, MARKDOWN_TASK, checks)
}

/// The start page of the task `task` and the paths of the pages it lists.
fn task_pages(site: &Site, task: &str, checks: &mut Checks) -> Option<(String, Vec<String>)> {
    let index = site.start_url(task).map(ToOwned::to_owned);
    checks.check(
        format!("the Proving ground serves task `{task}`"),
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

/// Each Markdown page's main content, read through `page.formats` with the
/// main-content step `extractor` names (`content.extractor`, the default
/// when none), is the Markdown the oracle holds for it, judged by
/// structure: the lab's 30 hand-written fixtures and a page with no main
/// content (ADR 0007).
pub(super) fn markdown_matches_the_oracle(
    connection: &mut Connection,
    site: &Site,
    extractor: Option<&str>,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, paths)) = markdown_pages(site, checks) else {
        return Ok(());
    };
    let origin = origin_of(&index).to_owned();
    let run_id = &format!(
        "conformance-formats-markdown{}",
        extractor.map(|name| format!("-{name}")).unwrap_or_default()
    );
    start_run(
        connection,
        site,
        run_id,
        &extractor::run_settings(extractor),
        checks,
    )?;
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
            format!(
                "the oracle verifies the Markdown of every page ({:?})",
                answer.as_ref().map(|answer| &answer.outcome)
            ),
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
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        goto(connection, &page.page_id, &url, checks)?;
        let read = formats(connection, &page.page_id, &json!({}))?;
        let keys: Option<Vec<String>> = read
            .as_ref()
            .and_then(Value::as_object)
            .map(|result| result.keys().cloned().collect());
        checks.check(
            "the result holds the URL and the base URL, and no format nor whether there is main content",
            keys.as_deref() == Some(&["base_url".to_owned(), "url".to_owned()][..]),
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
    start_run(connection, site, run_id, &json!({}), checks)?;
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

/// The `rendered` task whose item page a script fills after `load` (#166).
const RENDERED_TASK: &str = "rendered/2";

/// The Markdown `page.formats` answers for `page`, empty when it answers
/// none.
fn markdown_of(connection: &mut Connection, page: &PageId) -> Result<String, Gone> {
    Ok(formats(connection, page, &json!({ "markdown": true }))?
        .and_then(|result| result["markdown"].as_str().map(ToOwned::to_owned))
        .unwrap_or_default())
}

/// The `formats` lines of the Run `run_id`'s Provenance log.
fn formats_entries(target: &EngineTarget, run_id: &str) -> Vec<Value> {
    clean_up::provenance(target, run_id)
        .into_iter()
        .filter(|line| line["action"] == json!("formats"))
        .collect()
}

/// On #166's item page, the name its script puts in an open shadow root
/// and the price it writes once its request for the item answers, both
/// after `load`, are in the Markdown (ADR 0121).
pub(super) fn text_rendered_after_load_is_read(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let start = site.start_url(RENDERED_TASK).map(ToOwned::to_owned);
    checks.check(
        format!("the Proving ground serves task `{RENDERED_TASK}`"),
        start.is_some(),
    );
    let Some(start) = start else {
        return Ok(());
    };
    let item = start
        .rsplit_once("/items/")
        .map(|(origin, id)| format!("{origin}/api/items/{id}"))
        .and_then(|url| site.proving_ground.page(&url).ok())
        .and_then(|body| serde_json::from_str::<Value>(&body).ok());
    let (Some(name), Some(price)) = (
        item.as_ref().and_then(|item| item["name"].as_str()),
        item.as_ref().and_then(|item| item["price"].as_str()),
    ) else {
        checks.check(
            "the Proving ground answers the item's name and price",
            false,
        );
        return Ok(());
    };
    let run_id = "conformance-formats-rendered";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        goto(connection, &page.page_id, &start, checks)?;
        let id = connection.request(
            PAGE_WAIT_FOR_FUNCTION.name,
            &json!({
                "page_id": page.page_id,
                "function": "() => document.querySelector('.price').dataset.price !== undefined",
            }),
        )?;
        let filled = connection.browser_answer(id)?.message;
        checks.check(
            "the item page's script fills it after `load`",
            filled.get("result").is_some(),
        );
        let markdown = markdown_of(connection, &page.page_id)?;
        checks.check(
            format!("the Markdown holds the name in the open shadow root, {name:?} ({markdown:?})"),
            markdown.contains(name),
        );
        checks.check(
            format!("the Markdown holds the price the script wrote, ${price}"),
            markdown.contains(&format!("${price}")),
        );
    }
    finish_run(connection, run_id)
}

/// On `/shadow` of `clean-up.test`, the text of its open shadow root and of
/// the closed one its script attaches is in the Markdown, on Chromium,
/// without the ads a cosmetic rule matches in either (ADRs 0047, 0121).
pub(super) fn closed_shadow_root_text_is_read(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-formats-shadow-roots";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let (url, answer) = clean_up::visit(connection, site, &page, "/shadow")?;
        answered(&answer, "the `goto` to `/shadow`", &url, 200, checks);
        let markdown = markdown_of(connection, &page.page_id)?;
        checks.check(
            format!("the Markdown holds the open and the closed shadow root's text ({markdown:?})"),
            markdown.contains("Text in an open root.")
                && markdown.contains("Text in a closed root."),
        );
        checks.check(
            "the Markdown holds neither shadow root's ad",
            !markdown.contains("An open-root ad") && !markdown.contains("A closed-root ad"),
        );
    }
    finish_run(connection, run_id)
}

/// On `/hiding`, the ad a cosmetic rule hides is in the document as the
/// page holds it and a program's own locator finds it, but the Markdown
/// leaves it out (ADR 0047).
pub(super) fn the_rules_touch_only_the_main_content(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-formats-rules-scope";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let (url, answer) = clean_up::visit(connection, site, &page, "/hiding")?;
        answered(&answer, "the `goto` to `/hiding`", &url, 200, checks);
        let markdown = markdown_of(connection, &page.page_id)?;
        checks.check(
            "the Markdown holds the article's text and not the ad's",
            markdown.contains(clean_up::ARTICLE_TEXT) && !markdown.contains(clean_up::AD_TEXT),
        );
        let function = format!(
            "() => document.documentElement.outerHTML.includes({:?})",
            clean_up::AD_TEXT
        );
        let serialized = call(
            connection,
            PAGE_EVALUATE.name,
            &json!(page.page_id),
            &json!({ "function": function }),
        )?;
        checks.check(
            "the document's HTML as the page holds it still holds the ad",
            serialized["result"]["value"] == json!(true),
        );
        let id = connection.request(
            LOCATOR_WAIT_FOR.name,
            &json!({
                "locator": { "page_id": page.page_id, "selector": ".ad-slot" },
                "state": "attached",
            }),
        )?;
        let found = connection.browser_answer(id)?.message;
        checks.check(
            format!("the program's own locator finds the ad ({found})"),
            found.get("result").is_some(),
        );
    }
    finish_run(connection, run_id)
}

/// The Run trace holds one `formats` entry per call, each naming the
/// Clean-up rules' version the Run holds to and what the rules removed;
/// with every Filter list off, the entry names no version and no removal
/// (ADR 0047; G6).
pub(super) fn each_call_is_in_the_run_trace(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-formats-trace";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        for path in ["/hiding", "/shadow"] {
            let (url, answer) = clean_up::visit(connection, site, &page, path)?;
            answered(&answer, &format!("the `goto` to {path}"), &url, 200, checks);
            markdown_of(connection, &page.page_id)?;
        }
        markdown_of(connection, &page.page_id)?;
    }
    finish_run(connection, run_id)?;
    let lines = clean_up::provenance(target, run_id);
    let version = lines
        .iter()
        .find(|line| line["action"] == json!("clean_up_rules"))
        .map(|line| line["digest"].clone());
    let entries = formats_entries(target, run_id);
    checks.check(
        format!(
            "the Run trace holds one `formats` entry per call, three ({})",
            entries.len()
        ),
        entries.len() == 3,
    );
    checks.check(
        "each names the version of the Clean-up rules the Run holds to",
        version.is_some()
            && entries
                .iter()
                .all(|entry| Some(&entry["clean_up_rules"]) == version.as_ref()),
    );
    let rules = |entry: &Value| -> Vec<Value> {
        entry["removed"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|removed| removed["rule"].clone())
            .collect()
    };
    checks.check(
        "the `/hiding` entry names the rule that removed its ad",
        entries
            .first()
            .is_some_and(|entry| rules(entry) == [json!(".ad-slot")]),
    );
    checks.check(
        "each `/shadow` entry names its three ads removed",
        entries.len() == 3
            && entries[1..].iter().all(|entry| {
                rules(entry).len() == 3
                    && entry["url"]
                        .as_str()
                        .is_some_and(|url| url.ends_with("/shadow"))
            }),
    );

    let run_id = "conformance-formats-trace-lists-off";
    let off = json!({ "filter_lists": { "ads": false, "trackers": false, "annoyances": false } });
    start_run(connection, site, run_id, &off, checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let (url, answer) = clean_up::visit(connection, site, &page, "/hiding")?;
        answered(&answer, "the `goto` to `/hiding`", &url, 200, checks);
        let markdown = markdown_of(connection, &page.page_id)?;
        checks.check(
            "with every Filter list off, the Markdown keeps the ad's text",
            markdown.contains(clean_up::AD_TEXT),
        );
    }
    finish_run(connection, run_id)?;
    let entries = formats_entries(target, run_id);
    checks.check(
        "with every Filter list off, the entry names no version and no removal",
        entries.len() == 1
            && entries[0]["clean_up_rules"].is_null()
            && entries[0]["removed"] == json!([]),
    );
    Ok(())
}
