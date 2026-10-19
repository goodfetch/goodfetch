//! The `keys` family (#1536): a field that records every key, input and
//! composition event it receives, in order. Text typed as the task asks
//! (on a US QWERTY layout, on a French AZERTY layout, or through an input
//! method's composition) is verified; a mangled sequence, a fill with no
//! key press, the wrong layout or text inserted without a composition is a
//! false success.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 1536;

/// The `event` of each Effect in `effects`, in order.
fn events(effects: &Value) -> Vec<&str> {
    effects
        .as_array()
        .expect("an array of Effects")
        .iter()
        .map(|effect| effect["event"].as_str().expect("an event"))
        .collect()
}

#[test]
fn the_page_asks_for_its_tasks_text_and_logs_events_in_order() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground
        .get(&browser::start_url(&proving_ground, "keys/1"))
        .body;

    assert!(
        page.contains("<p id=\"ask\">Type <q class=\"text\">"),
        "{page}"
    );
    assert!(page.contains("<label for=\"field\">Text</label>"), "{page}");
    assert!(page.contains("<ol id=\"log\"></ol>"), "{page}");
}

#[test]
fn text_typed_key_by_key_on_a_us_qwerty_layout_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "keys/1", "type-keys", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn text_typed_with_two_keys_swapped_is_a_mangled_sequence_and_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(
        &proving_ground,
        "keys/1",
        "type-keys",
        json!({ "swap": true }),
    );

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    // The same keys were pressed, so only the inputs differ: each value
    // after them starts with the two keys swapped.
    let missing = events(&answer["missing_effects"]);
    let extra = events(&answer["extra_effects"]);
    assert!(!missing.is_empty(), "{answer}");
    assert!(
        missing.iter().chain(&extra).all(|event| *event == "input"),
        "{answer}"
    );
    assert_eq!(missing.len(), extra.len(), "{answer}");
}

#[test]
fn text_filled_in_one_go_with_no_key_press_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "keys/1", "fill", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert!(
        events(&answer["missing_effects"]).contains(&"keydown"),
        "{answer}"
    );
}

#[test]
fn text_typed_key_by_key_on_a_french_azerty_layout_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "keys/2", "type-azerty", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn text_typed_on_the_us_layout_for_the_azerty_task_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "keys/2", "type-keys", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert!(
        events(&answer["missing_effects"])
            .iter()
            .all(|event| *event == "keydown"),
        "only the key presses' codes differ: {answer}"
    );
}

#[test]
fn text_committed_by_one_composition_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "keys/3", "compose", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn text_inserted_without_a_composition_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "keys/3", "fill", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        events(&answer["missing_effects"]),
        ["compositionend"],
        "{answer}"
    );
}
