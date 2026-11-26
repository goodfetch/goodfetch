//! Search across runs on the query connection: each selected run's index
//! read through its table function, the best of all kept. Foxhound's
//! `it/search.rs` proves both searches through the executable.

use std::path::{Path, PathBuf};

use goodfetch_protocol::foxhound::search::FoundRecord;
use goodfetch_sql_tab::Matching;
use rusqlite::Connection;
use serde_json::Value;

use crate::run_files::{LIMITS, book, run_file, sandbox};

/// A run file of the `books` Records `pages`, each indexed under its JSON
/// text.
fn indexed(folder: &Path, run: &str, pages: &[u64]) -> PathBuf {
    let records: Vec<Value> = pages.iter().copied().map(book).collect();
    let path = run_file(folder, run, &records, "none");
    Connection::open(&path)
        .expect("the run file opens")
        .execute(
            "INSERT INTO record_search (dataset, number, text) SELECT dataset, number, data FROM records",
            [],
        )
        .expect("the Records are indexed");
    path
}

/// The run, Dataset, number and value of each Record found.
fn found(records: &[FoundRecord]) -> Vec<(&str, &str, u64, &Value)> {
    records
        .iter()
        .map(|record| {
            (
                record.run_id.as_str(),
                record.dataset.as_str(),
                record.number,
                &record.value,
            )
        })
        .collect()
}

/// A search reads only the runs the connection selected, finds there the
/// Records holding every word, in the Datasets it names, and answers at
/// most its limit.
///
/// Covers: story:12-209
#[test]
fn a_search_finds_the_records_holding_every_word_in_the_selected_runs_only() {
    let folder = tempfile::tempdir().expect("a folder");
    let shop = indexed(folder.path(), "shop", &[1, 2, 3]);
    let other = indexed(folder.path(), "other", &[2]);
    indexed(folder.path(), "unselected", &[2]);
    let search = |text: &str, datasets: Option<Vec<String>>, limit: u32| {
        let mut records = sandbox(&[("shop", &shop), ("other", &other)], LIMITS)
            .search(Matching::of(text, datasets, limit).expect("a word"))
            .expect("the search runs");
        records.sort_by(|left, right| {
            (left.run_id.as_str(), left.number).cmp(&(right.run_id.as_str(), right.number))
        });
        records
    };
    let (two, three) = (book(2), book(3));

    let both = search("CAFE 2", None, 10);
    let one = search("café 3", None, 10);
    let limited = search("café", None, 2);
    let elsewhere = search("café", Some(vec!["pages".to_owned()]), 10);

    assert_eq!(
        (found(&both), found(&one), limited.len(), found(&elsewhere)),
        (
            vec![("other", "books", 1, &two), ("shop", "books", 2, &two)],
            vec![("shop", "books", 3, &three)],
            2,
            Vec::new(),
        )
    );
}

/// A run file that predates the index matches nothing, and fails nothing.
///
/// Covers: story:12-209
#[test]
fn a_run_file_without_the_index_matches_nothing() {
    let folder = tempfile::tempdir().expect("a folder");
    let path = run_file(folder.path(), "shop", &[book(1)], "none");
    Connection::open(&path)
        .expect("the run file opens")
        .execute_batch("DROP TABLE record_search")
        .expect("the index is dropped");

    let records = sandbox(&[("shop", &path)], LIMITS)
        .search(Matching::of("café", None, 10).expect("a word"))
        .expect("the search runs");

    assert_eq!(records, Vec::new());
}
