//! The family's content-mode pages (spec #580): pages read in every form
//! `page.formats` gives, each with what each form holds
//! (`pages/content_modes.toml`), served under `/content-modes/` and listed
//! by `/content-modes`.

use std::fmt::Write as _;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::task::Record;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pages {
    pages: Vec<Page>,
}

/// One page: its path under `/content-modes/`, its HTML and each form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    path: String,
    html: String,
    markdown: String,
    markdown_full: String,
    markdown_fit: String,
    html_main: String,
    text: String,
}

static PAGES: LazyLock<Pages> = LazyLock::new(|| {
    toml::from_str(include_str!("../../pages/content_modes.toml"))
        .expect("invariant: the committed content-mode pages parse, which the tests check")
});

/// The path of the index that lists every page.
pub(super) const INDEX: &str = "/content-modes";

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
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Content-mode pages</title>\n</head>\n<body>\n<ul>\n",
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

/// One Record per page: its path as `id`, and each form. No cosmetic rule
/// matches these pages, so their raw and cleaned HTML are the page's own.
pub(super) fn records() -> Vec<Record> {
    PAGES
        .pages
        .iter()
        .map(|page| {
            Record::from([
                ("id", format!("{INDEX}/{}", page.path)),
                ("markdown", page.markdown.clone()),
                ("markdown_full", page.markdown_full.clone()),
                ("markdown_fit", page.markdown_fit.clone()),
                ("html_raw", page.html.clone()),
                ("html_cleaned", page.html.clone()),
                ("html_main", page.html_main.clone()),
                ("text", page.text.clone()),
            ])
        })
        .collect()
}
