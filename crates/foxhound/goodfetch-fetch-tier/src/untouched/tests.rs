//! The readers of the untouched page through their interface: one parse
//! of a document gives what each reader read of its own parse before
//! (spec #1577's #1910), and the Page signature consumes the tree last.

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_protocol::foxhound::crawl::FieldLocator;
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};
use rstest::rstest;
use url::Url;

use super::{UntouchedDemand, UntouchedReaders};
use crate::document::{first_elements, matched};
use crate::expected_page::{self, judge};
use crate::refresh;

const URL: &str = "http://shop.test/kites/delta";

/// A page with a refresh, a `<base>`, a canonical, links one of which its
/// markup hides, fields and a login form.
const PAGE: &str = r#"<!doctype html><html><head><title>Delta</title>
<base href="http://shop.test/kites/">
<meta http-equiv="refresh" content="5; url=delta-2">
<meta name="robots" content="noindex">
<link rel="canonical" href="delta?ref=1">
</head><body><nav><a href="/">Home</a></nav>
<main><h1>Delta</h1><p class="sku">DK-0042</p>
<a href="manual.pdf">Manual</a><a href="trap" style="display:none">Trap</a>
<button aria-controls="more">More</button><div id="more" hidden><a href="more">More</a></div>
<form id="login"></form></main></body></html>"#;

/// A page with none of them.
const BARE: &str = "<p>Nothing <a href=a>here</a>.</p>";

fn css(selector: &str) -> FieldLocator {
    FieldLocator::Css {
        selector: selector.to_owned(),
    }
}

fn demand() -> UntouchedDemand {
    UntouchedDemand {
        links: vec!["a[href]".to_owned(), "a >".to_owned()],
        fields: vec![
            css("h1"),
            FieldLocator::Xpath {
                selector: "//p[@class='sku']".to_owned(),
            },
            css("table"),
        ],
        page_check: Some(PageCheck {
            login_page: Some(PageMatch::Css {
                selector: "form#signin".to_owned(),
            }),
            expected_page: Some(PageMatch::Css {
                selector: "p.sku".to_owned(),
            }),
        }),
        signs: true,
        keeps: false,
    }
}

/// A document is parsed once, its readers compiled once before it, and
/// each reads on that tree what it read of its own parse.
#[rstest]
#[case::page(PAGE)]
#[case::bare(BARE)]
fn one_tree_reads_what_each_reader_read_of_its_own_parse(#[case] html: &str) {
    let demand = demand();
    let work = CountedWork::default();
    let readers = UntouchedReaders::of(demand.clone(), &work);
    let compiled = work.read(Row::PageSelectorCompilations);
    let url = Url::parse(URL).unwrap();
    let tree = work.parse_html(Row::PageParses, html);
    let base = readers.base_href(&tree);
    let refreshed = readers.refresh(&url, &tree);
    let (read, _) = readers.read(tree, (URL, URL), (|_| [7; 32], &work));
    assert_eq!(
        (
            work.read(Row::PageParses),
            work.read(Row::PageSelectorCompilations)
        ),
        (1, compiled),
        "one parse, and no selector compiled for the page"
    );

    let alone = CountedWork::default();
    assert_eq!(
        base,
        alone
            .parse_html(Row::PageParses, html)
            .select_matcher(&alone.literal_selector(Row::PageSelectorCompilations, "base[href]"))
            .attr("href")
            .map(|href| href.to_string())
    );
    assert_eq!(refreshed, refresh::in_markup(&url, html.as_bytes(), &alone));
    for selector in &demand.links {
        assert_eq!(
            read.links(selector),
            Some(matched(
                &alone.parse_html(Row::PageParses, html),
                selector,
                &alone
            ))
        );
    }
    assert_eq!(
        read.fields(&demand.fields),
        Some(first_elements(
            &alone.parse_html(Row::PageParses, html),
            &demand.fields,
            &alone
        ))
    );
    let check = demand.page_check.as_ref().unwrap();
    let (verdict, robots) = read.judged(check).unwrap();
    assert_eq!(verdict, judge(check, URL, html, &alone));
    let headers = std::collections::BTreeMap::new();
    assert_eq!(
        goodfetch_page::noindex::marked(&headers, robots),
        expected_page::noindex(&headers, html)
    );
    let canonical = crate::canonical::canonical(html, &[], &url, &url, &alone);
    assert_eq!(read.signed(), Some(([7; 32], canonical.as_ref())));
}

