//! The learned rules as a pure function of what a crawl showed the
//! Frontier (spec #581's thresholds; ADR 0005): seeds and enqueues through
//! `admit`, each page fetched through `learn` with its page signature, and
//! what the Frontier then answers, reports and says of the Run's
//! Completeness, in a run file in memory; no owned code is mocked.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod model;

use std::collections::BTreeMap;

use goodfetch_engine::runs::CompletenessReason;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod};
use goodfetch_protocol::foxhound::report::{LearnedRule, LearnedRuleKind};
use goodfetch_protocol::run::{AxisReason, Completeness};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use url::Url;

use super::{PageSeen, learn};
use crate::admit::{Candidate, admit};
use crate::identity::Declared;
use crate::rules::{self, learned_rules};
use crate::scope::{self, Narrowing, Patterns, Scope, Strategy};
use crate::{bounds, identity};

/// A crawl as the Frontier sees it, in a run file in memory: what it
/// admitted, each URL with its crawl request's ID.
struct Crawl {
    connection: Connection,
    scope: Scope,
    ids: BTreeMap<String, String>,
}

impl Crawl {
    fn new() -> Self {
        let connection = Connection::open_in_memory().unwrap();
        for statement in identity::TABLES
            .statements
            .iter()
            .chain(scope::TABLES.statements)
            .chain(bounds::TABLES.statements)
            .chain(rules::TABLES.statements)
        {
            connection.execute(statement, []).unwrap();
        }
        Self {
            connection,
            scope: Scope::new(
                Strategy::SameHostname,
                Patterns::default(),
                Patterns::default(),
            ),
            ids: BTreeMap::new(),
        }
    }

    fn admit(&mut self, candidate: Candidate<'_>) -> Enqueued {
        let next = (self.ids.len() + 1).to_string();
        let answer = admit(&self.connection, &self.scope, candidate, |_| {
            Ok(next.clone())
        })
        .unwrap();
        if let Enqueued::Added { id, url } = &answer {
            self.ids.insert(url.clone(), id.clone());
        }
        answer
    }

    fn seed(&mut self, url: &str) -> String {
        answer(&self.admit(Candidate::Seed {
            url,
            declared: get(),
        }))
    }

    /// Enqueues `url` from the page at `from`, which the crawl admitted.
    fn enqueue(&mut self, from: &str, url: &str) -> String {
        let from = Url::parse(from).unwrap();
        answer(&self.admit(Candidate::Enqueue {
            url,
            base: &from,
            from: &from,
            declared: get(),
            narrowing: Narrowing::none(),
            depth: 1,
        }))
    }

    /// The page at `url`, which the crawl admitted, fetched with the main
    /// text `text`.
    fn fetched(&self, url: &str, text: &str) {
        let crawl_request = &self.ids[url];
        learn(
            &self.connection,
            PageSeen {
                crawl_request,
                url: &Url::parse(url).unwrap(),
                method: get().method,
                body: None,
                signature: Sha256::digest(text).into(),
            },
        )
        .unwrap();
    }

    fn rules(&self) -> Vec<LearnedRule> {
        learned_rules(&self.connection).unwrap()
    }

    fn completeness(&self) -> Vec<CompletenessReason> {
        rules::completeness_reasons(&self.connection).unwrap()
    }
}

/// A `GET` with no body and no declared key.
fn get() -> Declared<'static> {
    static GET: std::sync::LazyLock<HttpMethod> = std::sync::LazyLock::new(HttpMethod::get);
    Declared {
        method: &GET,
        body: None,
        key: None,
    }
}

/// An answer: `added`, or the reason it was not.
fn answer(enqueued: &Enqueued) -> String {
    match enqueued {
        Enqueued::Added { .. } => "added".to_owned(),
        Enqueued::NotAdded { reason, .. } => serde_json::to_value(reason)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned(),
    }
}

