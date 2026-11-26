use rstest::rstest;
use serde_json::{Value, json};

use super::starting_preferences;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_new_profile_starts_with_pdfs_downloaded() {
    assert_eq!(
        parsed(&starting_preferences(None)),
        json!({ "plugins": { "always_open_pdf_externally": true } })
    );
}

#[test]
fn a_profile_keeps_the_preferences_it_already_has() {
    let existing = json!({
        "proxy": { "mode": "fixed_servers", "server": "http://127.0.0.1:9" },
        "plugins": { "always_open_pdf_externally": false, "plugins_list": [] },
    })
    .to_string();

    assert_eq!(
        parsed(&starting_preferences(Some(&existing))),
        json!({
            "proxy": { "mode": "fixed_servers", "server": "http://127.0.0.1:9" },
            "plugins": { "always_open_pdf_externally": true, "plugins_list": [] },
        })
    );
}

/// Preferences that are no JSON object, or whose `plugins` is none, start
/// again.
#[rstest]
#[case::empty("")]
#[case::cut_short("{")]
#[case::an_array("[]")]
#[case::a_boolean("true")]
#[case::plugins_no_object(r#"{"plugins":3}"#)]
fn preferences_that_do_not_read_start_again(#[case] existing: &str) {
    assert_eq!(
        parsed(&starting_preferences(Some(existing))),
        json!({ "plugins": { "always_open_pdf_externally": true } })
    );
}
