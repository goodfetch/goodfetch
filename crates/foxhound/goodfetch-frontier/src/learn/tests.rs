//! The learned rules as a pure function of what a crawl showed the
//! Frontier (spec #581's thresholds; ADR 0005): seeds and enqueues through
//! `admit`, each page fetched through `learn` with its page signature, and
//! what the Frontier then answers, reports and says of the Run's
//! Completeness, in a run file in memory; no owned code is mocked.
mod model;

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::CompletenessReason;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod};
use goodfetch_protocol::foxhound::report::{EquivalentForm, LearnedRule, LearnedRuleKind};
use goodfetch_protocol::run::{AxisReason, Completeness};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use url::Url;

use super::{EndMerged, PageSeen, Redirect, learn, learn_redirect};
use crate::admit::{Candidate, admit};
use crate::bounds::{self, Bounds};
use crate::identity;
use crate::identity::Declared;
use crate::rules::{self, learned_rules};
use crate::scope::{self, Narrowing, Patterns, Scope, Strategy};

/// A crawl as the Frontier sees it, in a run file in memory: what it
/// admitted, each URL with its crawl request's ID.
struct Crawl {
    connection: Connection,
    scope: Scope,
    ids: BTreeMap<String, String>,
    requests: Requests,
}

/// The crawl's requests as the Frontier's merges end them: those whose
/// page was fetched, and those a rule learned later ended `skipped` with
/// `duplicate`, in order.
#[derive(Default)]
struct Requests {
    urls: BTreeMap<String, String>,
    fetched: BTreeSet<String>,
    skipped: Vec<String>,
}

