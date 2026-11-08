use rusqlite::Connection;

use super::RUN_FILE_TABLES;
use crate::TABLES;

/// A run file holding the Datasets' tables.
fn run_file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
}

/// The columns the run file's table `name` holds.
fn columns_of(connection: &Connection, name: &str) -> Vec<String> {
    connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .unwrap()
        .query_map([name], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn every_column_and_key_of_a_run_table_is_one_its_run_file_table_holds() {
    let file = run_file();

    let missing: Vec<(&str, &str)> = RUN_FILE_TABLES
        .iter()
        .flat_map(|table| {
            let held = columns_of(&file, table.name);
            let read = (0..table.run_table.columns.len()).filter_map(|index| table.column(index));
            table
                .key
                .iter()
                .copied()
                .chain(read)
                .filter(move |column| !held.iter().any(|held| held == column))
                .map(move |column| (table.name, column))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(missing, Vec::<(&str, &str)>::new());
}

#[test]
fn only_the_run_id_reads_no_column_of_the_run_file() {
    let unread: Vec<(&str, &str)> = RUN_FILE_TABLES
        .iter()
        .flat_map(|table| {
            table
                .run_table
                .columns
                .iter()
                .enumerate()
                .filter(|(index, _)| table.column(*index).is_none())
                .map(|(_, column)| (table.run_table.name, column.name))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(
        unread,
        vec![
            ("records", "run"),
            ("rejected_records", "run"),
            ("datasets", "run")
        ]
    );
}

/// Which column of the run file each column of a run table reads, written
/// out here: the SQL tab answers each name with what its source holds, and
/// `request_id` and `value` read columns named otherwise.
#[test]
fn each_run_tables_column_reads_its_own_column_of_the_run_file() {
    let read: Vec<Vec<(&str, Option<&str>)>> = RUN_FILE_TABLES
        .iter()
        .map(|table| {
            table
                .run_table
                .columns
                .iter()
                .enumerate()
                .map(|(index, column)| (column.name, table.column(index)))
                .collect()
        })
        .collect();

    assert_eq!(
        read,
        [
            vec![
                ("run", None),
                ("dataset", Some("dataset")),
                ("number", Some("number")),
                ("value", Some("data")),
                ("request_id", Some("work_item")),
                ("url", Some("url")),
                ("committed_at", Some("committed_at")),
                ("provenance", Some("provenance")),
            ],
            vec![
                ("run", None),
                ("dataset", Some("dataset")),
                ("number", Some("number")),
                ("data", Some("data")),
                ("request_id", Some("work_item")),
                ("rejection", Some("rejection")),
            ],
            vec![
                ("run", None),
                ("name", Some("name")),
                ("position", Some("position")),
                ("schema", Some("schema")),
                ("key", Some("key")),
            ],
        ]
    );
}

#[test]
fn a_run_file_no_crawl_wrote_holds_no_datasets_table() {
    let empty = Connection::open_in_memory().unwrap();

    let held = RUN_FILE_TABLES.map(|table| table.held_by(&empty).unwrap());

    assert_eq!(held, [false; 3]);
}

#[test]
fn a_crawls_run_file_holds_every_datasets_table() {
    let file = run_file();

    let held = RUN_FILE_TABLES.map(|table| table.held_by(&file).unwrap());

    assert_eq!(held, [true; 3]);
}
