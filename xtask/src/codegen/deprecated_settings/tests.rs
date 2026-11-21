use serde_json::json;

use super::generate;

/// Covers: story:11-182
#[test]
fn the_list_names_each_deprecated_key_with_its_replacement_in_key_order() {
    let catalog = json!({
        "settings": [
            {
                "name": "timeout.wait",
                "products": ["collie", "foxhound"],
                "deprecated": {
                    "since": "0.2",
                    "replacement": "timeout.navigation",
                    "earliest_removal": "1.0",
                    "note": "One timeout | both."
                }
            },
            { "name": "timeout.navigation", "products": ["collie"], "deprecated": null },
            {
                "name": "sample.echo_repeats",
                "products": ["collie"],
                "deprecated": {
                    "since": "0.1",
                    "replacement": null,
                    "earliest_removal": "1.0",
                    "note": "A sample."
                }
            }
        ]
    });

    let list = generate(&catalog).unwrap();

    let rows: Vec<&str> = list
        .lines()
        .filter(|line| line.starts_with("| `"))
        .collect();
    assert_eq!(
        rows,
        [
            "| `sample.echo_repeats` | collie | 0.1 | 1.0 | none | A sample. |",
            "| `timeout.wait` | collie, foxhound | 0.2 | 1.0 | `timeout.navigation` | One timeout \\| both. |",
        ],
        "{list}"
    );
}

#[test]
fn the_list_says_so_when_no_key_is_deprecated() {
    let catalog = json!({ "settings": [{ "name": "timeout.navigation", "products": ["collie"] }] });

    let list = generate(&catalog).unwrap();

    assert!(
        list.ends_with("\nNo settings key is deprecated.\n"),
        "{list}"
    );
}