/// What the Run did not ask for is not read, so its caller reads it alone.
#[test]
fn what_the_run_did_not_ask_for_is_not_read() {
    let work = CountedWork::default();
    let readers = UntouchedReaders::of(UntouchedDemand::default(), &work);
    let mut signed = false;
    let signing = |_| {
        signed = true;
        [0; 32]
    };
    let (read, _) = readers.read(
        work.parse_html(Row::PageParses, PAGE),
        (URL, URL),
        (signing, &work),
    );
    assert!(!signed);
    assert_eq!(read.links("a[href]"), None);
    assert_eq!(read.fields(&[css("h1")]), None);
    assert!(read.judged(&PageCheck::default()).is_none());
    assert_eq!(read.signed(), None);
}

/// The extractor's corpus, B1's page and this module's pages, each with
/// whether the streaming path vouches for its tree.
fn corpus() -> [(&'static str, bool); 12] {
    [
        (PAGE, true),
        (BARE, false),
        (
            include_str!("../../../../goodfetch-extractor/src/formats/fixtures/article.html"),
            true,
        ),
        (
            include_str!(
                "../../../../goodfetch-extractor/src/formats/fixtures/heading-with-break.html"
            ),
            true,
        ),
        (
            include_str!(
                "../../../../goodfetch-extractor/src/formats/fixtures/navigation-and-related.html"
            ),
            true,
        ),
        (
            include_str!(
                "../../../../goodfetch-extractor/src/formats/fixtures/no-main-content.html"
            ),
            true,
        ),
        (
            include_str!(
                "../../../../goodfetch-extractor/src/formats/fixtures/pre-without-code.html"
            ),
            true,
        ),
        (
            include_str!("../../../../goodfetch-extractor/src/formats/fixtures/row-headers.html"),
            true,
        ),
        (
            include_str!(
                "../../../../goodfetch-extractor/fuzz/formats/seeds/references-and-tables.html"
            ),
            false,
        ),
        (
            include_str!(
                "../../../../goodfetch-extractor/fuzz/formats/seeds/nested-table-and-fences.html"
            ),
            false,
        ),
        (B1_PAGE, true),
        (HIDDEN, true),
    ]
}

/// B1's page (`goodfetch-foxhound`'s `benches/scenarios.rs`).
const B1_PAGE: &str = "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Page 3</title></head>\n<body><h1>Page 3</h1>\n<nav><ul><li><a href=\"/\">Index</a></li><li><a href=\"/page-4\">Page 4</a></li></ul></nav>\n<main>\n<p>Paragraph 000: the crawler reads each page once.</p>\n</main></body></html>\n";

/// Links each way markup hides one, and the controls that reveal them,
/// written after them.
const HIDDEN: &str = r#"<!doctype html><html><head><base href="/kites/"><meta http-equiv="Refresh" content="0; url=elsewhere"><meta name="ROBOTS" content="nofollow"></head><body>
<div id="panel" hidden><a href="panel">Panel</a></div>
<p style="visibility: hidden"><a href="invisible" rel="nofollow">Invisible</a></p>
<span aria-hidden="true"><a href="aria">Aria</a></span>
<details><summary>More</summary><a href="details">Details &amp; more</a></details>
<div style="opacity:0"><a href="faded">Faded</a></div>
<div popover id="pop"><a href="popover">Popover</a></div>
<button aria-controls="panel pop">Open</button><a href="plain">Plain</a>
</body></html>"#;

/// The readers of a Run that needs no tree: no Page signature, CSS fields
/// and selectors lol-html reads, beside one the engine refuses.
fn streaming_demand() -> UntouchedDemand {
    UntouchedDemand {
        links: vec![
            "a[href]".to_owned(),
            "nav a".to_owned(),
            "[hidden] a, details a".to_owned(),
            "a >".to_owned(),
        ],
        fields: vec![css("h1"), css("title"), css("table")],
        page_check: Some(PageCheck {
            login_page: Some(PageMatch::Css {
                selector: "form#signin".to_owned(),
            }),
            expected_page: Some(PageMatch::Css {
                selector: "h1, p.sku".to_owned(),
            }),
        }),
        signs: false,
        keeps: false,
    }
}

/// On every document of the corpus, the streaming path reads with no
/// tree the same `<base>`, refresh directive, links and their trap-link
/// judgement, fields, Page check verdict and robots `meta` as the readers
/// read on the tree, or leaves the document to the tree.
#[test]
fn the_stream_reads_what_the_tree_reads_on_the_corpus() {
    let demand = streaming_demand();
    let url = Url::parse(URL).unwrap();
    for (html, streams) in corpus() {
        let work = CountedWork::default();
        let readers = UntouchedReaders::of(demand.clone(), &work);
        let compiled = work.read(Row::PageSelectorCompilations);
        let streamed = readers.stream(html);
        assert_eq!(streamed.is_some(), streams, "{html}");
        assert_eq!(
            (
                work.read(Row::PageParses),
                work.read(Row::PageSelectorCompilations)
            ),
            (0, compiled),
            "no parse and no selector compiled for the page"
        );
        let Some(streamed) = streamed else {
            continue;
        };
        let tree = work.parse_html(Row::PageParses, html);
        assert_eq!(streamed.base_href(), readers.base_href(&tree), "{html}");
        assert_eq!(
            streamed.refresh(&url),
            readers.refresh(&url, &tree),
            "{html}"
        );
        let (on_tree, _) = readers.read(tree, (URL, URL), (|_| [0; 32], &work));
        let on_stream = readers.read_streamed(streamed, URL);
        for selector in &demand.links {
            assert_eq!(
                on_stream.links(selector),
                on_tree.links(selector),
                "{selector} on {html}"
            );
        }
        assert_eq!(
            on_stream.fields(&demand.fields),
            on_tree.fields(&demand.fields),
            "{html}"
        );
        let check = demand.page_check.as_ref().unwrap();
        assert_eq!(on_stream.judged(check), on_tree.judged(check), "{html}");
        assert_eq!(on_stream.signed(), None);
    }
}

/// A Run whose readers need a tree never streams: one that learns, one
/// reading an `XPath` field or a selector lol-html does not read; nor
/// does a document whose markup names an AI-use signal.
#[rstest]
#[case::learning(UntouchedDemand { signs: true, ..streaming_demand() })]
#[case::xpath(UntouchedDemand {
    fields: vec![FieldLocator::Xpath { selector: "//h1".to_owned() }],
    ..streaming_demand()
})]
#[case::sibling(UntouchedDemand { links: vec!["h1 + a".to_owned()], ..streaming_demand() })]
fn readers_that_need_a_tree_never_stream(#[case] demand: UntouchedDemand) {
    let readers = UntouchedReaders::of(demand, &CountedWork::default());
    assert!(readers.stream(B1_PAGE).is_none());
}

