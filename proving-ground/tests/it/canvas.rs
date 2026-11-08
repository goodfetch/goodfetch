//! The `canvas` family (#1536): a button drawn on a `<canvas>` with no
//! element, role or text a DOM selector could reach, beside a decoy, for
//! the input calls that take no target and for the vision rung; and a
//! drawing surface read back through `evaluate` (pitfall B14). A click on
//! the button is verified; a click on the decoy or a miss is a false
//! success, as is a surface read back without its stroke.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 1536;

/// Every label the canvas may draw on a button.
const LABELS: [&str; 8] = [
    "Accept", "Begin", "Confirm", "Continue", "Enter", "Proceed", "Start", "Submit",
];

/// The `target` of each Effect in `effects`, in order.
fn targets(effects: &Value) -> Vec<&Value> {
    effects
        .as_array()
        .expect("an array of Effects")
        .iter()
        .map(|effect| &effect["target"])
        .collect()
}

#[test]
fn the_canvas_page_has_no_element_role_or_text_for_its_button() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground
        .get(&browser::start_url(&proving_ground, "canvas/1"))
        .body;

    assert!(page.contains("<canvas"), "{page}");
    for absent in ["<button", "role=", "<input", "<a ", "aria-label", "<map"] {
        assert!(!page.contains(absent), "the page holds {absent}: {page}");
    }
    for label in LABELS {
        assert!(
            !page.contains(label),
            "the page holds the label {label}: {page}"
        );
    }
}

#[test]
fn a_click_on_the_drawn_button_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "canvas/1", "canvas-button", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_click_on_the_decoy_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "canvas/1", "canvas-decoy", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(targets(&answer["missing_effects"]), [&json!("button")]);
    assert_eq!(targets(&answer["extra_effects"]), [&json!("decoy")]);
}

#[test]
fn a_missed_canvas_click_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "canvas/1", "canvas-corner", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(targets(&answer["missing_effects"]), [&json!("button")]);
    assert_eq!(targets(&answer["extra_effects"]), [&json!("canvas")]);
}

#[test]
fn a_click_the_page_reports_untrusted_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let effects = proving_ground.url("1.canvas.test", "/effects");
    let posted = proving_ground.post_json(
        &effects,
        &json!({ "event": "click", "target": "button", "trusted": false }),
    );

    let answer = proving_ground.judge(&submission("canvas/1", "success", &[]));

    assert_eq!(posted.status, 204);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn a_stroke_drawn_and_read_back_through_evaluate_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "canvas/2", "surface-stroke", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_surface_read_back_without_its_stroke_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "canvas/2", "surface-unread", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(targets(&answer["missing_effects"]), [&json!("surface")]);
    let submitted: Vec<&Value> = answer["wrong"]
        .as_array()
        .expect("`wrong` is an array")
        .iter()
        .map(|wrong| &wrong["submitted"])
        .collect();
    assert_eq!(
        submitted,
        [&json!("#ffffff"), &json!("#ffffff")],
        "{answer}"
    );
}
