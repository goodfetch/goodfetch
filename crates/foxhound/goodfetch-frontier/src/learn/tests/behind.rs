//! Learning's evidence written behind (#2169): with the Run learning
//! behind, a page's lesson and an admission's evidence read and write
//! nothing on the page's path, and the end of a batch teaches them in the
//! order noted, a failed job's evidence dropped; a rule learned there, one
//! batch late, merges the duplicates admitted before it.

use goodfetch_counted_work::Row;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_state::WriteWatch;
use goodfetch_protocol::foxhound::frontier::Enqueued;
use sha2::{Digest as _, Sha256};
use url::Url;

use std::sync::Arc;

use goodfetch_protocol::foxhound::frontier::HttpMethod;

use super::{CALENDAR, Crawl, ITEM, LIST, answer, get, month};
use crate::admit::{Candidate, admit, admit_in};
use crate::behind::Lesson;
use crate::identity::Declared;
use crate::invariants::Invariants;
use crate::learn::{PageSeen, Redirect};
use crate::scope::Narrowing;
use crate::staged::{Admitting, Staged};

/// A crawl whose Run learns behind, with the invariants its start built.
struct Behind {
    crawl: Crawl,
    invariants: Invariants,
}

impl Behind {
    fn new() -> Self {
        let crawl = Crawl::new();
        let invariants = crawl.invariants();
        invariants.count_on(crawl.work.clone());
        invariants.learn_behind();
        Self { crawl, invariants }
    }

    fn admit(&mut self, candidate: Candidate<'_>) -> String {
        let crawl = &mut self.crawl;
        let next = (crawl.ids.len() + 1).to_string();
        let every_host = Policy::new(&["*"], &[] as &[&str]).unwrap();
        let enqueued = admit(
            &crawl.connection,
            &self.invariants,
            &every_host,
            candidate,
            |_| Ok(next.clone()),
        )
        .unwrap();
        if let Enqueued::Added { id, url } = &enqueued {
            crawl.ids.insert(url.clone(), id.clone());
            crawl.requests.urls.insert(id.clone(), url.clone());
        }
        answer(&enqueued)
    }

    fn seed(&mut self, url: &str) -> String {
        self.admit(Candidate::Seed {
            url,
            declared: get(),
        })
    }

    fn enqueue(&mut self, from: &str, url: &str) -> String {
        let from = Url::parse(from).unwrap();
        self.admit(Candidate::Enqueue {
            url,
            base: &from,
            from: &from,
            declared: get(),
            narrowing: Narrowing::none(),
            depth: 1,
        })
    }

    /// The page at `url`, which the crawl admitted, fetched with the main
    /// text `text` after a redirect from `redirected` when given: its
    /// lesson noted.
    fn fetched(&mut self, url: &str, text: &str, redirected: Option<&str>) {
        let crawl_request = self.crawl.ids[url].clone();
        self.crawl.requests.fetched.insert(crawl_request.clone());
        let method = get().method;
        let mut lesson = Lesson::default();
        if let Some(from) = redirected {
            lesson.redirect(Redirect {
                from: &Url::parse(from).unwrap(),
                to: &Url::parse(url).unwrap(),
                method,
                body: None,
            });
        }
        lesson.page(PageSeen {
            crawl_request: &crawl_request,
            url: &Url::parse(url).unwrap(),
            method,
            body: None,
            signature: Sha256::digest(text).into(),
            canonical: None,
        });
        self.invariants.note(lesson);
    }

    /// The end of the writer's batch: what was noted is taught.
    fn batch_end(&mut self) {
        let crawl = &mut self.crawl;
        self.invariants
            .teach_behind(&crawl.connection, &mut crawl.requests, &crawl.work)
            .unwrap();
        assert!(!self.invariants.waiting());
    }

    fn watch(&self) -> std::sync::Arc<dyn WriteWatch> {
        self.invariants.watch().unwrap()
    }

