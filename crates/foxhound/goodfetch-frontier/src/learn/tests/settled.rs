//! The bound of learning's evidence (ADR 0005's note of 2026-11-10; ADR
//! 0120's of 2026-10-24): a parameter or a form that is refused or stopped
//! takes no more evidence, a learned one no more matching pairs, and an
//! offset parameter no more values than its decision needs, however many
//! pages follow, while every rule is learned, refused and stopped as
//! before. Read in the run file's tables, as the bound is one on its rows,
//! and in the rows of evidence each page's learning reads.

use std::collections::BTreeMap;

use goodfetch_counted_work::Row;

use super::{Crawl, ITEM, LIST, at, at_site, crawl_listing, slash_pair};

/// The tables that hold the evidence of a parameter or a form, and an
/// offset parameter's values.
const BOUNDED: [&str; 4] = [
    "rules_pairs",
    "rules_residues",
    "rules_canonicals",
    "rules_offset_values",
];

impl Crawl {
    /// The rows of [`BOUNDED`]'s tables, by table.
    fn evidence(&self) -> BTreeMap<String, i64> {
        let mut rows = self.rows();
        rows.retain(|table, _| BOUNDED.contains(&table.as_str()));
        rows
    }

    /// The page at `url` fetched with `text`, naming `canonical`: how many
    /// rows of evidence learning from it read.
    fn reads_of(&mut self, url: &str, text: &str, canonical: Option<&str>) -> u64 {
        let before = self.work.read(Row::EvidenceReads);
        self.fetched_naming(url, text, canonical);
        self.work.read(Row::EvidenceReads) - before
    }
}

fn rows(expected: &[(&str, i64)]) -> BTreeMap<String, i64> {
    expected
        .iter()
        .map(|(table, count)| ((*table).to_owned(), *count))
        .collect()
}

/// The item whose `id` no page of [`items`] has, admitted with them and
/// left to the test to fetch.
const LATE: &str = "http://shop.test/item?id=late";

/// A site of `pages` pages `/item?id=<n>`, each enqueued from the first,
/// and [`LATE`] after them, then each fetched with `text`'s main text: the
/// crawl, and the most rows of evidence one page's learning read.
fn items(pages: u32, text: impl Fn(u32) -> String) -> (Crawl, u64) {
    let mut crawl = Crawl::new();
    let item = |number: u32| format!("{ITEM}?id={number}");
    crawl.seed(&item(0));
    for number in 1..pages {
        assert_eq!(crawl.enqueue(&item(0), &item(number)), "added");
    }
    assert_eq!(crawl.enqueue(&item(0), LATE), "added");
    let most = (0..pages)
        .map(|number| crawl.reads_of(&item(number), &text(number), None))
        .max()
        .unwrap();
    (crawl, most)
}

/// On a site whose pages differ by one parameter, each page its own, the
/// second page refuses the parameter, and from then on a page adds no
/// pair, no residue and no offset value and reads one row of evidence, the
/// pair that refused it: three hundred pages leave the rows thirty do.
///
/// Covers: story:6-216
#[rstest::rstest]
#[case::thirty_pages(30)]
#[case::three_hundred_pages(300)]
fn a_refused_parameter_takes_no_more_evidence_however_many_pages_follow(#[case] pages: u32) {
    let (crawl, most) = items(pages, |number| format!("Item {number}"));

    assert_eq!(
        (crawl.evidence(), most, crawl.rules()),
        (
            rows(&[
                ("rules_offset_values", 2),
                ("rules_pairs", 1),
                ("rules_residues", 2)
            ]),
            2,
            Vec::new(),
        )
    );
}

/// On a site whose pages all read alike under one parameter, the third
/// page teaches that it has no effect, on the three pairs of the first
/// three pages, and the pages still fetched after it add no pair.
///
/// Covers: story:6-216
#[test]
fn a_learned_parameter_takes_no_more_matching_pairs() {
    let (crawl, _) = items(300, |_| "Item".to_owned());

    let rule = crawl.rules().remove(0);
    assert_eq!(
        (
            rule.evidence,
            rule.contradiction,
            crawl.evidence()["rules_pairs"]
        ),
        (
            format!(
                "3 pairs of URLs differing only by `id` served pages of matching signatures, \
                 and no pair pages of differing ones: {ITEM}?id=0 and {ITEM}?id=1; {ITEM}?id=0 \
                 and {ITEM}?id=2; {ITEM}?id=1 and {ITEM}?id=2."
            ),
            None,
            3,
        )
    );
}

