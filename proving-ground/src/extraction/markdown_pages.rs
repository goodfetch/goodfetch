//! The family's Markdown pages (ADR 0007): the lab's 30 hand-written
//! fixtures and a page with no main content, each with the Markdown its
//! main content is (`pages/markdown.toml`), served under `/markdown/` and
//! listed by `/markdown`.

use std::fmt::Write as _;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::task::Record;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pages {
    pages: Vec<Page>,
}

/// One page: its path under `/markdown/`, its HTML and its Markdown.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    path: String,
    html: String,
    markdown: String,
}

static PAGES: LazyLock<Pages> = LazyLock::new(|| {
    toml::from_str(include_str!("../../pages/markdown.toml"))
        .expect("invariant: the committed Markdown pages parse, which the tests check")
});

/// The path of the index that lists every page.
pub(super) const INDEX: &str = "/markdown";

/// The page at `path`: the index, or one page's HTML.
pub(super) fn page(path: &str) -> Option<String> {
    if path == INDEX {
        return Some(index());
    }
    let path = path.strip_prefix(INDEX)?.strip_prefix('/')?;
    PAGES
        .pages
        .iter()
        .find(|page| page.path == path)
        .map(|page| page.html.clone())
}

fn index() -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Markdown pages</title>\n</head>\n<body>\n<ul>\n",
    );
    for page in &PAGES.pages {
        writeln!(
            html,
            "<li><a href=\"{INDEX}/{path}\">{path}</a></li>",
            path = page.path
        )
        .expect("invariant: writing to a String succeeds");
    }
    html.push_str("</ul>\n</body>\n</html>\n");
    html
}

/// One Record per page: its path as `id`, and its Markdown.
pub(super) fn records() -> Vec<Record> {
    PAGES
        .pages
        .iter()
        .map(|page| {
            Record::from([
                ("id", format!("{INDEX}/{}", page.path)),
                ("markdown", page.markdown.clone()),
            ])
        })
        .collect()
}
