//! Covers: story:5-118

use std::collections::BTreeMap;

use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use rusqlite::Connection;
use serde_json::json;

use super::{TABLES, read, record};

/// The Labels a crawl's start declared read back as they were, so a
/// resumed crawl forces the same Label's tier again.
#[test]
fn the_declared_labels_read_back_as_the_start_recorded_them() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let declared = BTreeMap::from([
        (
            "list".to_owned(),
            LabelDeclaration {
                payload: Some(json!({ "type": "object" })),
                settings: json!({ "fetch": { "tier": "http" } })
                    .as_object()
                    .cloned()
                    .unwrap(),
            },
        ),
        ("page".to_owned(), LabelDeclaration::default()),
    ]);

    record(&connection, &declared).unwrap();

    assert_eq!(read(&connection).unwrap(), declared);
}
