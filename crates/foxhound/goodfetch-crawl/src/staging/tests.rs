//! An attempt's staging in process, on a run file in memory (spec #581:
//! pure rules and run state in process): what a staged admission answers,
//! the ID it keeps through the commit, and the refusal inside a declared
//! Effect's body.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::egress::Policy;
use goodfetch_engine::staging::{AttemptWrites as _, StagedKey, StagedWrite};
use goodfetch_frontier::{
    BOUNDS_TABLES, Candidate, Declared, Narrowing, Patterns, SCOPE_TABLES, Scope, ScopeStrategy,
    VISITED_TABLES, document_base,
};
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use goodfetch_protocol::foxhound::outcomes::NotAddedAtCommit;
use rusqlite::Connection;
use serde_json::json;

use super::{Answered, Staging};
use crate::work_items::{self, Adding, Positions};

/// A run file in memory holding one seed, `https://shop.test/`, leased to
/// its first attempt, under lease epoch 1.
fn leased() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in work_items::TABLES
        .statements
        .iter()
        .chain(crate::labels::TABLES.statements)
        .chain(goodfetch_dataset::TABLES.statements)
        .chain(VISITED_TABLES.statements)
        .chain(SCOPE_TABLES.statements)
        .chain(BOUNDS_TABLES.statements)
        .chain(goodfetch_frontier::RULES_TABLES.statements)
        .chain(goodfetch_frontier::payload::TABLES.statements)
        .chain(goodfetch_frontier::sitemap::TABLES.statements)
        .chain(goodfetch_frontier::trap_links::TABLES.statements)
    {
        connection.execute(statement, []).unwrap();
    }
    let scope = Scope::new(
        ScopeStrategy::SameDomain,
        Patterns::default(),
        Patterns::default(),
    );
    scope.record(&connection).unwrap();
    let seed: Seed =
        serde_json::from_value(json!({ "url": "https://shop.test/", "label": "page" })).unwrap();
    work_items::admit_seeds(
        &connection,
        &mut Positions::of(&connection).unwrap(),
        &scope,
        &every_host(),
        &[seed],
    )
    .unwrap();
    let leased = work_items::lease(&connection, &["1".to_owned()]).unwrap();
    assert_eq!(leased[0].lease_epoch, 1);
    connection
}

fn every_host() -> Policy {
    Policy::new(&["*"], &[] as &[&str]).unwrap()
}

/// An admission of `url` from the seed's page, at depth 1.
fn enqueue(
    url: &'static str,
) -> impl Fn(&Connection, &mut Positions) -> Answered<Enqueued> + Send + Sync + 'static {
    move |connection, positions| {
        let from = document_base("https://shop.test/", None).unwrap();
        let method = HttpMethod::get();
        let candidate = Candidate::Enqueue {
            url,
            base: &from,
            from: &from,
            declared: Declared {
                method: &method,
                body: None,
                key: None,
                keeps_fragment: false,
            },
            narrowing: Narrowing::none(),
            depth: 1,
        };
        let scope = Scope::read(connection)?;
        let adding = Adding {
            method: &method,
            body: None,
            label: "page",
            payload: None,
            priority: 0,
            forefront: false,
            depth: 1,
        };
        work_items::admit(
            connection,
            positions,
            &scope,
            &every_host(),
            candidate,
            adding,
        )
        .map(|enqueued| Some(Ok(enqueued)))
    }
}

