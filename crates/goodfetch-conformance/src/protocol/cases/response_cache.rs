//! The page suite's Response cache cases (ADR 0046, ticket #796): the
//! browser tier asks the Run's Response cache at step 5 of a page's
//! pipeline, so a later Run's page takes from it what an earlier Run's
//! page stored, with no request to the site. The page is the Proving
//! ground's `cache.test` page `/page/<name>`, never stored itself, which
//! loads the script `/asset/<name>.js` and frames `/fresh/<name>`, both
//! kept an hour; the script writes the number of its answer on the
//! document, so the page tells which answer it ran, and the Proving
//! ground counts the requests the site saw. The service worker line of the
//! same ticket is a service worker case.

use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use serde_json::{Value, json};

use super::fetch_tiers::cached::HOST;
use super::navigation::{answered, call};
use super::routes::received;
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};

/// One Response cache case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    SubresourcesAreServed,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-later-runs-page-takes-its-script-and-frame-from-the-response-cache" => {
                Self::SubresourcesAreServed
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::SubresourcesAreServed => subresources_are_served(connection, site, checks),
        }
    }
}

/// The settings of these cases' Runs: the Response cache on.
fn settings() -> Value {
    json!({ "cache": { "mode": "revalidate" } })
}

/// Loads `url` on a page of a new Run `run_id` under [`settings`] and
/// finishes the Run: what the page's script wrote, `None` when the page did
/// not open.
fn load(
    connection: &mut Connection,
    site: &Site,
    (run_id, url): (&str, &str),
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    start_run(connection, site, run_id, &settings(), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        finish_run(connection, run_id)?;
        return Ok(None);
    };
    let page_id = json!(page.page_id);
    let loaded = connection.goto(&page, url)?;
    answered(&loaded, &format!("`{run_id}`'s `goto`"), url, 200, checks);
    let written = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({ "function": "() => document.documentElement.dataset.answer ?? null" }),
    )?;
    finish_run(connection, run_id)?;
    Ok(Some(written["result"]["value"].clone()))
}

/// Under `cache.mode: revalidate`, a second Run's load of a page the site
/// never lets be stored reaches the site again, while its script and its
/// frame, kept an hour, come from the cache the first Run's load filled:
/// the site saw one request for each, and the script the second page ran
/// is the site's first answer.
fn subresources_are_served(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let name = "browser-served";
    let page = format!("/page/{name}");
    let url = site.named_url(HOST, &page);
    let first = load(
        connection,
        site,
        ("conformance-response-cache-1", &url),
        checks,
    )?;
    checks.check(
        "the first Run's page ran the script's first answer",
        first == Some(json!("1")),
    );
    let second = load(
        connection,
        site,
        ("conformance-response-cache-2", &url),
        checks,
    )?;
    checks.check(
        "the second Run's page ran the script's first answer, from the cache",
        second == Some(json!("1")),
    );
    checks.check(
        "the site saw the page twice, never stored, and its script and its frame once each",
        (
            received(site, HOST, &page),
            received(site, HOST, &format!("/asset/{name}.js")),
            received(site, HOST, &format!("/fresh/{name}")),
        ) == (2, 1, 1),
    );
    Ok(())
}
