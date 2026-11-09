use goodfetch_protocol::run::{Lifecycle, RunKind};
use rusqlite::Connection;

use super::{clean_up_digest, epoch, is_of_kind, lifecycle};
use crate::run_state::SCHEMA;

/// A run file's tables, holding the Run's row as `values` gives it.
fn run_file(values: &str) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in SCHEMA {
        connection.execute(statement, []).unwrap();
    }
    connection
        .execute(
            &format!(
                "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at, clean_up_digest)
                 VALUES {values}"
            ),
            [],
        )
        .unwrap();
    connection
}

#[test]
fn the_runs_row_reads_back_as_its_kind_lifecycle_digest_and_run_epoch() {
    let file =
        run_file("('r', 'crawl', 'foxhound', 'os_user', 3, '{}', 'finished', 't', 'sha256:ab')");

    let read = (
        is_of_kind(&file, RunKind::Crawl).unwrap(),
        is_of_kind(&file, RunKind::Collie).unwrap(),
        lifecycle(&file).unwrap(),
        clean_up_digest(&file).unwrap(),
        epoch(&file).unwrap(),
    );

    assert_eq!(
        read,
        (
            true,
            false,
            Lifecycle::Finished,
            Some("sha256:ab".to_owned()),
            3
        )
    );
}

/// A newer minor's run file may hold a kind this engine does not know: it
/// is of no kind this engine asks about, which is no failure.
#[test]
fn a_kind_this_engine_does_not_know_is_none_it_asks_about() {
    let file = run_file("('r', 'later', 'foxhound', 'os_user', 1, '{}', 'running', 't', NULL)");

    assert!(!is_of_kind(&file, RunKind::Crawl).unwrap());
}

#[test]
fn a_run_that_holds_to_no_clean_up_rules_reads_no_digest() {
    let file = run_file("('r', 'collie', 'collie', 'os_user', 1, '{}', 'running', 't', NULL)");

    assert_eq!(clean_up_digest(&file).unwrap(), None);
}