    /// The path patterns' streaks, by pattern.
    fn streaks(connection: &rusqlite::Connection) -> Vec<(String, i64)> {
        connection
            .prepare("SELECT pattern, streak FROM rules_patterns ORDER BY pattern")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

/// Three pairs of URLs differing only by `ref` serve one page: noted
/// behind, they teach nothing until the batch's end, so two URLs differing
/// only by `ref` are both added before it, and no page read or wrote the
/// run file for learning. The batch's end learns `ref` and merges them:
/// the one not kept ends `skipped` with `duplicate`, and the next such URL
/// is a `duplicate`.
#[test]
fn a_rule_learned_a_batch_late_merges_the_duplicates_admitted_before_it() {
    let mut behind = Behind::new();
    behind.seed(&format!("{ITEM}?n=0"));
    for number in 1..=3 {
        let without = format!("{ITEM}?n={number}");
        let with = format!("{ITEM}?n={number}&ref=r{number}");
        assert_eq!(behind.enqueue(&format!("{ITEM}?n=0"), &without), "added");
        assert_eq!(behind.enqueue(&without, &with), "added");
        behind.fetched(&without, &format!("Item {number}"), None);
        behind.fetched(&with, &format!("Item {number}"), None);
    }
    let late = format!("{ITEM}?n=9&ref=late");
    assert_eq!(
        behind.enqueue(&format!("{ITEM}?n=0"), &format!("{ITEM}?n=9")),
        "added"
    );
    assert_eq!(behind.enqueue(&format!("{ITEM}?n=9"), &late), "added");
    assert_eq!(behind.crawl.rows(), std::collections::BTreeMap::new());
    assert_eq!(behind.crawl.rules(), Vec::new());
    assert_eq!(behind.crawl.work.read(Row::PageLearning), 0);
    assert!(behind.invariants.waiting());

    behind.batch_end();

    assert_eq!(behind.crawl.rules().len(), 1);
    assert_eq!(behind.crawl.requests.skipped, vec![late]);
    assert_eq!(
        behind.enqueue(&format!("{ITEM}?n=1"), &format!("{ITEM}?n=9&ref=other")),
        "duplicate"
    );
    assert_eq!(behind.crawl.work.read(Row::PageLearning), 0);
}

/// A redirect between two forms, noted with its page, teaches the form at
/// the batch's end, as it does on the page's path.
#[test]
fn a_redirect_noted_behind_teaches_its_form_at_the_batch_end() {
    let mut behind = Behind::new();
    behind.seed("https://site.test/a");
    behind.fetched("https://site.test/a", "A", Some("http://site.test/a"));
    assert_eq!(behind.crawl.rules(), Vec::new());

    behind.batch_end();

    assert_eq!(behind.crawl.rules().len(), 1);
    assert_eq!(behind.seed("http://site.test/a"), "duplicate");
}

/// The batch's end teaches the evidence in the order it was noted: a
/// page's run in its path pattern, which an admitted link outside the
/// pattern ends, reads as it does when taught on each page's path.
#[test]
fn the_batch_end_teaches_the_evidence_in_the_order_it_was_noted() {
    let mut behind = Behind::new();
    let mut inline = Crawl::new();
    let home = "http://events.test/";
    behind.seed(home);
    inline.seed(home);
    behind.enqueue(home, &month(0));
    inline.enqueue(home, &month(0));
    for index in 0..6 {
        behind.fetched(&month(index), "No events this month.", None);
        inline.fetched(&month(index), "No events this month.");
        if index == 3 {
            let outside = format!("{CALENDAR}-concerts/1");
            behind.enqueue(&month(index), &outside);
            inline.enqueue(&month(index), &outside);
        }
        behind.enqueue(&month(index), &month(index + 1));
        inline.enqueue(&month(index), &month(index + 1));
    }

    behind.batch_end();

    let streaks = Behind::streaks(&inline.connection);
    assert_eq!(streaks.iter().map(|(_, streak)| *streak).max(), Some(2));
    assert_eq!(Behind::streaks(&behind.crawl.connection), streaks);
    assert_eq!(behind.crawl.rows(), inline.rows());
}

/// An admission whose job fails takes its evidence back with it; a page's
/// lesson noted meanwhile, which no job wrote, stays. The same admission
/// in a job that commits teaches its offset.
#[rstest::rstest]
#[case::failed(true)]
#[case::committed(false)]
fn a_failed_job_drops_its_admissions_evidence_and_keeps_the_lessons(#[case] fails: bool) {
    let mut behind = Behind::new();
    let watch = behind.watch();
    watch.began();
    behind.seed(LIST);
    watch.began();
    assert_eq!(behind.enqueue(LIST, &format!("{LIST}?offset=10")), "added");
    behind.fetched(LIST, "Rows", None);
    if fails {
        watch.failed();
    }

    behind.batch_end();

    let rows = behind.crawl.rows();
    assert_eq!(rows.contains_key("rules_offsets"), !fails);
    assert_eq!(rows.get("rules_pages"), Some(&1));
}

/// An admission that stages, whose writes are rolled back, notes nothing:
/// its attempt's commit admits it again.
#[test]
fn a_staged_admission_notes_nothing() {
    let mut behind = Behind::new();
    behind.seed(LIST);
    behind.batch_end();
    let get = HttpMethod::get();
    let from = Url::parse(LIST).unwrap();
    let mut staging = Admitting::Staging {
        earlier: Arc::new(Staged::default()),
        now: Staged::default(),
    };
    let enqueued = admit_in(
        &behind.crawl.connection,
        &behind.invariants,
        &Policy::new(&["*"], &[] as &[&str]).unwrap(),
        Candidate::Enqueue {
            url: &format!("{LIST}?offset=10"),
            base: &from,
            from: &from,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
            narrowing: Narrowing::none(),
            depth: 1,
        },
        &mut staging,
        |_| Ok("staged".to_owned()),
    )
    .unwrap();

    assert_eq!(answer(&enqueued), "added");
    assert!(!behind.invariants.waiting());
}
