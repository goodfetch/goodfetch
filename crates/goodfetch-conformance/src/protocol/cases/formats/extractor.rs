//! The main-content step `content.extractor` chooses (spec #580, ticket
//! #788): on the `extraction` family's page with a figure, dom-smoothie's
//! main content holds the figure's image and rs-trafilatura's does not, so
//! the Markdown tells which step read it; the setting set on a Run, on a
//! Session's own settings and on a page's own, each nearer scope over the
//! one above (ADR 0083); and a page's own settings refusing an extractor no
//! built-in has and any key but this one.

use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::settings::{SETTING_INVALID, SETTING_NOT_APPLICABLE};
use serde_json::{Value, json};

use super::super::{Checks, Site, finish_run, start_run};
use super::{formats, goto, origin_of, task_pages};
use crate::protocol::connection::{Connection, Gone};

/// The content-mode task, whose navigation page holds a figure.
const TASK: &str = "extraction/8";

/// Its page with a figure between two paragraphs.
const FIGURE_PAGE: &str = "/content-modes/navigation-and-related";

/// The figure's image, as the Markdown writes it.
const IMAGE: &str = "/images/line.jpg";

/// A paragraph of the page's article, which every step keeps.
const PARAGRAPH: &str = "Braided polyester is the usual choice for single-line kites";

/// One `content.extractor` case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    ChosenAtEachScope,
    TrafilaturaMarkdownMatchesTheOracle,
    TrafilaturaEveryFormMatchesTheOracle,
    PageSettingsRefusals,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(in crate::protocol) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "content-extractor-chooses-the-main-content-at-the-run-session-and-page-scopes" => {
                Self::ChosenAtEachScope
            }
            "with-trafilatura-markdown-matches-the-oracle-on-the-markdown-pages" => {
                Self::TrafilaturaMarkdownMatchesTheOracle
            }
            "with-trafilatura-each-markdown-content-html-kind-and-the-text-match-the-oracle" => {
                Self::TrafilaturaEveryFormMatchesTheOracle
            }
            "a-pages-own-settings-refuse-an-unknown-extractor-and-any-other-key" => {
                Self::PageSettingsRefusals
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
            Self::ChosenAtEachScope => chosen_at_each_scope(connection, site, checks),
            Self::TrafilaturaMarkdownMatchesTheOracle => {
                super::markdown_matches_the_oracle(connection, site, Some("trafilatura"), checks)
            }
            Self::TrafilaturaEveryFormMatchesTheOracle => {
                super::content_modes::every_form_matches_the_oracle(
                    connection,
                    site,
                    Some("trafilatura"),
                    checks,
                )
            }
            Self::PageSettingsRefusals => page_settings_refusals(connection, site, checks),
        }
    }
}

/// A Run's settings that choose `extractor`, or the default when none.
pub(in crate::protocol::cases) fn run_settings(extractor: Option<&str>) -> Value {
    extractor.map_or_else(
        || json!({}),
        |name| json!({ "content": { "extractor": name } }),
    )
}

/// `content.extractor` set to `name` in a scope's own settings.
fn choosing(name: &str) -> Value {
    json!({ "content": { "extractor": name } })
}

/// The page an answer of `run.new_page` or `session.new_page` names.
fn page_of(answer: &Value, checks: &mut Checks, what: &str) -> Option<PageOpened> {
    let page = serde_json::from_value(answer["result"].clone()).ok();
    checks.check(format!("{what} opens a page ({answer})"), page.is_some());
    page
}

/// Which step read the figure page on `page`: `dom_smoothie` when its
/// Markdown holds the figure's image, `trafilatura` when it holds the
/// article without it; none when it holds neither.
fn step_of(
    connection: &mut Connection,
    page: &PageOpened,
    url: &str,
    checks: &mut Checks,
) -> Result<Option<&'static str>, Gone> {
    goto(connection, &page.page_id, url, checks)?;
    let markdown = formats(connection, &page.page_id, &json!({ "markdown": true }))?
        .and_then(|result| result["markdown"].as_str().map(ToOwned::to_owned))
        .unwrap_or_default();
    Ok(
        match (markdown.contains(PARAGRAPH), markdown.contains(IMAGE)) {
            (true, true) => Some("dom_smoothie"),
            (true, false) => Some("trafilatura"),
            (false, _) => None,
        },
    )
}

/// The figure page's URL on the Proving ground.
fn figure_page(site: &Site, checks: &mut Checks) -> Option<String> {
    let (index, paths) = task_pages(site, TASK, checks)?;
    checks.check(
        format!("task `{TASK}` lists {FIGURE_PAGE}"),
        paths.iter().any(|path| path == FIGURE_PAGE),
    );
    Some(format!("{}{FIGURE_PAGE}", origin_of(&index)))
}

