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
    VISITED_TABLES, admit, document_base,
};
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
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
        admit(connection, &scope, &every_host(), candidate, |url| {
            work_items::add(
                connection,
                positions,
                url,
                Adding {
                    method: &method,
                    body: None,
                    label: "page",
                    payload: None,
                    priority: 0,
                    forefront: false,
                    depth: 1,
                },
            )
        })
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
    let writes = staging.take("1", 1).admit(&connection).unwrap().len();

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
