use std::time::{Duration, SystemTime};

use rusqlite::Connection;

use super::StoredTime;

#[test]
fn a_point_in_time_is_kept_as_rfc3339_text_in_utc_and_read_back() {
    #[expect(
        clippy::disallowed_methods,
        reason = "an in-memory file only this test opens"
    )]
    let connection = Connection::open_in_memory().unwrap();
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_091_400_250);

    let (text, read): (String, StoredTime) = connection
        .query_row("SELECT ?1, ?1", [StoredTime(at)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();

    assert_eq!(
        (text.as_str(), read),
        ("2026-11-19T12:30:00.25Z", StoredTime(at))
    );
}

#[test]
fn text_that_is_no_point_in_time_fails_to_read() {
    #[expect(
        clippy::disallowed_methods,
        reason = "an in-memory file only this test opens"
    )]
    let connection = Connection::open_in_memory().unwrap();

    let read = connection.query_row("SELECT '2026-11-19'", [], |row| row.get::<_, StoredTime>(0));

    assert!(read.is_err(), "{read:?}");
}