/// With no setting, the figure page is read by dom-smoothie; a Run's
/// `content.extractor: trafilatura` reads it with rs-trafilatura; a
/// page's own `dom_smoothie` reads it with dom-smoothie under that Run; a
/// Session's own `dom_smoothie` reads its pages with dom-smoothie, and a
/// page of that Session with its own `trafilatura` with rs-trafilatura.
fn chosen_at_each_scope(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some(url) = figure_page(site, checks) else {
        return Ok(());
    };
    let run_id = "conformance-formats-extractor-default";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let answer = connection.call(RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    if let Some(page) = page_of(&answer, checks, "`run.new_page`") {
        let step = step_of(connection, &page, &url, checks)?;
        checks.check(
            format!("with no setting, dom-smoothie reads the page ({step:?})"),
            step == Some("dom_smoothie"),
        );
    }
    finish_run(connection, run_id)?;

    let run_id = "conformance-formats-extractor-scopes";
    start_run(
        connection,
        site,
        run_id,
        &run_settings(Some("trafilatura")),
        checks,
    )?;
    let answer = connection.call(RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    if let Some(page) = page_of(&answer, checks, "`run.new_page`") {
        let step = step_of(connection, &page, &url, checks)?;
        checks.check(
            format!("the Run's `trafilatura` reads its page with rs-trafilatura ({step:?})"),
            step == Some("trafilatura"),
        );
    }
    let answer = connection.call(
        RUN_NEW_PAGE.name,
        &json!({ "run_id": run_id, "settings": choosing("dom_smoothie") }),
    )?;
    if let Some(page) = page_of(&answer, checks, "`run.new_page` with its own settings") {
        let step = step_of(connection, &page, &url, checks)?;
        checks.check(
            format!("a page's own `dom_smoothie` wins over its Run's ({step:?})"),
            step == Some("dom_smoothie"),
        );
    }
    let opened = connection.call(
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": choosing("dom_smoothie") }),
    )?;
    let session = opened["result"]["session_id"].clone();
    checks.check(
        format!(
            "`{}` with its own settings opens a Session",
            RUN_NEW_SESSION.name
        ),
        session.is_string(),
    );
    if session.is_string() {
        let answer = connection.call(SESSION_NEW_PAGE.name, &json!({ "session_id": session }))?;
        if let Some(page) = page_of(&answer, checks, "`session.new_page`") {
            let step = step_of(connection, &page, &url, checks)?;
            checks.check(
                format!("a Session's own `dom_smoothie` wins over its Run's ({step:?})"),
                step == Some("dom_smoothie"),
            );
        }
        let answer = connection.call(
            SESSION_NEW_PAGE.name,
            &json!({ "session_id": session, "settings": choosing("trafilatura") }),
        )?;
        if let Some(page) = page_of(&answer, checks, "`session.new_page` with its own settings") {
            let step = step_of(connection, &page, &url, checks)?;
            checks.check(
                format!("a page's own `trafilatura` wins over its Session's ({step:?})"),
                step == Some("trafilatura"),
            );
        }
    }
    finish_run(connection, run_id)
}

/// A page's own `content.extractor` naming no built-in fails with
/// `setting_invalid` naming the setting, and any other key in a page's own
/// settings with `setting_not_applicable` naming it; neither opens a page.
fn page_settings_refusals(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-formats-extractor-refusals";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let unknown = connection.call(
        RUN_NEW_PAGE.name,
        &json!({ "run_id": run_id, "settings": choosing("readability") }),
    )?;
    checks.check(
        format!(
            "an unknown extractor fails with `{}` naming `content.extractor` ({unknown})",
            SETTING_INVALID.name
        ),
        unknown["error"]["data"]["code"] == json!(SETTING_INVALID.name)
            && unknown["error"]["data"]["key"] == json!("content.extractor"),
    );
    let other = connection.call(
        RUN_NEW_PAGE.name,
        &json!({ "run_id": run_id, "settings": { "filter_lists": { "ads": false } } }),
    )?;
    checks.check(
        format!(
            "another key fails with `{}` naming it ({other})",
            SETTING_NOT_APPLICABLE.name
        ),
        other["error"]["data"]["code"] == json!(SETTING_NOT_APPLICABLE.name)
            && other["error"]["data"]["key"] == json!("filter_lists.ads"),
    );
    finish_run(connection, run_id)
}
