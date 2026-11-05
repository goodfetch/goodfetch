//! The readers of the untouched page through their interface: one parse
//! of a document gives what each reader read of its own parse before
//! (spec #1577's #1910), and the Page signature consumes the tree last.

use goodfetch_counted_work::{CountedWork, Row};
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
    let read = readers.read(tree, (URL, URL), |_| [7; 32]);
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
        assert_eq!(read.links(selector), Some(matched(html, selector, &alone)));
    }
    assert_eq!(
        read.fields(&demand.fields),
        Some(first_elements(html, &demand.fields, &alone))
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
    let read = readers.read(work.parse_html(Row::PageParses, PAGE), (URL, URL), |_| {
        signed = true;
        [0; 32]
    });
    assert!(!signed);
    assert_eq!(read.links("a[href]"), None);
    assert_eq!(read.fields(&[css("h1")]), None);
    assert!(read.judged(&PageCheck::default()).is_none());
    assert_eq!(read.signed(), None);
}
