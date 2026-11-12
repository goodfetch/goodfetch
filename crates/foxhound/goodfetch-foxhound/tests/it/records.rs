//! Run conformance cases of `run.records` on Foxhound's executable over
//! stdio (spec #581's seam: the protocol; ADRs 0008 and 0103): a reader
//! reads a Dataset's committed Records from the first, each in a `$/record`
//! notification under the credits it grants, follows the Run live and
//! answers once the Run ended and every Record is read; `after` resumes a
//! reader from the last number it handled, so a reader that disconnects
//! and reconnects sees each Record once; and a reader that grants no
//! credit never holds the crawl back, which commits and ends without it;
//! `through` bounds a reader of a live Run, which answers once it read
//! through that number.
//! Each crawl's handler answers with one Record of `pages` per request,
//! so no call of its own interleaves with the reader's notifications.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::{BTreeMap, VecDeque};

use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::site::Site;

/// An Attached engine and what it sent, sorted: each reader's Records by
/// its call's id, each reply by its id, and the crawl requests not yet
/// answered.
struct Session {
    engine: Attached,
    records: BTreeMap<String, Vec<Value>>,
    replies: BTreeMap<String, Value>,
    requests: VecDeque<Value>,
}

impl Session {
    fn new(engine: Attached) -> Self {
        Self {
            engine,
            records: BTreeMap::new(),
            replies: BTreeMap::new(),
            requests: VecDeque::new(),
        }
    }

    /// Sends the call `id`.
    fn send(&mut self, id: &str, method: &str, params: &Value) {
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    /// Starts a crawl `run_id` of `pages` seeds declaring `pages`, one
    /// request at a time, and waits for its start's answer.
    fn crawl(&mut self, site: &Site, run_id: &str, pages: usize) {
        let seeds: Vec<Value> = (1..=pages)
            .map(|page| json!({ "url": site.page(&format!("/{page}")), "label": "page" }))
            .collect();
        self.send(
            "start",
            "crawl.start",
            &json!({
                "run_id": run_id,
                "seeds": seeds,
                "datasets": [{ "name": "pages" }],
                "settings": { "crawl": { "handler": { "concurrency": 1 } } },
            }),
        );
        self.until(|session| session.replies.contains_key("start"));
        assert_eq!(
            self.replies["start"]["result"]["run_id"],
            json!(run_id),
            "{}",
            self.replies["start"]
        );
    }

    /// Opens the reader `id` of `pages` on `run_id`, after `after` when
    /// given.
    fn read(&mut self, id: &str, run_id: &str, after: Option<u64>) {
        let mut params = json!({ "run_id": run_id, "dataset": "pages" });
        if let Some(after) = after {
            params["after"] = json!(after);
        }
        self.send(id, "run.records", &params);
    }

    /// Grants the reader `id` `credits` more Records.
    fn credit(&mut self, id: &str, credits: u32) {
        self.engine.send(&json!({
            "jsonrpc": "2.0", "method": "$/credit", "params": { "id": id, "credits": credits },
        }));
    }

    /// Cancels the call `id`.
    fn cancel(&mut self, id: &str) {
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": id } }));
    }

    /// Reads the engine's messages until `done` holds.
    fn until(&mut self, done: impl Fn(&Self) -> bool) {
        while !done(self) {
            let message = self.engine.next().expect("the engine closed");
            if message["method"] == "crawl_request.handle" {
                self.requests.push_back(message);
            } else if message["method"] == "$/record" {
                let id = message["params"]["id"].as_str().unwrap().to_owned();
                self.records
                    .entry(id)
                    .or_default()
                    .push(message["params"]["record"].clone());
            } else if let Some(id) = message["id"].as_str() {
                self.replies.insert(id.to_owned(), message);
            }
        }
    }

    /// Waits for the next crawl request and answers it with one Record of
    /// `pages` naming its URL.
    fn answer_next(&mut self) {
        self.until(|session| !session.requests.is_empty());
        let request = self.requests.pop_front().unwrap();
        let url = request["params"]["crawl_request"]["url"].clone();
        self.engine.send(&json!({
            "jsonrpc": "2.0", "id": request["id"],
            "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] },
        }));
    }

    /// How many Records the reader `id` has read.
    fn read_by(&self, id: &str) -> usize {
        self.records.get(id).map_or(0, Vec::len)
    }

    /// `run.finish` of `run_id`, once every reader in `readers` answered
    /// too.
    fn finish(&mut self, run_id: &str, readers: &[&str]) -> Value {
        self.send("finish", "run.finish", &json!({ "run_id": run_id }));
        self.until(|session| {
            session.replies.contains_key("finish")
                && readers.iter().all(|id| session.replies.contains_key(*id))
        });
        self.replies["finish"].clone()
    }

    /// The Records the reader `id` read, each with its committed time
    /// checked as RFC 3339 and left out, and its Provenance, which
    /// `provenance.rs`'s cases read, left out.
    fn records_of(&self, id: &str) -> Vec<Value> {
        self.records
            .get(id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut record| {
                let committed_at = record
                    .as_object_mut()
                    .unwrap()
                    .remove("committed_at")
                    .unwrap();
                record.as_object_mut().unwrap().remove("provenance");
                assert!(
                    committed_at.as_str().is_some_and(|at| at.ends_with('Z')),
                    "{committed_at}"
                );
                record
            })
            .collect()
    }
}

/// The Record of `pages` the crawl request `n` committed as number
/// `number`, as a reader reads it, its committed time and Provenance left
/// out.
fn page(site: &Site, number: u64, n: u64) -> Value {
    let url = site.page(&format!("/{n}"));
    json!({ "number": number, "value": { "url": url }, "request_id": n.to_string(), "url": url })
}

/// A reader opened once a Record has committed reads it all the same:
/// it starts at the Dataset's first Record. It then follows the live Run,
/// sent each later Record as it commits, before the Run ends, and answers
/// once the Run ended and every Record is read.
///
/// Covers: story:6-271, story:6-268, operation:run.records
#[test]
fn a_reader_reads_from_the_first_record_and_follows_the_live_run_to_its_end() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut session = Session::new(site.engine(&env));

