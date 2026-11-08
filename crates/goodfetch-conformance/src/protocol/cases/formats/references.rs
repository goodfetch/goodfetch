//! A page's links, images, media and tables (spec #580, ticket #786;
//! pitfall U08's Collie side): on the `extraction` family's reference
//! pages, each format judged by the oracle, links resolved against a
//! `<base href>` and against the URL after a redirect, a URL that does not
//! parse kept with `url: null`; links untouched by the Clean-up rules; and
//! one call's formats read from one serialization of a page that changes
//! while it is read.

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_FUNCTION;
use goodfetch_protocol::page::{PAGE_FORMATS, PageFormats, PageId};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::{Value, json};

use super::super::navigation::answered;
use super::super::{Checks, Site, clean_up, finish_run, new_page, start_run};
use super::{formats, goto, origin_of, task_pages};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::Submission;

/// The task whose pages and oracle judge the references and tables.
const TASK: &str = "extraction/9";

/// The page whose link its script rewrites every millisecond.
const CHANGING: &str = "/references/changing";

/// What every call of the oracle's cases asks for.
const REQUEST: &str = r#"{ "links": true, "images": true, "media": true, "tables": true }"#;

/// One links, images, media and tables case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    ReferencesAndTablesMatchTheOracle,
    LinksAreUntouchedByTheRules,
    OneCallsFormatsComeFromOneSerialization,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(in crate::protocol) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "links-images-media-and-tables-match-the-oracle-resolved-against-the-base-href-and-the-url-after-a-redirect" => {
                Self::ReferencesAndTablesMatchTheOracle
            }
            "an-ads-link-a-cosmetic-rule-hides-is-in-links" => Self::LinksAreUntouchedByTheRules,
            "links-and-markdown-of-one-call-agree-on-a-page-that-changes-between-two-reads" => {
                Self::OneCallsFormatsComeFromOneSerialization
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(in crate::protocol) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::ReferencesAndTablesMatchTheOracle => {
                references_and_tables_match_the_oracle(connection, site, checks)
            }
            Self::LinksAreUntouchedByTheRules => {
                links_are_untouched_by_the_rules(connection, site, checks)
            }
            Self::OneCallsFormatsComeFromOneSerialization => {
                one_calls_formats_come_from_one_serialization(connection, site, checks)
            }
        }
    }
}

/// The formats `request` answers on `page`, as the protocol's type.
fn read(
    connection: &mut Connection,
    page: &PageId,
    request: &Value,
) -> Result<Option<PageFormats>, Gone> {
    Ok(formats(connection, page, request)?
        .and_then(|result| serde_json::from_value::<PageFormats>(result).ok()))
}