/// The URL of each work item the run file holds, by ID.
fn work_items_of(connection: &Connection) -> Vec<(String, String)> {
    connection
        .prepare("SELECT id, url FROM work_items ORDER BY position")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn answer(answered: Answered<Enqueued>) -> Enqueued {
    answered.unwrap().unwrap().unwrap()
}

/// A staged enqueue is answered as the Frontier will admit it, the
/// attempt's earlier enqueues counted, while the run file holds none of
/// them; the commit admits each at the ID its staging answered, though
/// another request was added meanwhile, and a discarded attempt adds none.
///
/// Covers: story:6-236
#[test]
fn a_staged_enqueue_is_answered_as_it_commits_and_keeps_its_id_through_the_commit() {
    let connection = leased();
    let staging = Staging::default();
    staging.open("1", 1);
    staging.open("1", 7);

    let answers = [
        answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b"))),
        answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b"))),
        answer(staging.admit(&connection, "1", 7, enqueue("https://shop.test/c"))),
    ];
    let staged = work_items_of(&connection);
    answer(enqueue("https://shop.test/d")(
        &connection,
        &mut Positions::of(&connection).unwrap(),
    ));
    staging.discard("1", 7);
    let writes = staging
        .take("1", 1)
        .admit(&connection)
        .unwrap()
        .writes
        .len();

    assert_eq!(
        (answers, staged, writes, work_items_of(&connection)),
        (
            [
                Enqueued::Added {
                    id: "2".to_owned(),
                    url: "https://shop.test/b".to_owned()
                },
                Enqueued::NotAdded {
                    reason: NotAddedReason::Duplicate,
                    url: "https://shop.test/b".to_owned()
                },
                Enqueued::Added {
                    id: "3".to_owned(),
                    url: "https://shop.test/c".to_owned()
                },
            ],
            vec![("1".to_owned(), "https://shop.test/".to_owned())],
            0,
            vec![
                ("1".to_owned(), "https://shop.test/".to_owned()),
                ("2".to_owned(), "https://shop.test/b".to_owned()),
                ("4".to_owned(), "https://shop.test/d".to_owned()),
            ],
        )
    );
}

/// While a declared Effect's body runs in an attempt, an enqueue and a
/// Key-value store write through it fail with `effect_staging_refused`
/// naming the fix, and stage nothing; once the body ended, they stage
/// again (ADR 0106). An attempt that ended refuses with `attempt_ended`.
///
/// Covers: story:6-237, error:effect_staging_refused, error:attempt_ended
#[test]
fn an_enqueue_or_a_write_inside_an_effects_body_fails_with_effect_staging_refused() {
    let connection = leased();
    let staging = Staging::default();
    staging.open("1", 1);
    let set = || StagedWrite::Set {
        key: "progress".to_owned(),
        value: json!(1),
    };

    assert!(staging.begin_effect("1", 1));
    let enqueued = staging
        .admit(&connection, "1", 1, enqueue("https://shop.test/b"))
        .unwrap()
        .unwrap()
        .unwrap_err();
    let written = staging.stage("1", 1, set()).unwrap_err();
    staging.end_effect("1", 1);
    let after = (
        answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b"))),
        staging.stage("1", 1, set()).unwrap(),
        staging.staged("1", 1, "progress").unwrap(),
    );
    staging.discard("1", 1);
    let ended = staging.stage("1", 1, set()).unwrap_err();

    assert_eq!(
        (
            enqueued.code(),
            enqueued
                .details()
                .and_then(|details| details.get("fix"))
                .is_some(),
            written.code.name,
            written.details.contains_key("fix"),
            after,
            ended.code.name,
            staging.begin_effect("1", 1),
        ),
        (
            "effect_staging_refused",
            true,
            "effect_staging_refused",
            true,
            (
                Enqueued::Added {
                    id: "2".to_owned(),
                    url: "https://shop.test/b".to_owned()
                },
                StagedKey::Untouched,
                StagedKey::Set(json!(1)),
            ),
            "attempt_ended",
            false,
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

/// What `act` answers, and each statement it ran on `connection`, in
/// order, with its parameters' values.
fn statements_of<T>(connection: &Connection, act: impl FnOnce() -> T) -> (T, Vec<String>) {
    RAN.with_borrow_mut(Vec::clear);
    connection.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
        Some(record),
    );
    let answered = act();
    connection.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    (answered, RAN.with_borrow_mut(std::mem::take))
}

/// Each step of the query plans of `statements` that scans `table`.
fn scans_of(connection: &Connection, statements: &[String], table: &str) -> Vec<String> {
    let scan = format!("SCAN {table}");
    let mut scans = Vec::new();
    for statement in statements
        .iter()
        .filter(|statement| !statement.starts_with("--") && statement.contains(table))
    {
        let mut plan = connection
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
                .filter(|step| step.starts_with(&scan))
                .map(|step| format!("{step}: {statement}")),
        );
    }
    scans
}

/// Fills the visited set with `count` identities of another host's
/// requests, as a crawl that admitted that many leaves it.
fn visited_elsewhere(connection: &Connection, count: u32) {
    connection
        .execute(
            "WITH RECURSIVE n (i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
             INSERT INTO visited (identity, crawl_request)
             SELECT 'GET https://elsewhere.test/item?id=' || i, 'elsewhere-' || i FROM n",
            [count],
        )
        .unwrap();
}

/// A staged enqueue on a crawl that visited 100,000 URLs names the
/// identity of the request it added, which a sample comparison weighs,
/// without any statement that scans the visited set: its write holds the
/// Run's one writer for one admission, whatever the crawl's size.
///
/// Covers: story:5-116, story:6-236
#[test]
fn a_staged_enqueue_names_its_identity_without_scanning_the_visited_set() {
    let connection = leased();
    visited_elsewhere(&connection, 100_000);
    let staging = Staging::default();
    staging.open("1", 1);

    let (answered, statements) = statements_of(&connection, || {
        answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b")))
    });

    assert_eq!(
        (
            answered,
            staging.pushed("1", 1).enqueues,
            scans_of(&connection, &statements, "visited"),
        ),
        (
            Enqueued::Added {
                id: "2".to_owned(),
                url: "https://shop.test/b".to_owned()
            },
            ["GET https://shop.test/b".to_owned()].into(),
            Vec::<String>::new(),
        )
    );
}

/// n separate enqueues of one attempt run n admissions while they stage,
/// one each, and n again at the commit, in order: an enqueue does not run
/// the attempt's earlier ones again to answer.
///
/// Covers: story:6-236
#[test]
fn separate_enqueues_of_one_attempt_run_one_admission_each() {
    let connection = leased();
    let staging = Staging::default();
    staging.open("1", 1);
    let admissions = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = |url: &'static str| {
        let admissions = std::sync::Arc::clone(&admissions);
        let admission = enqueue(url);
        move |connection: &Connection, positions: &mut Positions| {
            admissions.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            admission(connection, positions)
        }
    };
    let urls = [
        "https://shop.test/1",
        "https://shop.test/2",
        "https://shop.test/3",
        "https://shop.test/4",
        "https://shop.test/5",
        "https://shop.test/6",
        "https://shop.test/7",
        "https://shop.test/8",
        "https://shop.test/9",
        "https://shop.test/10",
    ];

    for url in urls {
        answer(staging.admit(&connection, "1", 1, counted(url)));
    }
    let staged = admissions.swap(0, std::sync::atomic::Ordering::Relaxed);
    staging.take("1", 1).admit(&connection).unwrap();
    let committed = admissions.load(std::sync::atomic::Ordering::Relaxed);

    assert_eq!(
        (staged, committed, work_items_of(&connection).len()),
        (10, 10, 11)
    );
}

/// Two attempts that stage the same link are each answered `added` with an
/// ID of its own; the commit of the second, when the first committed it
/// meanwhile, does not add it twice, and names the ID its staging answered,
/// why it was not added and the request it is a duplicate of, which its
/// outcome then carries (ADR 0093).
///
/// Covers: story:6-236
#[test]
fn a_commit_names_each_staged_request_another_commit_added_first() {
    let connection = leased();
    let seed: Seed =
        serde_json::from_value(json!({ "url": "https://shop.test/a", "label": "page" })).unwrap();
    work_items::admit_seeds(
        &connection,
        &mut Positions::of(&connection).unwrap(),
        &Scope::read(&connection).unwrap(),
        &every_host(),
        &[seed],
    )
    .unwrap();
    let epoch = work_items::lease(&connection, &["2".to_owned()]).unwrap()[0].lease_epoch;
    let staging = Staging::default();
    staging.open("1", 1);
    staging.open("2", epoch);
    let answers = [
        answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b"))),
        answer(staging.admit(&connection, "2", epoch, enqueue("https://shop.test/b"))),
    ];

    let second = staging
        .take("2", epoch)
        .admit(&connection)
        .unwrap()
        .not_added;
    let first = staging.take("1", 1).admit(&connection).unwrap().not_added;

    assert_eq!(
        (answers, second, first, work_items_of(&connection)),
        (
            [
                Enqueued::Added {
                    id: "3".to_owned(),
                    url: "https://shop.test/b".to_owned()
                },
                Enqueued::Added {
                    id: "4".to_owned(),
                    url: "https://shop.test/b".to_owned()
                },
            ],
            Vec::new(),
            vec![NotAddedAtCommit {
                id: "3".to_owned(),
                url: "https://shop.test/b".to_owned(),
                reason: NotAddedReason::Duplicate,
                duplicate_of: Some("4".to_owned()),
            }],
            vec![
                ("1".to_owned(), "https://shop.test/".to_owned()),
                ("2".to_owned(), "https://shop.test/a".to_owned()),
                ("4".to_owned(), "https://shop.test/b".to_owned()),
            ],
        )
    );
}

/// An attempt stages at most `crawl.max_staged_enqueues` admissions: one
/// more fails with `staging_full`, naming the setting, and stages nothing,
/// so the commit adds only those before it.
///
/// Covers: story:6-236, error:staging_full, setting:crawl.max_staged_enqueues
#[test]
fn an_admission_past_the_attempts_bound_fails_with_staging_full() {
    let connection = leased();
    let staging = Staging::bounded(2);
    staging.open("1", 1);
    answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b")));
    answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/c")));

    let refused = staging
        .admit(&connection, "1", 1, enqueue("https://shop.test/d"))
        .unwrap()
        .unwrap()
        .unwrap_err();
    staging.take("1", 1).admit(&connection).unwrap();

    assert_eq!(
        (
            refused.code(),
            refused
                .details()
                .and_then(|details| details.get("limit"))
                .cloned(),
            work_items_of(&connection).len(),
        ),
        ("staging_full", Some(json!("crawl.max_staged_enqueues")), 3)
    );
}

/// One admission of the crawl, as a staged call or a commit runs it.
type Step = Box<dyn Fn(&Connection, &mut Positions) -> Answered<Enqueued> + Send + Sync>;

/// Enqueues and a seed whose answers hang on the ones before them: a
/// duplicate, a path budget and `crawl.max_requests` filling up, and a
/// seed that widens the scope for a later link.
fn dependent_steps() -> Vec<Step> {
    let seed = |connection: &Connection, positions: &mut Positions| {
        let seed: Seed =
            serde_json::from_value(json!({ "url": "https://other.test/", "label": "page" }))
                .unwrap();
        let mut answers = work_items::admit_seeds(
            connection,
            positions,
            &Scope::read(connection)?,
            &every_host(),
            &[seed],
        )?;
        Ok(Some(Ok(answers.pop().unwrap())))
    };
    vec![
        Box::new(enqueue("https://shop.test/b")),
        Box::new(enqueue("https://shop.test/b")),
        Box::new(enqueue("https://other.test/early")),
        Box::new(enqueue("https://shop.test/sale/1")),
        Box::new(enqueue("https://shop.test/sale/2")),
        Box::new(enqueue("https://shop.test/sale/3")),
        Box::new(seed),
        Box::new(enqueue("https://other.test/late")),
        Box::new(enqueue("https://shop.test/c")),
    ]
}

/// A run file as [`leased`] makes it, bounded at 6 requests and 2 under
/// `/sale/*`.
fn bounded() -> Connection {
    let connection = leased();
    connection
        .execute_batch(
            "INSERT INTO bounds (id, max_requests) VALUES (1, 6);
             INSERT INTO bounds_path_budgets (position, pattern, max_requests)
             VALUES (0, '/sale/*', 2);",
        )
        .unwrap();
    connection
}

/// Separate staged admissions of one attempt are answered as admitting
/// each in turn on the run file answers them, though none reached it: a
/// duplicate of an earlier one, the path budget and `crawl.max_requests`
/// counting the earlier ones, and a seed widening the scope for a later
/// link; their commit then adds each as answered.
///
/// Covers: story:6-236
#[test]
fn staged_admissions_are_answered_as_admitting_them_in_turn_answers() {
    let staged_run = bounded();
    let direct_run = bounded();
    let staging = Staging::default();
    staging.open("1", 1);

    let staged: Vec<Enqueued> = dependent_steps()
        .into_iter()
        .map(|step| answer(staging.admit(&staged_run, "1", 1, step)))
        .collect();
    let not_added = staging.take("1", 1).admit(&staged_run).unwrap().not_added;
    let direct: Vec<Enqueued> = dependent_steps()
        .into_iter()
        .map(|step| answer(step(&direct_run, &mut Positions::of(&direct_run).unwrap())))
        .collect();

    assert_eq!(
        (
            staged.iter().map(named).collect::<Vec<_>>(),
            staged,
            not_added,
            work_items_of(&staged_run),
        ),
        (
            [
                "added 2",
                "duplicate",
                "out_of_scope",
                "added 3",
                "added 4",
                "path_budget",
                "added 5",
                "added 6",
                "max_requests",
            ]
            .map(str::to_owned)
            .to_vec(),
            direct,
            Vec::new(),
            work_items_of(&direct_run),
        )
    );
}

/// An answer as `added <id>`, or the reason it was not added.
fn named(enqueued: &Enqueued) -> String {
    match enqueued {
        Enqueued::Added { id, .. } => format!("added {id}"),
        Enqueued::NotAdded { reason, .. } => serde_json::to_value(reason)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned(),
    }
}

/// What an attempt staged so far names the Frontier identity of each
/// request it added, a duplicate not among them, which a sample comparison
/// weighs (ADR 0103); a discard answers them with the Records, and the
/// attempt staged anew holds none.
///
/// Covers: story:5-116
#[test]
fn what_an_attempt_staged_names_the_frontier_identity_of_each_request_it_added() {
    let connection = leased();
    let staging = Staging::default();
    staging.open("1", 1);

    answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b")));
    answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/b")));
    answer(staging.admit(&connection, "1", 1, enqueue("https://shop.test/c?x=1")));
    let so_far = staging.pushed("1", 1).enqueues;
    let discarded = staging.discard("1", 1).enqueues;
    staging.open("1", 1);

    assert_eq!(
        (so_far.clone(), discarded, staging.pushed("1", 1).enqueues),
        (
            ["GET https://shop.test/b", "GET https://shop.test/c?x=1"]
                .map(str::to_owned)
                .into(),
            so_far,
            std::collections::BTreeSet::new(),
        )
    );
}
