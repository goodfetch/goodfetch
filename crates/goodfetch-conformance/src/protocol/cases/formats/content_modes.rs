//! How much of the page a call reads (spec #580, ticket #785): each
//! Markdown content mode, each HTML kind and the text, judged by the
//! oracle on the `extraction` family's content-mode pages; `html.raw` as
//! `content()` returns it, before the Clean-up rules that `html.cleaned`
//! and the Markdown come after; the defaults; and an option the request
//! does not know refused as any invalid parameter is.

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::{PAGE_FORMATS, PageId};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::{Map, Value, json};

use super::super::navigation::{answered, call};
use super::super::{Checks, Site, clean_up, finish_run, new_page, refused_as, start_run};
use super::{formats, goto, origin_of, task_pages};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::Submission;

/// The task whose pages and oracle judge every form.
const TASK: &str = "extraction/8";

/// The page with a navigation list and a link-dense block.
const NAVIGATION_PAGE: &str = "/content-modes/navigation-and-related";

/// Each form a Record of the task holds: its field, the request that
/// asks for it and the key of the result that holds it.
const FORMS: [(&str, &str, &str); 7] = [
    (
        "markdown",
        r#"{ "markdown": { "content": "main" } }"#,
        "markdown",
    ),
    (
        "markdown_full",
        r#"{ "markdown": { "content": "full" } }"#,
        "markdown",
    ),
    (
        "markdown_fit",
        r#"{ "markdown": { "content": "fit" } }"#,
        "markdown",
    ),
    ("html_raw", r#"{ "html": { "kind": "raw" } }"#, "html"),
    (
        "html_cleaned",
        r#"{ "html": { "kind": "cleaned" } }"#,
        "html",
    ),
    ("html_main", r#"{ "html": { "kind": "main" } }"#, "html"),
    ("text", r#"{ "text": true }"#, "text"),
];

/// Playwright's `content()`: the doctype and the document element's
/// outer HTML, without a shadow root.
const CONTENT: &str = "() => (document.doctype ? new XMLSerializer().serializeToString(document.doctype) : '') + (document.documentElement ? document.documentElement.outerHTML : '')";

/// The value of `form` (its result key) that `request` answers on `page`.
fn form(
    connection: &mut Connection,
    page: &PageId,
    request: &Value,
    key: &str,
) -> Result<Option<String>, Gone> {
    Ok(formats(connection, page, request)?
        .and_then(|result| result[key].as_str().map(ToOwned::to_owned)))
}

/// On each content-mode page, every form `page.formats` gives, with the
/// main-content step `extractor` names (`content.extractor`, the default
/// when none), is what the oracle holds for it; "fit" is shorter than
/// "main" on the page with a navigation list and a link-dense block.
pub(in crate::protocol::cases) fn every_form_matches_the_oracle(
    connection: &mut Connection,
    site: &Site,
    extractor: Option<&str>,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, paths)) = task_pages(site, TASK, checks) else {
        return Ok(());
    };
    let origin = origin_of(&index).to_owned();
    let run_id = &format!(
        "conformance-formats-content-modes{}",
        extractor.map(|name| format!("-{name}")).unwrap_or_default()
    );
    start_run(
        connection,
        site,
        run_id,
        &super::extractor::run_settings(extractor),
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
            let mut record = Map::new();
            record.insert("id".to_owned(), json!(path));
            for (field, request, key) in FORMS {
                let request: Value =
                    serde_json::from_str(request).expect("invariant: each form's request is JSON");
                let value = form(connection, &page.page_id, &request, key)?;
                checks.check(
                    format!("`{}` of {path} answers its `{field}`", PAGE_FORMATS.name),
                    value.is_some(),
                );
                if let Some(value) = value {
                    record.insert(field.to_owned(), json!(value));
                }
            }
            if path == NAVIGATION_PAGE {
                let length = |field: &str| record.get(field).and_then(Value::as_str).map(str::len);
                checks.check(
                    format!(
                        "on {path}, fit Markdown is shorter than main Markdown ({:?} < {:?})",
                        length("markdown_fit"),
                        length("markdown")
                    ),
                    matches!(
                        (length("markdown_fit"), length("markdown")),
                        (Some(fit), Some(main)) if fit < main
                    ),
                );
            }
            records.push(Value::Object(record));
        }
        let task = TaskId::try_from(TASK.to_owned())
            .expect("invariant: the content-mode task's ID is one");
        let answer = site.proving_ground.judge(&Submission {
            task: &task,
            status: "success",
            records: &records,
        });
        checks.check(
            format!(
                "the oracle verifies every form of every page ({:?})",
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

/// On `clean-up.test`, `html.raw` is what `content()` returns, its shadow
/// roots left out as `content()` leaves them; on `/hiding`, the ad a
/// cosmetic rule hides is in `html.raw` and neither in `html.cleaned` nor
/// in the Markdown (ADR 0047).
pub(in crate::protocol::cases) fn raw_html_is_content_before_the_rules(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-formats-raw-html";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        for path in ["/hiding", "/shadow"] {
            let (url, answer) = clean_up::visit(connection, site, &page, path)?;
            answered(&answer, &format!("the `goto` to {path}"), &url, 200, checks);
            let raw = form(
                connection,
                &page.page_id,
                &json!({ "html": { "kind": "raw" } }),
                "html",
            )?;
            let content = call(
                connection,
                PAGE_EVALUATE.name,
                &json!(page.page_id),
                &json!({ "function": CONTENT }),
            )?;
            let content = content["result"]["value"].as_str().map(ToOwned::to_owned);
            checks.check(
                format!("on {path}, `html.raw` is what `content()` returns"),
                raw.is_some() && raw == content,
            );
            if path != "/hiding" {
                continue;
            }
            let cleaned = form(
                connection,
                &page.page_id,
                &json!({ "html": { "kind": "cleaned" } }),
                "html",
            )?
            .unwrap_or_default();
            let markdown = form(
                connection,
                &page.page_id,
                &json!({ "markdown": true }),
                "markdown",
            )?
            .unwrap_or_default();
            let raw = raw.unwrap_or_default();
            checks.check(
                "`html.raw` holds the ad a cosmetic rule hides",
                raw.contains(clean_up::AD_TEXT),
            );
            checks.check(
                "`html.cleaned` holds the article and not the ad, so it differs from `html.raw`",
                cleaned.contains(clean_up::ARTICLE_TEXT)
                    && !cleaned.contains(clean_up::AD_TEXT)
                    && cleaned != raw,
            );
            checks.check(
                "the Markdown holds the article and not the ad",
                markdown.contains(clean_up::ARTICLE_TEXT) && !markdown.contains(clean_up::AD_TEXT),
            );
        }
    }
    finish_run(connection, run_id)
}

/// `markdown: true` holds the main content, as `content: "main"` does, and
/// `html: true` the main content's HTML, as `kind: "main"` does; an
/// option, a content or a kind the request does not know is refused with
/// `invalid_params`, as any invalid parameter is.
pub(in crate::protocol::cases) fn defaults_and_unknown_options(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, _)) = task_pages(site, TASK, checks) else {
        return Ok(());
    };
    let url = format!("{}{NAVIGATION_PAGE}", origin_of(&index));
    let run_id = "conformance-formats-defaults";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        goto(connection, &page.page_id, &url, checks)?;
        for (asked, named, key) in [
            (
                json!({ "markdown": true }),
                json!({ "markdown": { "content": "main" } }),
                "markdown",
            ),
            (
                json!({ "html": true }),
                json!({ "html": { "kind": "main" } }),
                "html",
            ),
        ] {
            let default = form(connection, &page.page_id, &asked, key)?;
            let main = form(connection, &page.page_id, &named, key)?;
            checks.check(
                format!("`{asked}` answers what `{named}` does"),
                default.is_some() && default == main,
            );
        }
        for unknown in [
            json!({ "markdown": { "content": "everything" } }),
            json!({ "html": { "kind": "pretty" } }),
            json!({ "html": { "kind": "raw", "pretty": true } }),
            json!({ "text": { "content": "full" } }),
            json!({ "screenshot": true }),
        ] {
            let refused = call(
                connection,
                PAGE_FORMATS.name,
                &json!(page.page_id),
                &json!({ "formats": unknown }),
            )?;
            checks.check(
                format!(
                    "`{unknown}` is refused, not answered ({})",
                    refused["error"]
                ),
                refused.get("result").is_none(),
            );
            refused_as(&refused, ProtocolFailure::InvalidParams, checks);
        }
    }
    finish_run(connection, run_id)
}