/// A parameter learned long before, three hundred matching pages since,
/// still stops on the first page that differs, which is paired with each
/// page of its query.
///
/// Covers: story:6-218
#[test]
fn a_learned_parameter_still_stops_on_a_differing_page_after_many_matching_ones() {
    let (mut crawl, _) = items(300, |_| "Item".to_owned());

    crawl.fetched(LATE, "Another item");

    let rule = crawl.rules().remove(0);
    assert_eq!(
        (
            rule.contradiction.is_some(),
            crawl.evidence()["rules_pairs"]
        ),
        (true, 303)
    );
}

/// Fetches forty pages `/p<n>`, each naming its folder `/p<n>/` as its
/// canonical, and each folder the crawl admits, reading alike: the most
/// rows of evidence one of them read.
fn folders(crawl: &mut Crawl) -> u64 {
    let mut most = 0;
    for number in 0..40 {
        let (page, folder) = (
            at_site(&format!("/p{number}")),
            at_site(&format!("/p{number}/")),
        );
        crawl.seed(&page);
        most = most.max(crawl.reads_of(&page, "P", Some(&folder)));
        // Under a learned form the folder is a `duplicate`, not fetched.
        if crawl.seed(&folder) == "added" {
            most = most.max(crawl.reads_of(&folder, "P", None));
        }
    }
    most
}

/// A form a differing pair refused takes no more pairs and no canonical,
/// whatever its pages then show: forty pairs of them matching, each first
/// page naming its folder as its canonical, leave the two pairs it had,
/// and a page reads the pair that refused it, once for its own pair and
/// once for its canonical.
///
/// Covers: story:6-217, story:6-219
#[test]
fn a_refused_form_takes_no_more_pairs_and_no_canonical() {
    let mut crawl = Crawl::new();
    crawl.seed(&at_site("/"));
    slash_pair(&mut crawl, "/a", "A");
    for url in [at_site("/b"), at_site("/b/")] {
        crawl.enqueue(&at_site("/"), &url);
    }
    crawl.fetched(&at_site("/b"), "B");
    crawl.fetched(&at_site("/b/"), "B, as a folder");

    let most = folders(&mut crawl);

    assert_eq!(
        (crawl.evidence(), crawl.rules(), most),
        (rows(&[("rules_pairs", 2)]), Vec::new(), 2)
    );
}

/// A learned form takes no canonical: forty pages that name their folder
/// leave the two pairs that taught it, and its evidence as it was.
///
/// Covers: story:6-217, story:6-219
#[test]
fn a_learned_form_takes_no_canonical() {
    let mut crawl = Crawl::new();
    crawl.seed(&at_site("/"));
    slash_pair(&mut crawl, "/a", "A");
    slash_pair(&mut crawl, "/b", "B");

    let most = folders(&mut crawl);

    assert_eq!(
        (crawl.evidence(), crawl.rules().remove(0).evidence, most),
        (
            rows(&[("rules_pairs", 2)]),
            format!(
                "2 pairs of URLs differing only by a trailing slash served pages of matching \
                 signatures, and no pair pages of differing ones: {} and {}; {} and {}.",
                at_site("/a"),
                at_site("/a/"),
                at_site("/b"),
                at_site("/b/")
            ),
            0,
        )
    );
}

/// A listing paged by its offset keeps the three lowest values its cursor
/// rule's evidence names, whatever the order its pages are met in, and one
/// whose parameter steps by 1, which is no offset, keeps the two values
/// that showed it.
///
/// Covers: story:6-216
#[test]
fn an_offset_parameter_keeps_only_the_values_its_decision_needs() {
    let mut crawl = Crawl::new();
    crawl.seed(&at(40, 20));
    for page in (0..40).rev() {
        assert_eq!(crawl.enqueue(&at(page + 1, 20), &at(page, 20)), "added");
    }
    let cursors = [1, 2, 3].map(|page| format!("{LIST}?after=r-{page}"));
    for (page, cursor) in (38..=40).zip(&cursors) {
        assert_eq!(crawl.enqueue(&at(page, 20), cursor), "added");
    }
    let mut numbered = Crawl::new();
    crawl_listing(&mut numbered, 40, 1, None);

    assert_eq!(
        (
            crawl.evidence(),
            crawl.rules().remove(0).evidence,
            numbered.evidence(),
            numbered.completeness(),
        ),
        (
            rows(&[("rules_offset_values", 3)]),
            format!(
                "`offset` steps by 20 on {LIST} ({LIST}?offset=0, {LIST}?offset=20, \
                 {LIST}?offset=40), and three of its pages each link one value of `after` of \
                 their own: {LIST}?offset=760 links {}, {LIST}?offset=780 links {}, and \
                 {LIST}?offset=800 links {}.",
                cursors[0], cursors[1], cursors[2]
            ),
            rows(&[
                ("rules_offset_values", 2),
                ("rules_residues", 2),
                ("rules_pairs", 1)
            ]),
            Vec::new(),
        )
    );
}