impl EndMerged for Requests {
    /// Ends every request of `crawl_requests` neither fetched nor ended,
    /// keeping the first when none was fetched.
    fn end_merged(
        &mut self,
        _: &Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<String>> {
        let mut pending: Vec<&String> = crawl_requests
            .iter()
            .filter(|id| !self.fetched.contains(*id) && !self.skipped.contains(&self.urls[*id]))
            .collect();
        pending.sort_by_key(|id| id.parse::<u32>().unwrap());
        if !crawl_requests.iter().any(|id| self.fetched.contains(id)) && !pending.is_empty() {
            pending.remove(0);
        }
        let ended: Vec<String> = pending.iter().map(|id| self.urls[*id].clone()).collect();
        self.skipped.extend(ended.iter().cloned());
        Ok(ended)
    }
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
            requests: Requests::default(),
        }
    }

    fn admit(&mut self, candidate: Candidate<'_>) -> Enqueued {
        let next = (self.ids.len() + 1).to_string();
        let every_host = Policy::new(&["*"], &[] as &[&str]).unwrap();
        let answer = admit(
            &self.connection,
            &self.scope,
            &every_host,
            candidate,
            |_| Ok(next.clone()),
        )
        .unwrap();
        if let Enqueued::Added { id, url } = &answer {
            self.ids.insert(url.clone(), id.clone());
            self.requests.urls.insert(id.clone(), url.clone());
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
    fn fetched(&mut self, url: &str, text: &str) {
        self.fetched_naming(url, text, None);
    }

    /// The same, the page naming `canonical` as its `rel=canonical`.
    fn fetched_naming(&mut self, url: &str, text: &str, canonical: Option<&str>) {
        let crawl_request = self.ids[url].clone();
        self.requests.fetched.insert(crawl_request.clone());
        let canonical = canonical.map(|canonical| Url::parse(canonical).unwrap());
        learn(
            &self.connection,
            PageSeen {
                crawl_request: &crawl_request,
                url: &Url::parse(url).unwrap(),
                method: get().method,
                body: None,
                signature: Sha256::digest(text).into(),
                canonical: canonical.as_ref(),
            },
            &mut self.requests,
        )
        .unwrap();
    }

    /// A request's fetch was redirected from `from` to `to`.
    fn redirected(&mut self, from: &str, to: &str) {
        learn_redirect(
            &self.connection,
            Redirect {
                from: &Url::parse(from).unwrap(),
                to: &Url::parse(to).unwrap(),
                method: get().method,
                body: None,
            },
            &mut self.requests,
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
        keeps_fragment: false,
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
                form: None,
                evidence: format!(
                    "3 pairs of URLs differing only by `ref` served pages of matching signatures, \
                     and no pair pages of differing ones: {ITEM}?n=1 and {ITEM}?n=1&ref=a; \
                     {ITEM}?n=2 and {ITEM}?n=2&ref=b; {ITEM}?n=3 and {ITEM}?n=3&ref=c."
                ),
                contradiction: None,
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
                form: None,
                evidence: format!(
                    "100 pages under `/calendar/{{number}}/{{number}}` in a row, from {} to {}, \
                     brought no new page signature and no URL outside it.",
                    month(1),
                    month(100)
                ),
                contradiction: None,
                merged: vec![month(101)],
            }],
            vec![CompletenessReason {
                completeness: Completeness::Unverified,
                reason: AxisReason {
                    code: "trap_path".to_owned(),
                    message:
                        "A learned trap refused 1 URL the crawl would otherwise have admitted."
                            .to_owned(),
                    count: Some(1),
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
            count: Some(1),
            call_id: None,
        },
    }]
}

/// The Completeness reason of `refused` offset URLs a learned cursor
/// refused.
fn cursor_reason(refused: u64, message: &str) -> Vec<CompletenessReason> {
    vec![CompletenessReason {
        completeness: Completeness::Unverified,
        reason: AxisReason {
            code: "cursor_pagination".to_owned(),
            message: message.to_owned(),
            count: Some(refused),
            call_id: None,
        },
    }]
}

/// An offset parameter, whose values step by one page size on one path,
/// gives way to a cursor its pages link: a parameter that two pages of
/// the listing each link with a value of their own. The offset URLs not
/// yet fetched are then not added, with `duplicate`, the report lists the
/// rule with its evidence and what it refused, and the Run is `unverified`
/// by the count of those URLs.
///
/// Covers: story:6-216, story:6-310
#[test]
fn a_cursor_found_on_an_offset_listing_is_preferred() {
    let cursors = [format!("{LIST}?after=r-40"), format!("{LIST}?after=r-60")];
    let mut crawl = Crawl::new();
    crawl_listing(&mut crawl, 3, 20, None);
    assert_eq!(crawl.enqueue(&at(1, 20), &cursors[0]), "added");
    let one_page = crawl.rules();
    assert_eq!(crawl.enqueue(&at(2, 20), &cursors[1]), "added");

    let next = next_offset(&mut crawl, 3, 20);

    assert_eq!(
        (one_page, next, crawl.rules(), crawl.completeness()),
        (
            Vec::new(),
            "duplicate".to_owned(),
            vec![LearnedRule {
                kind: LearnedRuleKind::CursorPagination,
                host: "table.test".to_owned(),
                path: Some("/list".to_owned()),
                parameter: Some("offset".to_owned()),
                form: None,
                evidence: format!(
                    "`offset` steps by 20 on {LIST} ({LIST}?offset=0, {LIST}?offset=20, \
                     {LIST}?offset=40), and two of its pages each link a value of `after` of \
                     their own: {LIST}?offset=20 links {}, and {LIST}?offset=40 links {}.",
                    cursors[0], cursors[1]
                ),
                contradiction: None,
                merged: vec![format!("{LIST}?offset=60")],
            }],
            cursor_reason(
                1,
                "A learned cursor refused 1 offset URL of a listing the crawl pages by its \
                 cursor instead."
            ),
        )
    );
}

/// A parameter a listing's pages link is a cursor only on evidence: a
/// sort that every page links alike, first met once the offset is known,
/// teaches nothing, and neither does a filter one page links with two
/// values, nor a view two pages link with one value. Each listing is paged
/// on by its offset, whose next page is added.
///
/// Covers: story:6-216
#[rstest::rstest]
#[case::a_sort_every_page_links_alike(&[(0, "sort=price"), (1, "sort=price")])]
#[case::a_filter_one_page_links_with_two_values(
    &[(0, "color=red"), (0, "color=blue"), (1, "color=green")]
)]
#[case::a_view_two_pages_link_with_one_value(&[(0, "view=grid"), (1, "lang=fr&view=grid")])]
fn a_parameter_that_does_not_change_from_page_to_page_is_no_cursor(#[case] links: &[(u32, &str)]) {
    let mut crawl = Crawl::new();
    crawl.seed(&at(0, 20));
    crawl.fetched(&at(0, 20), "Rows 0");
    // The listing's first page links its next two, as a pagination bar
    // does, so its offset is known before any other link of it is met.
    for page in [1, 2] {
        assert_eq!(crawl.enqueue(&at(0, 20), &at(page, 20)), "added");
    }
    crawl.fetched(&at(1, 20), "Rows 1");
    for (page, query) in links {
        crawl.enqueue(&at(*page, 20), &format!("{LIST}?{query}"));
    }

    let next = crawl.enqueue(&at(1, 20), &at(3, 20));

    assert_eq!(
        (next, crawl.rules(), crawl.completeness()),
        ("added".to_owned(), Vec::new(), offset_reason())
    );
}

