//! The readers of the untouched page through their interface: one parse
//! of a document gives what each reader read of its own parse before
//! (spec #1577's #1910), and the Page signature consumes the tree last.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a test's own parse, on no page's path"
)]

use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_protocol::page::FormatsRequest;
use goodfetch_protocol::page::read::FieldLocator;
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};
use proptest::prelude::*;
use rstest::rstest;
use url::Url;

use super::{StreamRead, UntouchedDemand, UntouchedReaders};
use crate::document::{first_elements_in, locating, matched};
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
        formats: FormatsRequest::default(),
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
        Some(
            demand
                .fields
                .iter()
                .map(|locator| locating(locator, &alone))
                .collect::<Result<Vec<_>, _>>()
                .map(|locating| {
                    first_elements_in(&alone.parse_html(Row::PageParses, html), &locating)
                })
        )
    );
    let check = demand.page_check.as_ref().unwrap();
    let (verdict, robots) = read.judged(check).unwrap();
    let headers = std::collections::BTreeMap::new();
    assert_eq!(
        (
            verdict,
            verdict == expected_page::Verdict::NotExpected
                && goodfetch_page::noindex::marked(&headers, robots)
        ),
        judge(check, URL, html, &headers, &alone)
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
        formats: FormatsRequest::default(),
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

/// What `readers` read of `html` in one pass, its text written in chunks
/// cut at `cuts`, in order.
fn passed(readers: &Arc<UntouchedReaders>, html: &str, cuts: &[usize]) -> Option<StreamRead> {
    let mut pass = readers.pass().expect("the readers read in one pass");
    let mut at = 0;
    for &cut in cuts.iter().chain(&[html.len()]) {
        pass.write(&html.as_bytes()[at..cut]);
        at = cut;
    }
    pass.end()
        .by(readers)
        .expect("the readers that started it read it")
        .read()
}

/// The corpus, and B1's page naming an AI-use signal.
fn chunked_corpus() -> Vec<String> {
    let signalled = B1_PAGE.replace(
        "<title>",
        "<meta name=\"tdm-reservation\" content=\"1\"><title>",
    );
    corpus()
        .into_iter()
        .map(|(html, _)| html.to_owned())
        .chain([signalled])
        .collect()
}

/// The parts the every-cut test runs in: part `p` cuts each text at the
/// offsets congruent to `p`, so each part makes an eighth of the passes and
/// stays far under the Quick suite's kill on a loaded host (#2776).
const CUT_PARTS: usize = 8;

/// The pass over a crawl request's own document, read in two chunks cut at
/// any byte, reads what the readers read in one pass of the whole text, or
/// leaves the document to the tree as they do, a doctype, an end tag or a
/// signal's name split between two chunks included (issue 2188).
#[rstest]
fn a_pass_reads_a_text_cut_at_any_byte_as_it_reads_it_whole(
    #[values(0, 1, 2, 3, 4, 5, 6, 7)] part: usize,
) {
    let work = CountedWork::default();
    let readers = Arc::new(UntouchedReaders::of(streaming_demand(), &work));
    for html in chunked_corpus() {
        let whole = readers.stream(&html);
        for cut in (part..=html.len()).step_by(CUT_PARTS) {
            assert_eq!(
                passed(&readers, &html, &[cut]),
                whole,
                "cut at {cut} of {html}"
            );
        }
    }
    assert_eq!(work.read(Row::PageParses), 0);
}

/// The same pass, its text written a byte at a time.
#[test]
fn a_pass_reads_a_text_written_byte_by_byte_as_it_reads_it_whole() {
    let work = CountedWork::default();
    let readers = Arc::new(UntouchedReaders::of(streaming_demand(), &work));
    for html in chunked_corpus() {
        let cuts: Vec<usize> = (1..html.len()).collect();
        assert_eq!(
            passed(&readers, &html, &cuts),
            readers.stream(&html),
            "{html}"
        );
    }
    assert_eq!(work.read(Row::PageParses), 0);
}

#[test]
fn readers_of_another_label_do_not_read_a_pass() {
    let work = CountedWork::default();
    let (readers, other) = (
        Arc::new(UntouchedReaders::of(streaming_demand(), &work)),
        UntouchedReaders::of(streaming_demand(), &work),
    );
    let mut pass = readers.pass().expect("the readers read in one pass");
    pass.write(PAGE.as_bytes());

    assert!(pass.end().by(&other).is_none());
}

#[test]
fn readers_that_need_a_tree_start_no_pass() {
    let readers = Arc::new(UntouchedReaders::of(
        UntouchedDemand {
            signs: true,
            ..streaming_demand()
        },
        &CountedWork::default(),
    ));

    assert!(readers.pass().is_none());
}

/// A page of 300 sections, each with this module's links, fields and
/// controls, multi-byte text and a comment, and no table, since the pass
/// does not vouch for a table's tree.
fn large_page() -> String {
    let mut html = String::from(
        "<!doctype html><html><head><title>Large</title><base href=\"/k/\"><meta name=\"robots\" content=\"noindex\"></head><body>\n",
    );
    for section in 0..300 {
        write!(
            html,
            "<section><h1>Café {section} 漢字</h1><!-- note ç {section} -->\n<nav><a href=\"/n/{section}\">Nav</a></nav><div id=\"p{section}\" hidden><a href=\"h/{section}\">Hidden &amp; more</a></div>\n<button aria-controls=\"p{section}\">Open</button><p class=\"sku\">naïve 🦀 {section}</p><ul><li>{section}</li><li><a href=\"l/{section}\">List</a></li></ul></section>\r\n",
        )
        .expect("invariant: writing to a String cannot fail");
    }
    html.push_str("</body></html>");
    html
}

/// The large page and what the readers read of it in one pass of the whole
/// text, read once for every case (#2776).
static LARGE_PAGE: LazyLock<(String, Option<StreamRead>)> = LazyLock::new(|| {
    let html = large_page();
    let whole = UntouchedReaders::of(streaming_demand(), &CountedWork::default()).stream(&html);
    (html, whole)
});

/// The parts the large-page property runs in: each runs this share of the
/// suite's cases on a seed of its own, so the suite's count is kept and each
/// part stays far under the Quick suite's kill on a loaded host (#2776).
const LARGE_PAGE_PARTS: u32 = 4;

/// The suite's proptest configuration, with a part's share of its cases.
fn large_page_part_config() -> ProptestConfig {
    let config = goodfetch_test_support::proptest_config();
    ProptestConfig {
        cases: config.cases / LARGE_PAGE_PARTS,
        ..config
    }
}

/// Up to 63 offsets into the large page, in any order.
fn large_page_cuts() -> impl Strategy<Value = Vec<prop::sample::Index>> {
    prop::collection::vec(any::<prop::sample::Index>(), 0..64)
}

/// The large page, read in the chunks that `cuts` end, reads as it is read
/// whole.
fn the_large_page_cut_at(cuts: &[prop::sample::Index]) -> Result<(), TestCaseError> {
    let (html, whole) = &*LARGE_PAGE;
    let readers = Arc::new(UntouchedReaders::of(
        streaming_demand(),
        &CountedWork::default(),
    ));
    prop_assert!(whole.is_some());
    let mut cuts: Vec<usize> = cuts.iter().map(|cut| cut.index(html.len() + 1)).collect();
    cuts.sort_unstable();
    prop_assert_eq!(&passed(&readers, html, &cuts), whole);
    Ok(())
}

proptest! {
    #![proptest_config(large_page_part_config())]

    /// A large page cut anywhere is read in chunks as it is read whole: the
    /// first of the property's parts.
    #[test]
    fn a_large_page_cut_anywhere_reads_as_it_reads_whole_part_1(cuts in large_page_cuts()) {
        the_large_page_cut_at(&cuts)?;
    }

    /// The second part.
    #[test]
    fn a_large_page_cut_anywhere_reads_as_it_reads_whole_part_2(cuts in large_page_cuts()) {
        the_large_page_cut_at(&cuts)?;
    }

    /// The third part.
    #[test]
    fn a_large_page_cut_anywhere_reads_as_it_reads_whole_part_3(cuts in large_page_cuts()) {
        the_large_page_cut_at(&cuts)?;
    }

    /// The fourth part.
    #[test]
    fn a_large_page_cut_anywhere_reads_as_it_reads_whole_part_4(cuts in large_page_cuts()) {
        the_large_page_cut_at(&cuts)?;
    }
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
