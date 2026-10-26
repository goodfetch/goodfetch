//! The family's structured pages (spec #580, ticket #787): pages whose
//! metadata and contacts a program reads, each with what each format holds
//! as JSON (`pages/structured.toml`), served under `/structured/` and
//! listed by `/structured`.

use std::fmt::Write as _;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::task::Record;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pages {
    pages: Vec<Page>,
}

/// One page: its path under `/structured/`, its HTML and each format as
/// JSON with `{origin}` for the task's origin.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    path: String,
    html: String,
    metadata: String,
    contacts: String,
}

static PAGES: LazyLock<Pages> = LazyLock::new(|| {
    toml::from_str(include_str!("../../pages/structured.toml"))
        .expect("invariant: the committed structured pages parse, which the tests check")
});

/// The path of the index that lists every page.
pub(super) const INDEX: &str = "/structured";

/// The page at `path`: the index or one page's HTML.
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
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Structured pages</title>\n</head>\n<body>\n<ul>\n",
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

/// One Record per page: its listed path as `id`, and each format as JSON
/// text, `origin` in place of `{origin}`.
pub(super) fn records(origin: &str) -> Vec<Record> {
    PAGES
        .pages
        .iter()
        .map(|page| {
            let json = |text: &str| text.replace("{origin}", origin).trim().to_owned();
            Record::from([
                ("id", format!("{INDEX}/{}", page.path)),
                ("metadata", json(&page.metadata)),
                ("contacts", json(&page.contacts)),
            ])
        })
        .collect()
}
