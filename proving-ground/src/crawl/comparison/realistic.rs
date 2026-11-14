//! The realistic-page site (#2316) on `realistic.localhost`: the `scale`
//! family's tree of pages, each page an article of tens of kilobytes with
//! 30 to 100 links, as a real site's pages are (crawlee-rs publishes its
//! figures on pages of 58 KB), where the tree's own pages are a few
//! hundred bytes with eleven links.
//!
//! A page links, as absolute paths on its host:
//!
//! - home and the first ten pages, three times: in its header's site bar
//!   and menu, and in its footer, so most of its links repeat;
//! - its ancestors, as a breadcrumb;
//! - its siblings, its parent's children;
//! - its own children twice: a list and related teasers.
//!
//! Every page it links but its children was linked before it was, by its
//! parent or an ancestor, so each page is still first found by its parent.
//! A page whose number ends in 7 is linked only through its old path,
//! `/sites/<size>/moved/<n>`, which answers `301` to `/sites/<size>/<n>`:
//! a crawl reaches it through one redirect.

use std::fmt::Write as _;

use hyper::StatusCode;

use super::super::scale::{children, item_name, site_place};
use super::super::{Page, document};
use crate::generator;

/// The site's host.
pub(super) const HOST: &str = "realistic.localhost";

/// The generator place of the site's values.
const PLACE: u64 = 26;

/// The fewest and most paragraphs of a page's article.
const PARAGRAPHS: (u64, u64) = (30, 90);

/// The sentences a paragraph is drawn from, here and in the article site's
/// pages.
pub(super) const SENTENCES: [&str; 16] = [
    "The workshop has made this piece by hand since the spring, in small batches that sell out within the week.",
    "Each one is checked twice before it leaves the bench, once for its finish and once for the fit of its parts.",
    "Customers who bought it last season wrote that it still looks new after months of daily use in a busy kitchen.",
    "The wood comes from a sawmill two valleys away, dried slowly for a year so that it neither warps nor splits.",
    "Shipping takes three to five working days, and every parcel is packed in paper rather than plastic foam.",
    "A spare set of fittings comes in the box, with a short guide that shows how to tighten them without tools.",
    "The colour deepens with sunlight over the first few weeks, which the makers consider part of its character.",
    "Returns are accepted for thirty days, and the repair service keeps parts for every model made in ten years.",
    "Our editors tested it beside three similar pieces and kept coming back to it for its weight and balance.",
    "The design started as a sketch on the back of an order form and went through eleven prototypes in a year.",
    "Cleaning it needs nothing more than a damp cloth; harsh detergents dull the oil that protects its surface.",
    "Several readers asked whether it suits a small flat, and the answer is yes: it folds flat against a wall.",
    "The limited edition carries a numbered brass plate, engraved in the same workshop that cuts the joints.",
    "Its measurements are listed below, along with the weight of the box, for anyone planning a delivery.",
    "A matching set is in the works, and subscribers to the newsletter will hear about it before anyone else.",
    "Photographs on this page were taken in natural light, without retouching, so the grain shows as it is.",
];

/// The path a page of the site of `size` pages is linked by: its old path
/// for a page whose number ends in 7, its own otherwise.
fn href(size: u64, page: u64) -> String {
    if page % 10 == 7 {
        format!("/sites/{size}/moved/{page}")
    } else {
        format!("/sites/{size}/{page}")
    }
}

/// The parent of `page` in the tree, none for the first page.
fn parent(page: u64) -> Option<u64> {
    (page > 1).then(|| (page - 2) / 10 + 1)
}

pub(super) fn respond(seed: u64, path: &str) -> Page {
    let Some(rest) = path.strip_prefix("/sites/") else {
        return Page::not_found();
    };
    if let Some((size, page)) = rest.split_once("/moved/") {
        return match site_place(&format!("{size}/{page}")) {
            Some((size, page)) => Page::redirect(
                StatusCode::MOVED_PERMANENTLY,
                &format!("/sites/{size}/{page}"),
            ),
            None => Page::not_found(),
        };
    }
    match site_place(rest) {
        Some((size, page)) => Page::ok(page_of(seed, size, page)),
        None => Page::not_found(),
    }
}

/// Writes one link, a list item, to `body`.
fn link(body: &mut String, seed: u64, size: u64, page: u64) {
    writeln!(
        body,
        "<li><a href=\"{}\">{}</a></li>",
        href(size, page),
        item_name(seed, page)
    )
    .expect("invariant: writing to a String succeeds");
}

/// The menu of home and the first ten pages, as `class` names it.
fn menu(body: &mut String, seed: u64, size: u64, class: &str) {
    writeln!(body, "<nav class=\"{class}\">\n<ul>")
        .expect("invariant: writing to a String succeeds");
    for page in 1..=11.min(size) {
        link(body, seed, size, page);
    }
    body.push_str("</ul>\n</nav>\n");
}

/// The page `page` of the site of `size` pages.
fn page_of(seed: u64, size: u64, page: u64) -> String {
    let name = item_name(seed, page);
    let mut body = String::with_capacity(64 * 1024);
    body.push_str("<header>\n");
    menu(&mut body, seed, size, "site");
    menu(&mut body, seed, size, "menu");
    body.push_str("</header>\n");
    let mut ancestors = Vec::new();
    let mut at = parent(page);
    while let Some(ancestor) = at {
        ancestors.push(ancestor);
        at = parent(ancestor);
    }
    body.push_str("<nav class=\"breadcrumb\">\n<ul>\n");
    for ancestor in ancestors.iter().rev() {
        link(&mut body, seed, size, *ancestor);
    }
    body.push_str("</ul>\n</nav>\n");
    writeln!(
        body,
        "<main>\n<article class=\"item\" data-id=\"S-{page:07}\">\n<h1 class=\"name\">{name}</h1>"
    )
    .expect("invariant: writing to a String succeeds");
    let paragraphs =
        PARAGRAPHS.0 + generator::below(seed, &[PLACE, page], PARAGRAPHS.1 - PARAGRAPHS.0 + 1);
    for paragraph in 0..paragraphs {
        let drawn = generator::draw(seed, &[PLACE, page, paragraph]);
        body.push_str("<p>");
        // Six sentences, each four bits of the draw.
        for sentence in 0..6 {
            let index = usize::try_from((drawn >> (4 * sentence)) & 0xf)
                .expect("invariant: four bits fit a usize");
            body.push_str(SENTENCES[index]);
            body.push(' ');
        }
        body.push_str("</p>\n");
    }
    body.push_str("<ul class=\"pages\">\n");
    for child in children(size, page) {
        link(&mut body, seed, size, child);
    }
    body.push_str("</ul>\n</article>\n<aside class=\"related\">\n<ul>\n");
    for child in children(size, page) {
        link(&mut body, seed, size, child);
    }
    body.push_str("</ul>\n</aside>\n<aside class=\"siblings\">\n<ul>\n");
    if let Some(parent) = parent(page) {
        for sibling in children(size, parent) {
            link(&mut body, seed, size, sibling);
        }
    }
    body.push_str("</ul>\n</aside>\n</main>\n<footer>\n");
    menu(&mut body, seed, size, "footer");
    body.push_str("</footer>\n");
    document(&name, &body)
}
