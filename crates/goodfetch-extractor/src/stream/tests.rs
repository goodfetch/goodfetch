//! The streaming path against the tree: on every document of its corpus,
//! a pass either reads each selector's elements as the parsed tree holds
//! them, with the same text and the same elements holding them, or
//! answers nothing; and it answers for the documents whose tree it can
//! vouch for.

use goodfetch_counted_work::{CountedWork, Row};
use rstest::rstest;

use super::{Held, Query, StreamSelector, Streamed, Streaming};

/// What the tree holds of each element `selector` matches in `html`, as a
/// pass reads it.
fn on_the_tree(html: &str, selector: &str) -> Vec<Streamed> {
    let work = CountedWork::default();
    let document = work.parse_html(Row::PageParses, html);
    let matcher = work
        .compile_selector(Row::PageSelectorCompilations, selector)
        .expect("a selector of the test is CSS");
    let held = |node: &dom_query::NodeRef<'_>| Held {
        name: node
            .node_name()
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default(),
        attributes: node
            .attrs()
            .iter()
            .map(|attribute| {
                (
                    attribute.name.local.to_string(),
                    attribute.value.to_string(),
                )
            })
            .collect(),
    };
    document
        .select_matcher(&matcher)
        .nodes()
        .iter()
        .map(|node| Streamed {
            held: std::iter::once(*node)
                .chain(node.ancestors_it(None))
                .filter(dom_query::NodeRef::is_element)
                .map(|element| held(&element))
                .collect(),
            text: node.text().to_string(),
        })
        .collect()
}

/// What a pass reads of `html` for `selector` alone.
fn streamed(html: &str, selector: &str) -> Option<Vec<Streamed>> {
    let work = CountedWork::default();
    let streaming = Streaming::new(&work);
    let selector = StreamSelector::of(selector, &work).expect("lol_html reads the test's selector");
    let read = streaming.read(
        html,
        &[Query {
            selector: &selector,
            first: false,
        }],
    );
    assert_eq!(work.read(Row::PageParses), 0, "a pass parses no tree");
    read.map(|mut read| read.remove(0))
}

/// A well-formed page: what most of the corpus is.
fn page(head: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=en><head><meta charset=utf-8>{head}</head>\n<body>{body}</body></html>\n"
    )
}

const SELECTORS: [&str; 10] = [
    "*",
    "a[href]",
    "h1",
    "[aria-controls]",
    "meta[name]",
    "body",
    "li:nth-child(2) a",
    ".nav a",
    "a[href^=\"/p\"]",
    "title",
];

