//! A SQLite store as SQL text (ADR 0066): what a SQLite format's
//! Compatibility corpus holds, written the way `sqlite3`'s `.dump` writes
//! it, so `xtask compat` reads its columns, but byte-stable: tables and
//! indexes in name order, each table's rows in `rowid` order (or primary
//! key order for a table without one), every value spelled one way.

use std::fmt::Write as _;

use rusqlite::Connection;
use rusqlite::types::ValueRef;

/// The schema and rows of every table of `connection`'s main database, and
/// its indexes, as SQL statements.
///
/// # Errors
///
/// When SQLite cannot read the schema or a table.
pub fn sql_dump(connection: &Connection) -> rusqlite::Result<String> {
    let mut dump = String::from("PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\n");
    let mut tables = connection.prepare(
        "SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let tables: Vec<(String, String)> = tables
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (table, sql) in &tables {
        let _ = writeln!(dump, "{sql};");
        let quoted = table.replace('"', "\"\"");
        let without_rowid = sql.to_ascii_uppercase().contains("WITHOUT ROWID");
        let order = if without_rowid { "1" } else { "rowid" };
        let mut rows =
            connection.prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY {order}"))?;
        let columns = rows.column_count();
        let mut query = rows.query([])?;
        while let Some(row) = query.next()? {
            let mut values = Vec::with_capacity(columns);
            for index in 0..columns {
                values.push(literal(row.get_ref(index)?));
            }
            let _ = writeln!(
                dump,
                "INSERT INTO \"{quoted}\" VALUES({});",
                values.join(",")
            );
        }
    }
    let mut indexes = connection.prepare(
        "SELECT sql FROM sqlite_schema WHERE type = 'index' AND sql IS NOT NULL ORDER BY name",
    )?;
    let indexes: Vec<String> = indexes
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for sql in indexes {
        let _ = writeln!(dump, "{sql};");
    }
    dump.push_str("COMMIT;\n");
    Ok(dump)
}

/// One value as a SQL literal.
fn literal(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_owned(),
        ValueRef::Integer(integer) => integer.to_string(),
        ValueRef::Real(real) => format!("{real:?}"),
        ValueRef::Text(text) => format!("'{}'", String::from_utf8_lossy(text).replace('\'', "''")),
        ValueRef::Blob(blob) => {
            let mut hex = String::with_capacity(blob.len() * 2 + 3);
            hex.push_str("X'");
            for byte in blob {
                let _ = write!(hex, "{byte:02X}");
            }
            hex.push('\'');
            hex
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::sql_dump;

    #[test]
    fn a_dump_holds_each_table_its_rows_in_order_and_its_indexes() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE b (x TEXT, y BLOB, z REAL);
                 CREATE TABLE a (n INTEGER);
                 CREATE INDEX a_by_n ON a (n);
                 INSERT INTO b VALUES ('it''s', X'00ff', 1.5), (NULL, NULL, NULL);
                 INSERT INTO a VALUES (2), (1);",
            )
            .unwrap();

        let dump = sql_dump(&connection).unwrap();

        assert_eq!(
            dump,
            "PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\n\
             CREATE TABLE a (n INTEGER);\nINSERT INTO \"a\" VALUES(2);\nINSERT INTO \"a\" VALUES(1);\n\
             CREATE TABLE b (x TEXT, y BLOB, z REAL);\nINSERT INTO \"b\" VALUES('it''s',X'00FF',1.5);\nINSERT INTO \"b\" VALUES(NULL,NULL,NULL);\n\
             CREATE INDEX a_by_n ON a (n);\nCOMMIT;\n"
        );
    }
}