    session.crawl(&site, "followed", 3);
    session.answer_next();
    session.until(|session| !session.requests.is_empty());
    session.read("reader", "followed", None);
    session.credit("reader", 10);
    session.until(|session| session.read_by("reader") == 1);
    let first = session.records_of("reader");
    session.answer_next();
    session.until(|session| session.read_by("reader") == 2);
    let answered_while_live = session.replies.contains_key("reader");
    session.answer_next();
    let finished = session.finish("followed", &["reader"]);
    let read = session.records_of("reader");
    let reply = session.replies["reader"]["result"].clone();
    assert!(session.engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            first,
            answered_while_live,
            read,
            reply
        ),
        (
            &json!("finished"),
            vec![page(&site, 1, 1)],
            false,
            vec![page(&site, 1, 1), page(&site, 2, 2), page(&site, 3, 3)],
            json!({ "count": 3, "last": 3 }),
        )
    );
}

/// A reader that disconnects after its first Record and reconnects with
/// `after` set to that Record's number, while the crawl goes on, reads
/// every Record once; a reader on another engine, once the Run ended,
/// resumes after a number and reads to the last Record.
///
/// Covers: story:6-271, operation:run.records
#[test]
fn a_reader_that_reconnects_after_a_number_reads_each_record_once() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut session = Session::new(site.engine(&env));

    session.crawl(&site, "resumed", 4);
    session.read("first", "resumed", None);
    session.credit("first", 1);
    session.answer_next();
    session.answer_next();
    session.until(|session| session.read_by("first") == 1);
    let last_read = session.records_of("first")[0]["number"].as_u64().unwrap();
    session.cancel("first");
    session.until(|session| session.replies.contains_key("first"));
    session.read("again", "resumed", Some(last_read));
    session.credit("again", 10);
    session.answer_next();
    session.answer_next();
    let finished = session.finish("resumed", &["again"]);
    let mut read = session.records_of("first");
    read.extend(session.records_of("again"));
    let again = session.replies["again"]["result"].clone();
    assert!(session.engine.finished());

    let mut later = Session::new(site.engine(&env));
    later.read("ended", "resumed", Some(2));
    later.credit("ended", 10);
    later.until(|session| session.replies.contains_key("ended"));
    let ended = (
        later.records_of("ended"),
        later.replies["ended"]["result"].clone(),
    );
    assert!(later.engine.finished());

    assert_eq!(
        (&finished["result"]["lifecycle"], read, again, ended),
        (
            &json!("finished"),
            vec![
                page(&site, 1, 1),
                page(&site, 2, 2),
                page(&site, 3, 3),
                page(&site, 4, 4)
            ],
            json!({ "count": 3, "last": 4 }),
            (
                vec![page(&site, 3, 3), page(&site, 4, 4)],
                json!({ "count": 2, "last": 4 })
            ),
        )
    );
}