/// The extractor's own fixtures, its fuzz seeds and the documents whose
/// tree differs from a stack of start and end tags.
fn corpus() -> Vec<(String, bool)> {
    let streams = |html: String| (html, true);
    let refused = |html: String| (html, false);
    vec![
        streams(include_str!("../formats/fixtures/article.html").to_owned()),
        streams(include_str!("../formats/fixtures/heading-with-break.html").to_owned()),
        streams(include_str!("../formats/fixtures/navigation-and-related.html").to_owned()),
        streams(include_str!("../formats/fixtures/no-main-content.html").to_owned()),
        streams(include_str!("../formats/fixtures/pre-without-code.html").to_owned()),
        streams(include_str!("../formats/fixtures/row-headers.html").to_owned()),
        refused(include_str!("../../fuzz/formats/seeds/nested-table-and-fences.html").to_owned()),
        refused(include_str!("../../fuzz/formats/seeds/references-and-tables.html").to_owned()),
        // B1's pages.
        streams(page(
            "<title>Page 1</title>",
            "<h1>Page 1</h1>\n<nav><ul><li><a href=\"/\">Index</a></li><li><a href=\"/page-2\">Page 2</a></li></ul></nav>\n<main>\n<p>Paragraph 000: the crawler reads.</p>\n</main>",
        )),
        // Markup that hides a link, revealed by a control after it.
        streams(page(
            "<title>Hidden</title>",
            "<div id=more hidden><a href=/p/more class=nav>More</a></div><p style=\"display:none\"><a href=/p/gone>Gone</a></p><button aria-controls=\"more other\">Show</button>",
        )),
        // Character references, CR LF and duplicate attributes.
        streams(page(
            "<title>Fish &amp; chips</title>",
            "<a HREF=\"/a?b=1&amp;c=2\" title=\"&quot;x&quot;\" href=/second>Fish &amp; chips &copy &notanentity; &#x41;\r\nend</a><A href=/upper>UPPER</A>",
        )),
        // A selector reading `href`'s value is not matched on one undecoded.
        streams(page("", "<ul class=nav><li><a href=/p/1>One</a></li><li><a href=/p/2>Two</a></li></ul>")),
        // SVG inside a link, and white space after `</body>`.
        streams(
            "<!doctype html><html><head></head><body><a href=/home><svg viewBox=\"0 0 1 1\"><title>Icon</title><path d=\"M0\"/></svg>Home</a></body>\n\n</html>\n".to_owned(),
        ),
        // `<pre>`'s first line feed, a `<textarea>`'s, and text that looks
        // like markup in a script, a comment and an attribute.
        streams(page(
            "<script>var a = '</div>'; if (a < 1) {}</script><!-- </p> -->",
            "<pre>\n<a href=/p/pre>pre</a>\nmore</pre><textarea>\n</a> kept</textarea><a href=/x title=\"</b>\">x</a>",
        )),
        // A table with its sections, a nested list and a description list.
        streams(page(
            "",
            "<table><tbody><tr><td><a href=/p/cell>cell</a></td><th>head</th></tr></tbody></table><ul><li>a<ul><li><a href=/p/inner>inner</a></li></ul></li></ul><dl><dt>t</dt><dd><a href=/d>d</a></dd></dl>",
        )),
        // A `<p>` inside a `<button>` inside a `<p>`, and `<div/>`.
        streams(page(
            "",
            "<p><button><p><a href=/b>b</a></p></button></p><div/><a href=/in-div>in div</a>",
        )),
        // `</html>` ends the body.
        streams("<!doctype html><html><head></head><body><a href=/end>end</a></html>".to_owned()),
        // A `<p>` or `<li>` the next start tag closes.
        refused(page("", "<p>a<p>b <a href=/x>y</a>")),
        refused(page("", "<ul><li>a<li><a href=/x>b</a></ul>")),
        refused(page("", "<dl><dt>a<dd><a href=/x>b</a></dl>")),
        refused(page("", "<h1>a<h2><a href=/x>b</a></h2></h1>")),
        refused(page("", "<p>a<div><a href=/x>b</a></div>")),
        // A second `<a>` inside one, and an end tag that closes two.
        refused(page("", "<a href=/1>one<a href=/2>two</a>")),
        refused(page("", "<p><a href=/1>x</p>y</a>")),
        // A table's implied sections and its content moved before it.
        refused(page("", "<table><tr><td><a href=/x>x</a></td></tr></table>")),
        refused(page("", "<table><a href=/x>x</a><tbody></tbody></table>")),
        refused(page("", "<table><tbody>text<tr></tr></tbody></table>")),
        // End tags the tree reads as elements, or ignores.
        refused(page("", "<a href=/x>x</a></p>")),
        refused(page("", "<br></br><a href=/x>x</a>")),
        refused(page("", "<div><a href=/x>x</a></span></div>")),
        // What lol-html parses otherwise than the tree.
        refused(page("", "<noscript><a href=/x>x</a></noscript>")),
        refused(page("", "<template><a href=/x>x</a></template>")),
        refused(page("", "<select><option>a<a href=/x>x</a></select>")),
        // Quirks mode, an implied `<head>` or `<body>`, text in the head.
        refused("<html><head></head><body><a href=/x>x</a></body></html>".to_owned()),
        refused("<!doctype html><a href=/x>x</a>".to_owned()),
        refused("<!doctype html><html><head><title>t</title><body><a href=/x>x</a></body></html>".to_owned()),
        refused("<!doctype html><html><head>text</head><body></body></html>".to_owned()),
        // An element after `</body>`, and HTML or a link inside SVG.
        refused("<!doctype html><html><head></head><body></body><a href=/x>x</a></html>".to_owned()),
        refused(page("", "<svg><a href=/x>x</a></svg>")),
        refused(page("", "<svg><p>x</p></svg>")),
        refused(page("", "<a href=/x>\0</a>")),
        // A value a selector compares, undecoded in lol-html: see
        // `a_value_a_selector_compares_is_read_decoded`.
        streams(UNDECODED_CLASS.to_owned()),
    ]
}

