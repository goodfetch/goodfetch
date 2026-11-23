//! The SQLite Sink's translation point: one named case per SQLite result
//! code it knows, and one it does not (ADR 0127).

use rstest::rstest;
use rusqlite::ffi;

use super::SqliteSink;
use crate::contract::{FailureKind, Primitives};

#[rstest]
#[case::busy(ffi::SQLITE_BUSY, FailureKind::Transient)]
#[case::locked(ffi::SQLITE_LOCKED, FailureKind::Transient)]
#[case::disk_full(ffi::SQLITE_FULL, FailureKind::Transient)]
#[case::io(ffi::SQLITE_IOERR, FailureKind::Transient)]
#[case::out_of_memory(ffi::SQLITE_NOMEM, FailureKind::Transient)]
#[case::read_only(ffi::SQLITE_READONLY, FailureKind::Auth)]
#[case::permission(ffi::SQLITE_PERM, FailureKind::Auth)]
#[case::authorization(ffi::SQLITE_AUTH, FailureKind::Auth)]
#[case::cannot_open(ffi::SQLITE_CANTOPEN, FailureKind::Gone)]
#[case::not_a_database(ffi::SQLITE_NOTADB, FailureKind::Gone)]
#[case::constraint(ffi::SQLITE_CONSTRAINT, FailureKind::RejectedRecords)]
#[case::too_big(ffi::SQLITE_TOOBIG, FailureKind::RejectedRecords)]
#[case::mismatch(ffi::SQLITE_MISMATCH, FailureKind::RejectedRecords)]
#[case::unknown_code(ffi::SQLITE_PROTOCOL, FailureKind::Unrecognized)]
fn sqlite_result_codes_become_delivery_failures(#[case] code: i32, #[case] expected: FailureKind) {
    let sink = SqliteSink::new(rusqlite::Connection::open_in_memory().unwrap(), "records");
    let error = rusqlite::Error::SqliteFailure(ffi::Error::new(code), None);

    assert_eq!(sink.map_failure(error).kind(), expected);
}

#[test]
fn a_failure_that_is_not_sqlites_is_unrecognized() {
    let sink = SqliteSink::new(rusqlite::Connection::open_in_memory().unwrap(), "records");

    assert_eq!(
        sink.map_failure(rusqlite::Error::InvalidQuery).kind(),
        FailureKind::Unrecognized
    );
}
