use serde_json::json;

use super::{ROWS, settle_labels};

/// A workspace whose rows name two Shared paths and three works.
fn workspace_with_rows() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    let rows = folder.path().join(ROWS);
    std::fs::create_dir_all(rows.parent().unwrap()).unwrap();
    std::fs::write(
        rows,
        "\nrows! {\n    /// A row.\n    One => \"f1.parses\", \"F1\", \"parses\";\n    Two => \"f1.reads\", \"F1\", \"reads\";\n    Three => \"z9.new\", \"Z9\", \"news\";\n}\n",
    )
    .unwrap();
    folder
}

/// A row added since this `xtask` was built reaches the catalog at once:
/// the linked crate still holds the table as it stood, so one `xtask codegen`
/// would otherwise leave the labels a build behind what it wrote.
#[test]
fn a_new_rows_labels_reach_the_catalog_the_linked_crate_emitted() {
    let folder = workspace_with_rows();
    let mut catalog = json!({ "metrics": [
        { "name": "goodfetch.other", "labels": [{ "name": "work", "values": ["kept"] }] },
        { "name": "goodfetch.counted_work", "labels": [
            { "name": "shared_path", "values": ["F1"] },
            { "name": "work", "values": ["parses"] },
        ] },
    ] });

    settle_labels(&mut catalog, folder.path()).unwrap();

    assert_eq!(
        catalog,
        json!({ "metrics": [
            { "name": "goodfetch.other", "labels": [{ "name": "work", "values": ["kept"] }] },
            { "name": "goodfetch.counted_work", "labels": [
                { "name": "shared_path", "values": ["F1", "Z9"] },
                { "name": "work", "values": ["news", "parses", "reads"] },
            ] },
        ] })
    );
}

#[test]
fn a_catalog_without_the_metric_the_rows_label_is_an_error() {
    let folder = workspace_with_rows();
    let mut catalog = json!({ "metrics": [] });

    assert!(settle_labels(&mut catalog, folder.path()).is_err());
}

#[test]
fn a_tree_without_rows_leaves_the_catalog_alone() {
    let folder = tempfile::tempdir().unwrap();
    let mut catalog = json!({ "metrics": [] });

    settle_labels(&mut catalog, folder.path()).unwrap();

    assert_eq!(catalog, json!({ "metrics": [] }));
}