/// With `frontier.learning` off, admission keeps no evidence of an offset
/// or of a cursor and no rule refuses a URL: the listing above is paged on
/// by its offset, the report lists no rule and no reason, and the learned
/// rules' tables hold no row.
///
/// Covers: story:6-216, setting:frontier.learning
#[test]
fn with_learning_off_admission_keeps_no_evidence_and_refuses_nothing() {
    let mut crawl = Crawl::new();
    Bounds {
        learning: false,
        ..Bounds::default()
    }
    .record(&crawl.connection)
    .unwrap();
    // No page is signed with learning off, so none is handed to `learn`.
    crawl.seed(&at(0, 20));
    for page in 0..2 {
        assert_eq!(crawl.enqueue(&at(page, 20), &at(page + 1, 20)), "added");
    }
    crawl.enqueue(&at(1, 20), &format!("{LIST}?after=r-40"));
    crawl.enqueue(&at(2, 20), &format!("{LIST}?after=r-60"));

    let next = next_offset(&mut crawl, 3, 20);

    let rows: Vec<(String, i64)> = rules::TABLES
        .statements
        .iter()
        .filter_map(|statement| statement.strip_prefix("CREATE TABLE IF NOT EXISTS "))
        .map(|rest| rest.split_whitespace().next().unwrap().to_owned())
        .map(|table| {
            let count = crawl
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            (table, count)
        })
        .filter(|(_, count)| *count != 0)
        .collect();
    assert_eq!(
        (next, crawl.rules(), crawl.completeness(), rows),
        ("added".to_owned(), Vec::new(), Vec::new(), Vec::new())
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

const SITE: &str = "http://site.test";

fn at_site(path: &str) -> String {
    format!("{SITE}{path}")
}

fn form_rule(form: EquivalentForm, evidence: String, merged: &[&str]) -> LearnedRule {
    LearnedRule {
        kind: LearnedRuleKind::EquivalentForm,
        host: "site.test".to_owned(),
        path: None,
        parameter: None,
        form: Some(form),
        evidence,
        contradiction: None,
        merged: strings(merged),
    }
}

/// Fetches `path` and `path/`, both reading `text`: a pair of URLs
/// differing only by a trailing slash.
fn slash_pair(crawl: &mut Crawl, path: &str, text: &str) {
    for url in [at_site(path), at_site(&format!("{path}/"))] {
        if !crawl.ids.contains_key(&url) {
            assert_eq!(crawl.enqueue(&at_site("/"), &url), "added");
        }
        crawl.fetched(&url, text);
    }
}

/// Two pairs of URLs differing only by a trailing slash, matching, teach
/// that form on the host, and one pair does not: a URL that differs from
/// an admitted one only by its slash is then a `duplicate`, and of two
/// requests admitted before, both pending, the later ends `skipped` with
/// `duplicate`, listed with the rule.
///
/// Covers: story:6-217, story:6-222, story:6-310
#[test]
fn two_matching_pairs_teach_a_form_and_one_does_not() {
    let mut crawl = Crawl::new();
    crawl.seed(&at_site("/"));
    crawl.fetched(&at_site("/"), "Home");
    slash_pair(&mut crawl, "/a1", "Page 1");
    let one_short = [
        crawl.enqueue(&at_site("/"), &at_site("/c")),
        crawl.enqueue(&at_site("/"), &at_site("/c/")),
    ];
    slash_pair(&mut crawl, "/a2", "Page 2");

    let answers = [
        crawl.enqueue(&at_site("/"), &at_site("/a3")),
        crawl.enqueue(&at_site("/"), &at_site("/a3/")),
        crawl.enqueue(&at_site("/"), &at_site("/a1")),
    ];

    assert_eq!(
        (
            one_short.to_vec(),
            answers.to_vec(),
            crawl.rules(),
            crawl.requests.skipped.clone()
        ),
        (
            strings(&["added", "added"]),
            strings(&["added", "duplicate", "duplicate"]),
            vec![form_rule(
                EquivalentForm::TrailingSlash,
                format!(
                    "2 pairs of URLs differing only by a trailing slash served pages of matching \
                     signatures, and no pair pages of differing ones: {SITE}/a1 and {SITE}/a1/; \
                     {SITE}/a2 and {SITE}/a2/."
                ),
                &[&at_site("/c/"), &at_site("/a3/")],
            )],
            strings(&[&at_site("/c/")]),
        )
    );
}

/// One redirect from one form of a URL to the other teaches the form at
/// once, for `www.` and the scheme alike; a redirect to another path
/// teaches nothing.
///
/// Covers: story:6-217
#[test]
fn a_redirect_between_two_forms_teaches_the_form() {
    let mut crawl = Crawl::new();
    crawl.seed("http://www.site.test/a");
    crawl.seed(&at_site("/old"));
    crawl.redirected("http://www.site.test/a", "https://www.site.test/a");
    crawl.redirected("https://www.site.test/a", "https://site.test/a");
    crawl.redirected(&at_site("/old"), &at_site("/new"));

    let answers = [
        crawl.enqueue(&at_site("/"), "https://www.site.test/a"),
        crawl.enqueue(&at_site("/"), "https://site.test/a"),
        crawl.enqueue(&at_site("/"), "http://www.site.test/b"),
        crawl.enqueue(&at_site("/"), "https://site.test/b"),
        crawl.enqueue(&at_site("/"), &at_site("/new")),
    ];

    assert_eq!(
        (answers.to_vec(), crawl.rules()),
        (
            strings(&["duplicate", "duplicate", "added", "duplicate", "added"]),
            vec![
                LearnedRule {
                    host: "www.site.test".to_owned(),
                    ..form_rule(
                        EquivalentForm::Scheme,
                        "http://www.site.test/a redirected to https://www.site.test/a, which \
                         differs from it only by the scheme."
                            .to_owned(),
                        &["https://site.test/b"],
                    )
                },
                form_rule(
                    EquivalentForm::Www,
                    "https://www.site.test/a redirected to https://site.test/a, which differs \
                     from it only by `www.`."
                        .to_owned(),
                    &["https://www.site.test/a", "https://site.test/a"],
                ),
            ],
        )
    );
}

/// Three pages whose `rel=canonical` differs from their URL only by an
/// index file teach that form, and two do not: the folder of a page
/// fetched then is a `duplicate`, and one admitted before and pending
/// ends `skipped` with `duplicate`.
///
/// Covers: story:6-217, story:6-219, story:6-222
#[test]
fn three_canonicals_teach_a_form_and_two_do_not() {
    let mut crawl = Crawl::new();
    crawl.seed(&at_site("/"));
    let page = |number: u32| at_site(&format!("/d{number}/index.html"));
    let folder = |number: u32| at_site(&format!("/d{number}/"));
    for number in 1..=2 {
        crawl.enqueue(&at_site("/"), &page(number));
        crawl.fetched_naming(&page(number), "Docs", Some(&folder(number)));
    }
    let one_short = crawl.enqueue(&at_site("/"), &folder(1));
    crawl.enqueue(&at_site("/"), &page(3));
    crawl.fetched_naming(&page(3), "Docs", Some(&folder(3)));

    let answer = crawl.enqueue(&at_site("/"), &folder(2));

    assert_eq!(
        (
            one_short,
            answer,
            crawl.rules(),
            crawl.requests.skipped.clone()
        ),
        (
            "added".to_owned(),
            "duplicate".to_owned(),
            vec![form_rule(
                EquivalentForm::IndexHtml,
                format!(
                    "3 pages named as their `rel=canonical` their own URL but for `index.html`: \
                     {} names {}; {} names {}; {} names {}.",
                    page(1),
                    folder(1),
                    page(2),
                    folder(2),
                    page(3),
                    folder(3)
                ),
                &[&folder(1), &folder(2)],
            )],
            strings(&[&folder(1)]),
        )
    );
}

/// A `rel=canonical` to another path, such as a templated site's home page
/// on every page, or to another registrable domain, is no evidence: the
/// site keeps every page.
///
/// Covers: story:6-219
#[test]
fn a_canonical_to_another_path_or_domain_is_ignored() {
    let mut crawl = Crawl::new();
    crawl.seed(&at_site("/"));
    for number in 1..=4 {
        let url = at_site(&format!("/p{number}"));
        crawl.enqueue(&at_site("/"), &url);
        crawl.fetched_naming(&url, &format!("Page {number}"), Some(&at_site("/")));
        let elsewhere = at_site(&format!("/q{number}"));
        crawl.enqueue(&at_site("/"), &elsewhere);
        crawl.fetched_naming(
            &elsewhere,
            &format!("Other {number}"),
            Some(&format!("http://www.mirror.test/q{number}")),
        );
    }

    let answers = [
        crawl.enqueue(&at_site("/"), &at_site("/p5")),
        crawl.enqueue(&at_site("/"), &at_site("/p1/")),
        crawl.enqueue(&at_site("/"), &at_site("/q1/")),
    ];

    assert_eq!(
        (answers.to_vec(), crawl.rules()),
        (strings(&["added", "added", "added"]), Vec::new())
    );
}

/// On a host whose `www.` and bare pages differ, a pair of them keeps the
/// form from being learned, whatever evidence follows: both are fetched
/// and kept.
///
/// Covers: story:6-217
#[test]
fn a_site_whose_forms_differ_keeps_both() {
    let mut crawl = Crawl::new();
    for path in ["/a", "/b"] {
        let bare = at_site(path);
        let www = format!("http://www.site.test{path}");
        crawl.seed(&bare);
        crawl.seed(&www);
        crawl.fetched(&bare, &format!("Bare {path}"));
        crawl.fetched(&www, &format!("Www {path}"));
    }
    crawl.redirected("http://www.site.test/c", &at_site("/c"));

    let answers = [
        crawl.enqueue(&at_site("/"), &at_site("/d")),
        crawl.enqueue(&at_site("/"), "http://www.site.test/d"),
    ];

    assert_eq!(
        (answers.to_vec(), crawl.rules()),
        (strings(&["added", "added"]), Vec::new())
    );
}

/// A learned form that meets a contradicting pair, both its URLs fetched
/// with no redirect and their pages differing, stops: it merges no URL
/// from then on, and the report gives the pair beside the URLs it had
/// merged.
///
/// Covers: story:6-218, story:6-310
#[test]
fn a_contradicting_pair_stops_a_learned_form() {
    let mut crawl = Crawl::new();
    crawl.seed(&at_site("/"));
    crawl.fetched(&at_site("/"), "Home");
    crawl.enqueue(&at_site("/"), &at_site("/c"));
    crawl.enqueue(&at_site("/"), &at_site("/c/"));
    crawl.fetched(&at_site("/c"), "A page");
    let in_flight = crawl.ids[&at_site("/c/")].clone();
    crawl.requests.fetched.insert(in_flight);
    slash_pair(&mut crawl, "/a1", "Page 1");
    slash_pair(&mut crawl, "/a2", "Page 2");
    crawl.enqueue(&at_site("/"), &at_site("/a3"));
    let merged = crawl.enqueue(&at_site("/"), &at_site("/a3/"));

    crawl.fetched(&at_site("/c/"), "A list of pages");
    let answers = [
        crawl.enqueue(&at_site("/"), &at_site("/a4")),
        crawl.enqueue(&at_site("/"), &at_site("/a4/")),
        crawl.enqueue(&at_site("/"), &at_site("/a3/")),
    ];

    assert_eq!(
        (merged, answers.to_vec(), crawl.rules()),
        (
            "duplicate".to_owned(),
            strings(&["added", "added", "added"]),
            vec![LearnedRule {
                contradiction: Some(format!(
                    "{SITE}/c and {SITE}/c/, both fetched and neither redirected, served pages \
                     of differing signatures."
                )),
                ..form_rule(
                    EquivalentForm::TrailingSlash,
                    format!(
                        "2 pairs of URLs differing only by a trailing slash served pages of \
                         matching signatures, and no pair pages of differing ones: {SITE}/a1 and \
                         {SITE}/a1/; {SITE}/a2 and {SITE}/a2/."
                    ),
                    &[&at_site("/a3/")],
                )
            }],
        )
    );
}

/// A learned parameter stops the same way once a pair of pages differing
/// only by it differs.
///
/// Covers: story:6-218
#[test]
fn a_contradicting_pair_stops_a_learned_parameter() {
    let mut crawl = Crawl::new();
    crawl.seed(&format!("{ITEM}?n=0"));
    crawl.enqueue(ITEM, &format!("{ITEM}?n=7"));
    crawl.enqueue(ITEM, &format!("{ITEM}?n=7&ref=x"));
    crawl.fetched(&format!("{ITEM}?n=7"), "Item 7");
    let in_flight = crawl.ids[&format!("{ITEM}?n=7&ref=x")].clone();
    crawl.requests.fetched.insert(in_flight);
    for (number, reference) in [(1, "a"), (2, "b"), (3, "c")] {
        pair(&mut crawl, number, reference, None);
    }
    let merged = crawl.enqueue(ITEM, &format!("{ITEM}?n=1&ref=z"));

    crawl.fetched(&format!("{ITEM}?n=7&ref=x"), "Item 7, as x sees it");
    let after = crawl.enqueue(ITEM, &format!("{ITEM}?n=2&ref=z"));

    let rule = crawl.rules().remove(0);
    assert_eq!(
        (merged, after, rule.contradiction, rule.merged),
        (
            "duplicate".to_owned(),
            "added".to_owned(),
            Some(format!(
                "{ITEM}?n=7 and {ITEM}?n=7&ref=x, both fetched and neither redirected, served \
                 pages of differing signatures."
            )),
            strings(&[&format!("{ITEM}?n=1&ref=z")]),
        )
    );
}

thread_local! {
    /// The statements the traced connection of this thread ran, each with
    /// its parameters' values.
    static RAN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record(event: rusqlite::trace::TraceEvent<'_>) {
    if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event {
        let sql = statement
            .expanded_sql()
            .unwrap_or_else(|| statement.sql().into_owned());
        RAN.with_borrow_mut(|ran| ran.push(sql));
    }
}

impl Crawl {
    /// Each statement `act` runs on the crawl's run file, in order, with
    /// its parameters' values.
    fn statements_of(&mut self, act: impl FnOnce(&mut Self)) -> Vec<String> {
        RAN.with_borrow_mut(Vec::clear);
        self.connection.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(record),
        );
        act(self);
        self.connection
            .trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
        RAN.with_borrow_mut(std::mem::take)
    }

    /// Each step of the query plans of `statements` that scans one of
    /// `tables`, with its statement.
    fn scans_of(&self, statements: &[String], tables: &[&str]) -> Vec<String> {
        let mut scans = Vec::new();
        for statement in statements.iter().filter(|statement| {
            !statement.starts_with("--") && tables.iter().any(|table| statement.contains(table))
        }) {
            let mut plan = self
                .connection
                .prepare(&format!("EXPLAIN QUERY PLAN {statement}"))
                .unwrap();
            let steps: Vec<String> = plan
                .query_map([], |row| row.get(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            scans.extend(
                steps
                    .into_iter()
                    .filter(|step| {
                        tables
                            .iter()
                            .any(|table| step.starts_with(&format!("SCAN {table}")))
                    })
                    .map(|step| format!("{step}: {statement}")),
            );
        }
        scans
    }
}

/// What learning and then contradicting `ref` on `shop.test` ran, once
/// the host holds `rows` more visited requests carrying `ref` beside the
/// pairs, and other hosts hold requests of their own, one naming
/// `shop.test` in its query: the crawl, the statements of the learning,
/// then those of the contradiction.
fn rekeying(rows: u32) -> (Crawl, Vec<String>, Vec<String>) {
    let mut crawl = Crawl::new();
    crawl.seed(&format!("{ITEM}?n=0"));
    crawl.seed("http://other.test/item?n=1&ref=a");
    crawl.seed("http://elsewhere.test/go?to=http://shop.test/item&ref=a");
    crawl.enqueue(ITEM, &format!("{ITEM}?n=7"));
    crawl.enqueue(ITEM, &format!("{ITEM}?n=7&ref=x"));
    crawl.fetched(&format!("{ITEM}?n=7"), "Item 7");
    let in_flight = crawl.ids[&format!("{ITEM}?n=7&ref=x")].clone();
    crawl.requests.fetched.insert(in_flight);
    for row in 0..rows {
        crawl.enqueue(ITEM, &format!("{ITEM}?n={}&ref=r", 100 + row));
    }
    pair(&mut crawl, 1, "a", None);
    pair(&mut crawl, 2, "b", None);
    let learning = crawl.statements_of(|crawl| pair(crawl, 3, "c", None));
    let contradicting = crawl.statements_of(|crawl| {
        crawl.fetched(&format!("{ITEM}?n=7&ref=x"), "Item 7, as x sees it");
    });
    (crawl, learning, contradicting)
}

/// How many of `statements` read the learned rules.
fn rule_reads(statements: &[String]) -> usize {
    statements
        .iter()
        .filter(|statement| statement.contains("FROM learned_rules"))
        .count()
}

/// A rule learned, then contradicted, re-keys its host's visited
/// identities through reads bounded to that host's rows, never a scan of
/// the visited set or of the learned identities, and reads the host's
/// rules once per event: a host holding ten times the rows reads the
/// learned rules as often, and the rule merges the same.
///
/// Covers: story:6-216, story:6-218
#[test]
fn a_learned_or_contradicted_rule_rekeys_its_host_through_bounded_reads() {
    let (few, few_learning, few_contradicting) = rekeying(3);
    let (many, many_learning, many_contradicting) = rekeying(30);

    let tables = ["visited", "learned_identities"];
    assert_eq!(
        (
            many.scans_of(&many_learning, &tables),
            many.scans_of(&many_contradicting, &tables),
            rule_reads(&many_learning),
            rule_reads(&many_contradicting),
            many.rules().remove(0).merged,
        ),
        (
            Vec::<String>::new(),
            Vec::<String>::new(),
            rule_reads(&few_learning),
            rule_reads(&few_contradicting),
            few.rules().remove(0).merged,
        )
    );
}