/// On each reference page, the links, images, media and tables
/// `page.formats` gives are what the oracle holds for them; on `/base`,
/// links resolve against its `<base href>` and one that does not parse has
/// a null URL, and on `/moved`, against the URL after its redirect.
fn references_and_tables_match_the_oracle(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, paths)) = task_pages(site, TASK, checks) else {
        return Ok(());
    };
    let origin = origin_of(&index).to_owned();
    let request: Value = serde_json::from_str(REQUEST).expect("invariant: the request is JSON");
    let run_id = "conformance-formats-references";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let mut records = Vec::new();
        for path in &paths {
            goto(
                connection,
                &page.page_id,
                &format!("{origin}{path}"),
                checks,
            )?;
            let read = read(connection, &page.page_id, &request)?;
            checks.check(
                format!("`{}` of {path} answers its result", PAGE_FORMATS.name),
                read.is_some(),
            );
            let Some(read) = read else { continue };
            let urls: Vec<Option<&str>> = read
                .links
                .iter()
                .flatten()
                .map(|link| link.url.as_deref())
                .collect();
            if path == "/references/base" {
                checks.check(
                    format!(
                        "on {path}, the base URL is its `<base href>` and a relative link resolves against it ({:?}, {urls:?})",
                        read.base_url
                    ),
                    read.base_url == "https://cdn.northwind.example/assets/"
                        && urls.first()
                            == Some(&Some("https://cdn.northwind.example/assets/reels/geared")),
                );
                checks.check(
                    format!("on {path}, a link whose URL does not parse has `url: null`"),
                    urls.contains(&None),
                );
            }
            if path == "/references/moved" {
                let landed = format!("{origin}/references/landed/shelf/");
                checks.check(
                    format!(
                        "on {path}, the URL is the one after the redirect and a relative link resolves against it ({:?}, {urls:?})",
                        read.url
                    ),
                    read.url == landed
                        && urls.first().copied().flatten()
                            == Some(format!("{landed}book/1").as_str()),
                );
            }
            records.push(json!({
                "id": path,
                "links": read.links,
                "images": read.images,
                "media": read.media,
                "tables": read.tables,
            }));
        }
        let task =
            TaskId::try_from(TASK.to_owned()).expect("invariant: the reference task's ID is one");
        let answer = site.proving_ground.judge(&Submission {
            task: &task,
            status: "success",
            records: &records,
        });
        checks.check(
            format!(
                "the oracle verifies every format of every page ({:?})",
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

/// On `/hiding`, the ad a cosmetic rule takes out of the Markdown keeps
/// its link in `links`: no Clean-up rule touches links (ADR 0047).
fn links_are_untouched_by_the_rules(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-formats-links-rules";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let (url, answer) = clean_up::visit(connection, site, &page, "/hiding")?;
        answered(&answer, "the `goto` to `/hiding`", &url, 200, checks);
        let read = read(
            connection,
            &page.page_id,
            &json!({ "links": true, "markdown": true }),
        )?;
        let offer = format!("{}/hiding/offer", origin_of(&url));
        checks.check(
            format!("the ad's link, {offer}, is in `links` ({read:?})"),
            read.as_ref().is_some_and(|read| {
                read.links
                    .iter()
                    .flatten()
                    .any(|link| link.url.as_deref() == Some(offer.as_str()))
            }),
        );
        checks.check(
            "the Markdown leaves the ad out",
            read.as_ref()
                .and_then(|read| read.markdown.as_deref())
                .is_some_and(|markdown| {
                    markdown.contains(clean_up::ARTICLE_TEXT)
                        && !markdown.contains(clean_up::AD_TEXT)
                }),
        );
    }
    finish_run(connection, run_id)
}

/// The kite a read of the changing page shows: the number its one link's
/// href and text name, when they agree with each other and with the
/// Markdown, which holds that link at its resolved URL.
fn kite_read(read: &PageFormats) -> Option<u64> {
    let [link] = read.links.as_deref()? else {
        return None;
    };
    let shown: u64 = link.href.strip_prefix("/kites/")?.parse().ok()?;
    let markdown = read.markdown.as_deref()?;
    let url = link.url.as_deref()?;
    (link.text == format!("kite {shown}")
        && url.ends_with(&format!("/kites/{shown}"))
        && markdown.contains(&format!("[kite {shown}]({url})")))
    .then_some(shown)
}

/// On a page whose script rewrites its link every millisecond, each call's
/// `links` and `markdown` name the same kite, read from one serialization,
/// and a second call, once the page moved on, names a later one.
fn one_calls_formats_come_from_one_serialization(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some((index, _)) = task_pages(site, TASK, checks) else {
        return Ok(());
    };
    let url = format!("{}{CHANGING}", origin_of(&index));
    let request = json!({ "links": true, "markdown": true });
    let run_id = "conformance-formats-one-serialization";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        goto(connection, &page.page_id, &url, checks)?;
        let first = read(connection, &page.page_id, &request)?;
        let first_kite = first.as_ref().and_then(kite_read);
        checks.check(
            format!(
                "the first call's link and Markdown name the same kite ({first_kite:?}, {first:?})"
            ),
            first_kite.is_some(),
        );
        let Some(first_kite) = first_kite else {
            return finish_run(connection, run_id);
        };
        let moved_on = connection.call(
            PAGE_WAIT_FOR_FUNCTION.name,
            &json!({
                "page_id": page.page_id,
                "function": format!("() => Number(document.body.dataset.shown) > {}", first_kite + 10),
            }),
        )?;
        checks.check(
            "the page moves on to a later kite",
            moved_on.get("result").is_some(),
        );
        let second = read(connection, &page.page_id, &request)?;
        let second_kite = second.as_ref().and_then(kite_read);
        checks.check(
            format!(
                "the second call's link and Markdown name the same kite, a later one ({first_kite} < {second_kite:?}, {second:?})"
            ),
            second_kite.is_some_and(|second| second > first_kite),
        );
    }
    finish_run(connection, run_id)
}