/// A reader given `through` on a live Run answers once it sent every
/// Record numbered through it, without waiting for the Run's end: through
/// what is committed at once, and through a number not committed yet once
/// that Record commits.
///
/// Covers: story:6-271, operation:run.records
#[test]
fn a_reader_given_through_answers_once_it_read_through_that_number_while_the_run_is_live() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut session = Session::new(site.engine(&env));

    session.crawl(&site, "bounded", 4);
    session.answer_next();
    session.answer_next();
    session.until(|session| !session.requests.is_empty());
    session.send(
        "committed",
        "run.records",
        &json!({ "run_id": "bounded", "dataset": "pages", "through": 2 }),
    );
    session.credit("committed", 10);
    session.until(|session| session.replies.contains_key("committed"));
    session.send(
        "ahead",
        "run.records",
        &json!({ "run_id": "bounded", "dataset": "pages", "after": 2, "through": 3 }),
    );
    session.credit("ahead", 10);
    session.answer_next();
    session.until(|session| session.replies.contains_key("ahead"));
    session.answer_next();
    let finished = session.finish("bounded", &[]);
    let read = (session.records_of("committed"), session.records_of("ahead"));
    let replies = (
        session.replies["committed"]["result"].clone(),
        session.replies["ahead"]["result"].clone(),
    );
    assert!(session.engine.finished());

    assert_eq!(
        (&finished["result"]["lifecycle"], read, replies),
        (
            &json!("finished"),
            (
                vec![page(&site, 1, 1), page(&site, 2, 2)],
                vec![page(&site, 3, 3)]
            ),
            (
                json!({ "count": 2, "last": 2 }),
                json!({ "count": 1, "last": 3 })
            ),
        )
    );
}

/// A reader that grants no credit holds nothing back: the crawl sends its
/// every request, commits every Record and ends while the reader has read
/// none; once it grants credits it reads them all and answers.
///
/// Covers: story:6-271, operation:run.records
#[test]
fn a_reader_that_grants_no_credit_never_holds_the_crawl_back() {
    const PAGES: u64 = 20;
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut session = Session::new(site.engine(&env));

    session.crawl(&site, "lagged", 20);
    session.read("stalled", "lagged", None);
    for _ in 0..PAGES {
        session.answer_next();
    }
    session.send("finish", "run.finish", &json!({ "run_id": "lagged" }));
    session.until(|session| session.replies.contains_key("finish"));
    let while_stalled = (
        session.replies["finish"]["result"]["lifecycle"].clone(),
        session.read_by("stalled"),
        session.replies.contains_key("stalled"),
    );
    session.credit("stalled", 100);
    session.until(|session| session.replies.contains_key("stalled"));
    let read = session.records_of("stalled");
    let reply = session.replies["stalled"]["result"].clone();
    assert!(session.engine.finished());

    assert_eq!(
        (while_stalled, read, reply),
        (
            (json!("finished"), 0, false),
            (1..=PAGES).map(|n| page(&site, n, n)).collect::<Vec<_>>(),
            json!({ "count": PAGES, "last": PAGES }),
        )
    );
}

/// A reader of a Dataset the crawl does not declare fails with
/// `dataset_unknown`, naming the Datasets it declares.
///
/// Covers: story:6-271, operation:run.records, error:dataset_unknown
#[test]
fn a_reader_of_a_dataset_the_crawl_does_not_declare_fails_with_dataset_unknown() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut session = Session::new(site.engine(&env));

    session.crawl(&site, "declared", 1);
    session.answer_next();
    session.finish("declared", &[]);
    session.send(
        "unknown",
        "run.records",
        &json!({ "run_id": "declared", "dataset": "books" }),
    );
    session.until(|session| session.replies.contains_key("unknown"));
    let error = session.replies["unknown"]["error"]["data"].clone();
    assert!(session.engine.finished());

    assert_eq!(
        (&error["code"], &error["dataset"], &error["declared"]),
        (
            &json!("dataset_unknown"),
            &json!("books"),
            &json!(["pages"])
        )
    );
}

/// A reader on another engine than the one whose writer works the Run, as
/// a reader on the Daemon of a Run an Attached engine crawls, follows it
/// live too: while that engine holds the Run's writer lock, it is sent
/// each Record as it commits, and it answers only once the Run ended and
/// every Record is read (ADRs 0102 and 0103).
///
/// Covers: story:6-271, operation:run.records
#[test]
fn a_reader_on_another_engine_follows_the_live_run_its_writer_works() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut writer = Session::new(site.engine(&env));
    let mut reader = Session::new(site.engine(&env));

    writer.crawl(&site, "elsewhere", 3);
    writer.answer_next();
    writer.until(|session| !session.requests.is_empty());
    reader.read("reader", "elsewhere", None);
    reader.credit("reader", 10);
    reader.until(|session| session.read_by("reader") == 1);
    writer.answer_next();
    reader
        .until(|session| session.read_by("reader") == 2 || session.replies.contains_key("reader"));
    let answered_while_live = reader.replies.contains_key("reader");
    writer.answer_next();
    let finished = writer.finish("elsewhere", &[]);
    reader.until(|session| session.replies.contains_key("reader"));
    let read = reader.records_of("reader");
    let reply = reader.replies["reader"]["result"].clone();
    assert!(writer.engine.finished());
    assert!(reader.engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            answered_while_live,
            read,
            reply
        ),
        (
            &json!("finished"),
            false,
            vec![page(&site, 1, 1), page(&site, 2, 2), page(&site, 3, 3)],
            json!({ "count": 3, "last": 3 }),
        )
    );
}
