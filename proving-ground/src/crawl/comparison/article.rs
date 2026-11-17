//! The article site (#2136) on `article.localhost`, for the comparison's
//! extraction scenario: the `scale` family's tree of pages, with the same
//! links, each page an article whose right answer the Proving ground holds,
//! so every tool's title and Main content can be checked against it.
//!
//! A page holds, in this order:
//!
//! - a header, with the header's marker;
//! - a nav, with the nav's marker and the tree's links: home and the page's
//!   children;
//! - `<main>`, whose article is a heading and a few paragraphs drawn from
//!   the seed, the first naming the page, with one ad slot after the first
//!   paragraph: a `div.ad-slot`, which EasyList's `##.ad-slot` matches,
//!   holding the ad's marker in a paragraph long enough for a main-content
//!   step to keep;
//! - an aside and a footer, each with its marker.
//!
//! No paragraph holds a marker. A page's title is its name, and on some
//! pages carries a character reference or white space to collapse.
//! [`answers`] gives each page's right answer.

use std::fmt::Write as _;

use super::super::scale::{children, item_name, site_place};
use super::super::{Page, document};
use super::realistic::SENTENCES;
use crate::generator;
use crate::interface::{ArticleAnswer, ArticleAnswers, ArticleLink};

/// The site's host.
pub(super) const HOST: &str = "article.localhost";

/// The generator place of the site's values.
const PLACE: u64 = 35;

/// The fewest and most paragraphs of a page's article.
const PARAGRAPHS: (u64, u64) = (3, 8);

/// The largest site whose answers [`answers`] gives: the answers of a
/// larger one would be hundreds of megabytes.
const ANSWERED_PAGES: u64 = 10_000;

/// The markers of the header, the nav, the aside and the footer: text no
/// paragraph of an article holds.
const HEADER: &str = "Tidewater Supply masthead";
const NAV: &str = "Catalogue index navigation";
const ASIDE: &str = "Related reading sidebar";
const FOOTER: &str = "Tidewater Supply imprint and terms";
/// The ad's marker.
const AD: &str = "Sponsored offer from Harbourline Freight";
/// The text of the nav's link to the site's first page.
const HOME: &str = "Home";

/// The ID page `page`'s article holds in its `data-id`.
fn item_id(page: u64) -> String {
    format!("S-{page:07}")
}

pub(super) fn respond(seed: u64, path: &str) -> Page {
    match path.strip_prefix("/sites/").and_then(site_place) {
        Some((size, page)) => Page::ok(page_of(seed, size, page)),
        None => Page::not_found(),
    }
}

/// A page's title as its markup writes it, and as a reader of the page
/// reads it: character references decoded, white space collapsed.
fn title(seed: u64, page: u64) -> (String, String) {
    let name = item_name(seed, page);
    match page % 8 {
        3 => (format!("{name} &amp; Co."), format!("{name} & Co.")),
        5 => (format!("  {name}   in  stock "), format!("{name} in stock")),
        6 => (
            format!("{name}&nbsp;&mdash; new"),
            format!("{name} \u{2014} new"),
        ),
        _ => (name.clone(), name),
    }
}

/// The paragraphs of a page's article, as a reader reads them: the first
/// names the page, and each of the others is four sentences drawn from the
/// seed.
fn paragraphs(seed: u64, page: u64) -> Vec<String> {
    let count =
        PARAGRAPHS.0 + generator::below(seed, &[PLACE, page], PARAGRAPHS.1 - PARAGRAPHS.0 + 1);
    (0..count)
        .map(|paragraph| {
            let drawn = generator::draw(seed, &[PLACE, page, paragraph]);
            let mut text = if paragraph == 0 {
                format!(
                    "{} is piece number {page} of the catalogue.",
                    item_name(seed, page)
                )
            } else {
                String::new()
            };
            // Four sentences, each four bits of the draw.
            for sentence in 0..4 {
                let index = usize::try_from((drawn >> (4 * sentence)) & 0xf)
                    .expect("invariant: four bits fit a usize");
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(SENTENCES[index]);
            }
            text
        })
        .collect()
}

/// The page `page` of the site of `size` pages.
fn page_of(seed: u64, size: u64, page: u64) -> String {
    let name = item_name(seed, page);
    let mut body = String::with_capacity(8 * 1024);
    let mut line = |text: std::fmt::Arguments<'_>| {
        body.write_fmt(text)
            .expect("invariant: writing to a String succeeds");
        body.push('\n');
    };
    line(format_args!("<header>\n<p>{HEADER}</p>\n</header>"));
    line(format_args!(
        "<nav>\n<p>{NAV}</p>\n<a href=\"/sites/{size}/1\">{HOME}</a>\n<ul class=\"pages\">"
    ));
    for child in children(size, page) {
        line(format_args!(
            "<li><a href=\"/sites/{size}/{child}\">{}</a></li>",
            item_name(seed, child)
        ));
    }
    line(format_args!("</ul>\n</nav>"));
    line(format_args!(
        "<main>\n<article class=\"item\" data-id=\"{}\">\n<h1 class=\"name\">{name}</h1>",
        item_id(page)
    ));
    for (index, paragraph) in paragraphs(seed, page).iter().enumerate() {
        line(format_args!("<p>{paragraph}</p>"));
        if index == 0 {
            line(format_args!(
                "<div class=\"ad-slot\">\n<p>{AD}: {} {}</p>\n</div>",
                SENTENCES[4], SENTENCES[7]
            ));
        }
    }
    line(format_args!("</article>\n</main>"));
    line(format_args!(
        "<aside>\n<p>{ASIDE}: {}</p>\n</aside>",
        SENTENCES[14]
    ));
    line(format_args!("<footer>\n<p>{FOOTER}</p>\n</footer>"));
    document(&title(seed, page).0, &body)
}

/// The right answer of each page of the site of `size` pages, its ID, its
/// name and its nav's links to its children among them, with the nav's
/// home link and the markers no Main content holds; none for a size that is
/// no site's, or past [`ANSWERED_PAGES`].
pub(super) fn answers(seed: u64, size: &str) -> Option<ArticleAnswers> {
    let (size, _) = site_place(&format!("{size}/1"))?;
    (size <= ANSWERED_PAGES).then(|| ArticleAnswers {
        pages: (1..=size)
            .map(|page| ArticleAnswer {
                path: format!("/sites/{size}/{page}"),
                title: title(seed, page).1,
                paragraphs: paragraphs(seed, page),
                id: item_id(page),
                name: item_name(seed, page),
                children: children(size, page)
                    .map(|child| ArticleLink {
                        path: format!("/sites/{size}/{child}"),
                        name: item_name(seed, child),
                    })
                    .collect(),
            })
            .collect(),
        home: ArticleLink {
            path: format!("/sites/{size}/1"),
            name: HOME.to_owned(),
        },
        boilerplate: [HEADER, NAV, ASIDE, FOOTER].map(str::to_owned).to_vec(),
        ad: AD.to_owned(),
    })
}
