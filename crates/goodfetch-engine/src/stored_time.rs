//! A point in time in a store's SQLite file (ADR 0092): its RFC 3339 text in
//! UTC, as the wire carries it, so the files the engine wrote before keep
//! reading the same.

use std::time::SystemTime;

use goodfetch_protocol::point_in_time;
use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};

/// A point in time as a store's column holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredTime(pub SystemTime);

impl ToSql for StoredTime {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        point_in_time::to_rfc3339(self.0)
            .map(ToSqlOutput::from)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    }
}

impl FromSql for StoredTime {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        point_in_time::from_rfc3339(value.as_str()?)
            .map(Self)
            .map_err(|error| FromSqlError::Other(Box::new(error)))
    }
}

#[cfg(test)]
mod tests;
