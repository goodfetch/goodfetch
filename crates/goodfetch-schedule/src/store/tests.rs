use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::user_store::{UserStore, UserStoreError};
use goodfetch_protocol::schedule::{
    EveryTrigger, RecipeTarget, ScheduleId, ScheduleInfo, ScheduleState, ScheduleTarget,
    ScheduleTrigger,
};
use goodfetch_test_support::{TestEnv, runtime};
use rusqlite::{Connection, OptionalExtension};

use super::{STORE, create, delete, get, jitter_ms, list, made_store_id};

mod model;

/// The Schedule of `id`, firing every `every` milliseconds.
fn schedule(id: &str, every: u64) -> ScheduleInfo {
    ScheduleInfo {
        schedule_id: ScheduleId::parse(id).unwrap(),
        target: ScheduleTarget::Recipe(RecipeTarget {
            recipe: "./prices.recipe.ts".to_owned(),
            inputs: None,
            settings: None,
            timeout: Some(None),
        }),
        trigger: ScheduleTrigger::Every(EveryTrigger {
            every,
            anchor: None,
        }),
        time_zone: "Europe/Paris".to_owned(),
        version: 1,
        state: ScheduleState::On,
        creator: "ada".to_owned(),
        created_at: UNIX_EPOCH + Duration::from_millis(1_764_061_234_567),
    }
}

/// A store ID draw that makes the ID of `draw`.
fn drawn(draw: u64) -> impl FnOnce() -> String + Send + 'static {
    move || made_store_id(draw, draw)
}

/// The store's ID, once a create drew one.
async fn store_id(store: &UserStore) -> Result<Option<String>, UserStoreError> {
    store
        .read(None, |connection| {
            connection
                .prepare_cached("SELECT store_id FROM store")?
                .query_row([], |row| row.get(0))
                .optional()
        })
        .await
}

fn path(env: &TestEnv) -> PathBuf {
    env.home().join("0.1").join(STORE.file())
}

fn store(env: &TestEnv) -> UserStore {
    UserStore::open(
        path(env),
        &STORE,
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    )
}

#[test]
fn a_created_schedule_is_read_and_listed_as_recorded() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        assert!(
            create(&store, schedule("prices", 60_000), 4_200, drawn(1))
                .await
                .unwrap()
        );
        assert!(
            create(&store, schedule("a-first", 1), 0, drawn(2))
                .await
                .unwrap()
        );

        assert_eq!(
            get(&store, &ScheduleId::parse("prices").unwrap())
                .await
                .unwrap(),
            Some(schedule("prices", 60_000))
        );
        assert_eq!(
            jitter_ms(&store, &ScheduleId::parse("prices").unwrap())
                .await
                .unwrap(),
            Some(4_200)
        );
        assert_eq!(
            list(&store).await.unwrap(),
            vec![schedule("a-first", 1), schedule("prices", 60_000)]
        );
    });
}

/// Covers: story:10-7
#[test]
fn a_create_of_a_taken_id_records_nothing_and_keeps_the_first() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        create(&store, schedule("prices", 60_000), 0, drawn(1))
            .await
            .unwrap();

        let again = create(&store, schedule("prices", 5), 99, drawn(2))
            .await
            .unwrap();

        assert!(!again, "a taken ID was recorded again");
        assert_eq!(
            list(&store).await.unwrap(),
            vec![schedule("prices", 60_000)]
        );
    });
}

#[test]
fn a_delete_answers_whether_the_store_held_the_schedule() {
    let env = TestEnv::new();
    let store = store(&env);
    let id = ScheduleId::parse("prices").unwrap();
    runtime().block_on(async {
        create(&store, schedule("prices", 60_000), 0, drawn(1))
            .await
            .unwrap();

        let first = delete(&store, &id).await.unwrap();
        let resent = delete(&store, &id).await.unwrap();

        assert_eq!((first, resent), (true, false));
        assert_eq!(get(&store, &id).await.unwrap(), None);
    });
}

#[test]
fn a_store_never_written_reads_empty_and_makes_no_file() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        assert_eq!(list(&store).await.unwrap(), Vec::new());
        assert_eq!(store_id(&store).await.unwrap(), None);
    });
    assert!(!path(&env).exists(), "a read made the store's file");
}

#[test]
fn the_store_keeps_the_id_its_first_create_drew() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        create(&store, schedule("one", 1), 0, drawn(7))
            .await
            .unwrap();
        create(&store, schedule("two", 1), 0, drawn(8))
            .await
            .unwrap();

        assert_eq!(store_id(&store).await.unwrap(), Some(made_store_id(7, 7)));
    });
}

#[test]
fn a_store_id_is_26_lowercase_crockford_characters_of_its_draws() {
    let made = made_store_id(u64::MAX, 1);

    assert_eq!(made.len(), 26);
    assert!(
        made.chars()
            .all(|character| character.is_ascii_digit() || character.is_ascii_lowercase()),
        "{made}"
    );
    assert_ne!(made, made_store_id(1, u64::MAX));
}

#[test]
fn the_store_records_its_formats_major_and_minor() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        create(&store, schedule("prices", 1), 0, drawn(1))
            .await
            .unwrap();
    });

    let recorded: String = Connection::open(path(&env))
        .unwrap()
        .prepare_cached("SELECT version FROM format WHERE name = 'schedule-store'")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();

    assert_eq!(recorded, "0.1");
}

#[test]
fn a_row_or_table_of_a_later_minor_is_left_as_it_is() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        create(&store, schedule("prices", 1), 0, drawn(1))
            .await
            .unwrap();
    });
    Connection::open(path(&env))
        .unwrap()
        .execute_batch(
            "ALTER TABLE schedules ADD COLUMN paused_by TEXT;
             UPDATE schedules SET paused_by = 'later';
             CREATE TABLE occurrences (id TEXT) STRICT;
             INSERT INTO occurrences VALUES ('kept');",
        )
        .unwrap();

    runtime().block_on(async {
        create(&store, schedule("other", 1), 0, drawn(2))
            .await
            .unwrap();
        delete(&store, &ScheduleId::parse("other").unwrap())
            .await
            .unwrap();
        assert_eq!(list(&store).await.unwrap(), vec![schedule("prices", 1)]);
    });

    let kept: (String, String) = Connection::open(path(&env))
        .unwrap()
        .prepare_cached(
            "SELECT (SELECT paused_by FROM schedules WHERE id = 'prices'), (SELECT id FROM occurrences)",
        )
        .unwrap()
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!(kept, ("later".to_owned(), "kept".to_owned()));
}
