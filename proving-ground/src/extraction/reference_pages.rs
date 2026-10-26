//! The family's reference pages (spec #580, ticket #786; pitfall U08):
//! pages whose links, images, media and tables a program reads, each with
//! what each format holds as JSON (`pages/references.toml`), served under
//! `/references/` and listed by `/references`; a page that moves answers
//! `302` to where it is served, so its relative URLs resolve against the
//! URL after the redirect. One more page, not listed, changes its link
//! while a program reads it.

use std::fmt::Write as _;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::task::Record;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pages {
    pages: Vec<Page>,
}

/// One page: its path under `/references/`, where it moves, its HTML and
/// each format as JSON with `{origin}` for the task's origin.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    path: String,
    moves_to: Option<String>,
    html: String,
    links: String,
    images: String,
    media: String,
    tables: String,
}

static PAGES: LazyLock<Pages> = LazyLock::new(|| {
    toml::from_str(include_str!("../../pages/references.toml"))
        .expect("invariant: the committed reference pages parse, which the tests check")
});

/// The path of the index that lists every page.
pub(super) const INDEX: &str = "/references";

/// The path of the page whose link changes while it is read.
pub(super) const CHANGING: &str = "/references/changing";

/// A page whose script rewrites its one link, href and text together,
/// every millisecond, counting up from 1.
const CHANGING_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>A changing page | Northwind</title></head>
<body>
<main>
<article>
<h1>Today's kite</h1>
<p>Each moment the shop shows another kite of its catalogue, and this page follows it as it goes, so whatever reads the page twice never reads the same kite.</p>
<p>The kite of the moment is <a id="kite" href="/kites/0">kite 0</a>, until the next one replaces it a moment later.</p>
</article>
</main>
<script>
let shown = 0;
setInterval(() => {
  shown += 1;
  const kite = document.getElementById("kite");
  kite.href = `/kites/${shown}`;
  kite.textContent = `kite ${shown}`;
  document.body.dataset.shown = String(shown);
}, 1);
</script>
</body>
</html>
"#;

/// The page at `path`: the index, the changing page or one page's HTML,
/// served where it moves to if it moves.
pub(super) fn page(path: &str) -> Option<String> {
    if path == INDEX {
        return Some(index());
    }
    if path == CHANGING {
        return Some(CHANGING_PAGE.to_owned());
    }
    let path = path.strip_prefix(INDEX)?.strip_prefix('/')?;
    PAGES
        .pages
        .iter()
        .find(|page| page.moves_to.as_deref().unwrap_or(&page.path) == path)
        .map(|page| page.html.clone())
}

/// Where the page at `path` moves, a path on the same host, if it moves.
pub(super) fn moved(path: &str) -> Option<String> {
    let path = path.strip_prefix(INDEX)?.strip_prefix('/')?;
    PAGES
        .pages
        .iter()
        .find(|page| page.path == path)
        .and_then(|page| page.moves_to.as_ref())
        .map(|to| format!("{INDEX}/{to}"))
}

fn index() -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Reference pages</title>\n</head>\n<body>\n<ul>\n",
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
                ("links", json(&page.links)),
                ("images", json(&page.images)),
                ("media", json(&page.media)),
                ("tables", json(&page.tables)),
            ])
        })
        .collect()
}