fn strings(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

const ITEM: &str = "http://shop.test/item";

/// Fetches `?n=<number>` and, from it, `?n=<number>&ref=<reference>`,
/// both reading `text_with` when given, else alike: one pair of URLs
/// differing only by `ref`.
fn pair(crawl: &mut Crawl, number: u32, reference: &str, text_with: Option<&str>) {
    let without = format!("{ITEM}?n={number}");
    let with = format!("{ITEM}?n={number}&ref={reference}");
    let text = format!("Item {number}");
    if !crawl.ids.contains_key(&without) {
        crawl.enqueue(&format!("{ITEM}?n=0"), &without);
    }
    crawl.fetched(&without, &text);
    assert_eq!(crawl.enqueue(&without, &with), "added");
    crawl.fetched(&with, text_with.unwrap_or(&text));
}

/// Three pairs of URLs differing only by `ref` that serve one page teach
/// the Frontier that `ref` has no effect on the host: a URL that differs
/// from an admitted one only by `ref` is then a `duplicate`, whether the
/// admitted one carried `ref` or not, and the report lists the rule with
/// its pairs and each URL it merged.
///
/// Covers: story:6-216, story:6-310
#[test]
fn three_matching_pairs_teach_a_parameter_with_no_effect() {
    let mut crawl = Crawl::new();
    crawl.seed(&format!("{ITEM}?n=0"));
    pair(&mut crawl, 1, "a", None);
    pair(&mut crawl, 2, "b", None);
    assert_eq!(crawl.enqueue(ITEM, &format!("{ITEM}?n=5&ref=a")), "added");
    assert!(crawl.rules().is_empty(), "two pairs teach nothing");

    pair(&mut crawl, 3, "c", None);
    let answers = [
        crawl.enqueue(ITEM, &format!("{ITEM}?n=1&ref=z")),
        crawl.enqueue(ITEM, &format!("{ITEM}?n=5")),
        crawl.enqueue(ITEM, &format!("{ITEM}?n=4&ref=q")),
        crawl.enqueue(ITEM, &format!("{ITEM}?n=4")),
        crawl.enqueue(ITEM, &format!("{ITEM}?ref=q&n=6&sort=new")),
    ];

    assert_eq!(
        (answers.to_vec(), crawl.rules(), crawl.completeness()),
        (
            strings(&["duplicate", "duplicate", "added", "duplicate", "added"]),
            vec![LearnedRule {
                kind: LearnedRuleKind::NoEffectParameter,
                host: "shop.test".to_owned(),
                path: None,
                parameter: Some("ref".to_owned()),
                evidence: format!(
                    "3 pairs of URLs differing only by `ref` served pages of matching signatures, \
                     and no pair pages of differing ones: {ITEM}?n=1 and {ITEM}?n=1&ref=a; \
                     {ITEM}?n=2 and {ITEM}?n=2&ref=b; {ITEM}?n=3 and {ITEM}?n=3&ref=c."
                ),
                merged: strings(&[
                    &format!("{ITEM}?n=1&ref=z"),
                    &format!("{ITEM}?n=5"),
                    &format!("{ITEM}?n=4"),
                ]),
            }],
            Vec::new(),
        )
    );
}

/// A pair whose pages differ keeps the parameter from ever being learned,
/// however many matching pairs follow, and a page fetched again (a retry)
/// makes no pair with itself.
///
/// Covers: story:6-216
#[test]
fn a_differing_pair_keeps_a_parameter_from_being_learned() {
    let mut crawl = Crawl::new();
    crawl.seed(&format!("{ITEM}?n=0"));
    pair(&mut crawl, 1, "a", None);
    pair(&mut crawl, 2, "b", Some("Item 2, in another colour"));
    pair(&mut crawl, 3, "c", None);
    pair(&mut crawl, 4, "d", None);
    crawl.fetched(&format!("{ITEM}?n=4&ref=d"), "Item 4");

    assert_eq!(
        (
            crawl.enqueue(ITEM, &format!("{ITEM}?n=1&ref=z")),
            crawl.rules()
        ),
        ("added".to_owned(), Vec::new())
    );
}

/// A parameter learned on one host merges nothing on another, though both
/// share a registrable domain, nor on a host of another domain.
///
/// Covers: story:6-216
#[test]
fn a_learned_parameter_merges_only_on_its_host() {
    let mut crawl = Crawl::new();
    crawl.seed(&format!("{ITEM}?n=0"));
    crawl.seed("http://www.shop.test/item?n=1");
    crawl.seed("http://other.test/item?n=1");
    for (number, reference) in [(1, "a"), (2, "b"), (3, "c")] {
        pair(&mut crawl, number, reference, None);
    }

    let answers = [
        crawl.enqueue(ITEM, &format!("{ITEM}?n=1&ref=z")),
        crawl.enqueue(ITEM, "http://www.shop.test/item?n=1&ref=z"),
        crawl.enqueue(
            "http://other.test/item?n=1",
            "http://other.test/item?n=1&ref=z",
        ),
    ];

    assert_eq!(answers.to_vec(), strings(&["duplicate", "added", "added"]));
}

const CALENDAR: &str = "http://events.test/calendar";

/// The month `index` months after January 2026, as the calendar links it.
fn month(index: u32) -> String {
    format!("{CALENDAR}/{}/{:02}", 2026 + index / 12, index % 12 + 1)
}

/// Crawls the calendar from its home page: each month reads `text(index)`
/// and links the next month and the home page, until `months` are fetched
/// or the next month is not added; answers the next month's answer from
/// each page.
fn crawl_calendar(crawl: &mut Crawl, months: u32, text: impl Fn(u32) -> String) -> Vec<String> {
    let home = "http://events.test/";
    crawl.seed(home);
    crawl.fetched(home, "Home");
    let mut answers = vec![crawl.enqueue(home, &month(0))];
    for index in 0..months {
        crawl.fetched(&month(index), &text(index));
        assert_eq!(crawl.enqueue(&month(index), home), "duplicate");
        let next = crawl.enqueue(&month(index), &month(index + 1));
        let refused = next != "added";
        answers.push(next);
        if refused {
            break;
        }
    }
    answers
}

/// A trap: 100 pages under one path pattern after the first, bringing no
/// new page signature and no URL outside it, close the pattern, whose
/// later URLs are not added with `trap_path`; the Run is `unverified`,
/// naming the count, and the report lists the rule with the URLs it
/// refused. One page fewer closes nothing.
///
/// Covers: story:6-216, story:6-310
#[test]
fn a_trap_closes_after_a_hundred_pages_that_bring_nothing_new() {
    let mut closed = Crawl::new();
    let mut short = Crawl::new();

    let answers = crawl_calendar(&mut closed, 200, |_| "No events this month.".to_owned());
    let short_answers = crawl_calendar(&mut short, 100, |_| "No events this month.".to_owned());

    let mut expected = vec!["added".to_owned(); 101];
    expected.push("trap_path".to_owned());
    assert_eq!(
        (answers, closed.rules(), closed.completeness()),
        (
            expected,
            vec![LearnedRule {
                kind: LearnedRuleKind::TrapPath,
                host: "events.test".to_owned(),
                path: Some("/calendar/{number}/{number}".to_owned()),
                parameter: None,
                evidence: format!(
                    "100 pages under `/calendar/{{number}}/{{number}}` in a row, from {} to {}, \
                     brought no new page signature and no URL outside it.",
                    month(1),
                    month(100)
                ),
                merged: vec![month(101)],
            }],
            vec![CompletenessReason {
                completeness: Completeness::Unverified,
                reason: AxisReason {
                    code: "trap_path".to_owned(),
                    message:
                        "A learned trap refused 1 URL the crawl would otherwise have admitted."
                            .to_owned(),
                    call_id: None,
                },
            }],
        )
    );
    assert_eq!(
        (short_answers.len(), short.rules(), short.completeness()),
        (101, Vec::new(), Vec::new())
    );
}

/// A page under the pattern that brings a new page signature, or a URL
/// outside the pattern, starts the count again.
///
/// Covers: story:6-216
#[test]
fn a_new_page_or_a_url_outside_the_pattern_starts_the_count_again() {
    let mut signed = Crawl::new();
    let mut linked = Crawl::new();

    let signed_answers = crawl_calendar(&mut signed, 150, |index| {
        if index == 40 {
            "A concert this month.".to_owned()
        } else {
            "No events this month.".to_owned()
        }
    });
    linked.seed("http://events.test/");
    linked.fetched("http://events.test/", "Home");
    linked.enqueue("http://events.test/", &month(0));
    let mut linked_answers = Vec::new();
    for index in 0..150 {
        linked.fetched(&month(index), "No events this month.");
        if index == 40 {
            assert_eq!(
                linked.enqueue(&month(index), "http://events.test/concerts/1"),
                "added"
            );
        }
        let next = linked.enqueue(&month(index), &month(index + 1));
        let refused = next != "added";
        linked_answers.push(next);
        if refused {
            break;
        }
    }

    assert_eq!(signed_answers.iter().filter(|a| *a == "added").count(), 141);
    assert_eq!(signed_answers.last().map(String::as_str), Some("trap_path"));
    assert_eq!(linked_answers[139], "added");
    assert_eq!(linked_answers[140], "trap_path");
}

const LIST: &str = "http://table.test/list";

/// The listing's page `page` by `offset`, pages `step` rows long.
fn at(page: u32, step: u32) -> String {
    format!("{LIST}?offset={}", page * step)
}

/// Crawls the listing by `offset` from 0, each page linking the next one,
/// `pages` pages in all, the last also linking `cursor` when given.
fn crawl_listing(crawl: &mut Crawl, pages: u32, step: u32, cursor: Option<&str>) {
    crawl.seed(&at(0, step));
    for page in 0..pages {
        crawl.fetched(&at(page, step), &format!("Rows {page}"));
        if page + 1 < pages {
            assert_eq!(crawl.enqueue(&at(page, step), &at(page + 1, step)), "added");
        }
    }
    if let Some(cursor) = cursor {
        assert_eq!(crawl.enqueue(&at(pages - 1, step), cursor), "added");
    }
}

/// The answer to the next offset, enqueued from the listing's last page.
fn next_offset(crawl: &mut Crawl, pages: u32, step: u32) -> String {
    crawl.enqueue(&at(pages - 1, step), &at(pages, step))
}

fn offset_reason() -> Vec<CompletenessReason> {
    vec![CompletenessReason {
        completeness: Completeness::Unverified,
        reason: AxisReason {
            code: "offset_pagination".to_owned(),
            message: "1 listing was paged only by an offset parameter, so a row that moved \
                      while the crawl paged it may be missed."
                .to_owned(),
            call_id: None,
        },
    }]
}

/// An offset parameter, whose values step by one page size on one path,
/// gives way to a cursor link found on a page of that listing: the offset
/// URLs not yet fetched are not added, with `duplicate`, the report lists
/// the rule with what it refused, and the Run is not `unverified` by it.
///
/// Covers: story:6-216, story:6-310
#[test]
fn a_cursor_found_on_an_offset_listing_is_preferred() {
    let cursor = format!("{LIST}?after=r-40");
    let mut crawl = Crawl::new();
    crawl_listing(&mut crawl, 3, 20, Some(&cursor));

    let next = next_offset(&mut crawl, 3, 20);

    assert_eq!(
        (next, crawl.rules(), crawl.completeness()),
        (
            "duplicate".to_owned(),
            vec![LearnedRule {
                kind: LearnedRuleKind::CursorPagination,
                host: "table.test".to_owned(),
                path: Some("/list".to_owned()),
                parameter: Some("offset".to_owned()),
                evidence: format!(
                    "`offset` steps by 20 on {LIST} ({LIST}?offset=0, {LIST}?offset=20, \
                     {LIST}?offset=40), and {LIST}?offset=40 links the cursor {cursor}."
                ),
                merged: vec![format!("{LIST}?offset=60")],
            }],
            Vec::new(),
        )
    );
}

/// A listing walked only by its offset leaves the Run `unverified`, while
/// two offsets, or a parameter that steps by 1 as page numbers do, are no
/// offset pagination, and a cursor link beside them is no rule.
///
/// Covers: story:6-216, story:6-310
#[test]
fn a_listing_paged_only_by_offset_leaves_the_run_unverified() {
    let cursor = format!("{LIST}?after=r-40");
    let (mut offset_only, mut short, mut numbered) = (Crawl::new(), Crawl::new(), Crawl::new());
    crawl_listing(&mut offset_only, 3, 20, None);
    crawl_listing(&mut short, 2, 20, Some(&cursor));
    crawl_listing(&mut numbered, 3, 1, Some(&cursor));

    let short_state = (short.rules(), short.completeness());
    let answers = [
        next_offset(&mut offset_only, 3, 20),
        next_offset(&mut numbered, 3, 1),
    ];

    assert_eq!(
        (
            answers.to_vec(),
            (offset_only.rules(), offset_only.completeness()),
            short_state,
            (numbered.rules(), numbered.completeness()),
        ),
        (
            strings(&["added", "added"]),
            (Vec::new(), offset_reason()),
            (Vec::new(), Vec::new()),
            (Vec::new(), Vec::new()),
        )
    );
}

/// A cursor-like link that a page of another path gives is no cursor of
/// the listing.
///
/// Covers: story:6-216
#[test]
fn a_cursor_link_from_another_path_is_none() {
    let mut crawl = Crawl::new();
    crawl.seed("http://table.test/");
    crawl.fetched("http://table.test/", "Home");
    crawl_listing(&mut crawl, 3, 20, None);

    let answers = [
        crawl.enqueue("http://table.test/", &format!("{LIST}?after=r-40")),
        next_offset(&mut crawl, 3, 20),
    ];

    assert_eq!(
        (answers.to_vec(), crawl.rules()),
        (strings(&["added", "added"]), Vec::new())
    );
}