/// A document whose markup names an AI-use signal is read on its tree.
#[test]
fn a_document_naming_a_use_signal_is_read_on_its_tree() {
    let readers = UntouchedReaders::of(streaming_demand(), &CountedWork::default());
    assert!(readers.stream(B1_PAGE).is_some());
    let signalled = B1_PAGE.replace(
        "<title>",
        "<meta name=\"tdm-reservation\" content=\"1\"><title>",
    );
    assert!(readers.stream(&signalled).is_none());
}

/// While a handler reads the Run's pages, the Page signature reads a copy
/// of the page's one tree, so the readers answer the tree as it was served
/// for the handler's reads, whatever the main-content step took out of the
/// copy: one parse and one copy for the page (spec #1577's #1918).
#[test]
fn a_handlers_page_keeps_its_untouched_tree_and_signs_a_copy() {
    let work = CountedWork::default();
    let readers = UntouchedReaders::of(
        UntouchedDemand {
            keeps: true,
            ..demand()
        },
        &work,
    );
    let tree = work.parse_html(Row::PageParses, PAGE);
    let served = tree.html().to_string();
    let main = work.literal_selector(Row::PageSelectorCompilations, "main");
    let stripping = |tree: ParsedHtml| {
        tree.select_matcher(&main).remove();
        [7; 32]
    };

    let (read, kept) = readers.read(tree, (URL, URL), (stripping, &work));

    assert_eq!(
        (
            read.signed().map(|(signature, _)| signature),
            kept.map(|tree| tree.html().to_string()),
            work.read(Row::PageParses),
            work.read(Row::PageTreeCopies),
        ),
        (Some([7; 32]), Some(served), 1, 1)
    );
}
