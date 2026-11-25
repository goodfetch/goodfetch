//! The pages of the Memory site the corpus writers' crawls fetch, since no
//! fetch tier reaches a network here: with nothing set, `auto` serves every
//! request on a tier (issue 1602).

use goodfetch_memory_site::{Page, Url};

/// The page at `url`: one small document that names it without its `ref`
/// parameter, so a page reads alike with or without `ref` and unlike any
/// other.
pub(crate) fn page(url: &Url) -> Page {
    let url = url.as_str();
    let named = url.split_once("&ref=").map_or(url, |(named, _)| named);
    Page::html(format!(
        "<!doctype html><html><head><title>{named}</title></head><body><h1>{named}</h1></body></html>"
    ))
}