/// A page whose class names a character reference.
const UNDECODED_CLASS: &str = "<!doctype html><html><head></head><body><ul class=\"nav&#32;x\"><li><a href=/p/1>One</a></li><li><a href=/p/2>Two</a></li></ul></body></html>";

/// A selector comparing an attribute's value is not matched on a value
/// lol-html reads undecoded: the pass answers nothing.
#[test]
fn a_value_a_selector_compares_is_read_decoded() {
    assert_eq!(streamed(UNDECODED_CLASS, ".nav a"), None);
    assert_eq!(on_the_tree(UNDECODED_CLASS, ".nav a").len(), 2);
    assert!(streamed(UNDECODED_CLASS, "a[href]").is_some());
}

/// Each selector of each document reads what the tree holds, or nothing.
#[rstest]
fn a_pass_reads_what_the_tree_holds_or_nothing(#[values(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)] at: usize) {
    let selector = SELECTORS[at];
    for (html, _) in corpus() {
        if let Some(read) = streamed(&html, selector) {
            assert_eq!(read, on_the_tree(&html, selector), "{selector} on {html}");
        }
    }
}

/// A pass answers for a well-formed document, and answers nothing for one
/// whose tree it cannot vouch for.
#[test]
fn a_pass_answers_for_the_documents_whose_tree_it_vouches_for() {
    for (html, streams) in corpus() {
        assert_eq!(streamed(&html, "a[href]").is_some(), streams, "{html}");
    }
}

/// Several queries read in one pass what each reads alone, the first only
/// for a query asking for it, with no selector parsed for the pass.
#[test]
fn one_pass_reads_every_query() {
    let html = page(
        "<base href=/kites/><meta name=robots content=noindex>",
        "<h1>One</h1><h1>Two</h1><a href=/p>p</a>",
    );
    let work = CountedWork::default();
    let streaming = Streaming::new(&work);
    let selectors: Vec<StreamSelector> = ["h1", "a[href]", "base[href]"]
        .iter()
        .map(|selector| StreamSelector::of(selector, &work).unwrap())
        .collect();
    let parsed = work.read(Row::PageSelectorCompilations);
    let queries: Vec<Query<'_>> = selectors
        .iter()
        .zip([true, false, true])
        .map(|(selector, first)| Query { selector, first })
        .collect();
    let read = streaming.read(&html, &queries).unwrap();
    assert_eq!(work.read(Row::PageSelectorCompilations), parsed);
    assert_eq!(read[0], on_the_tree(&html, "h1")[..1]);
    assert_eq!(read[1], on_the_tree(&html, "a[href]"));
    assert_eq!(read[2], on_the_tree(&html, "base[href]"));
}

/// A selector lol-html does not read, such as a sibling combinator, has
/// no streaming selector, so its reader reads the tree.
#[test]
fn a_selector_lol_html_does_not_read_has_none() {
    let work = CountedWork::default();
    assert!(StreamSelector::of("h1 + p", &work).is_none());
    assert!(StreamSelector::of("a[href]", &work).is_some());
}
