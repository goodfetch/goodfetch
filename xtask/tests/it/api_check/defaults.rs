//! A changed default of a setting with a value that weakens a guarantee
//! (ADR 0030): a break when it moves toward the values that weaken, an
//! addition when it moves away, read from the previous release's
//! `weakening` (issue 148). Where that release says no direction, or
//! predates the field, any changed default is a break.

use serde_json::{Value, json};

use super::{COLLIE, FOXHOUND, edit_json, entry, expected, released_with, setting};
use crate::workspace::Workspace;

/// A weakening Collie setting released with `default` and `weakening`, the
/// field left out when `weakening` is none, as a release from before the
/// projection carried it.
fn released_weakening(default: &Value, weakening: Option<&Value>) -> Workspace {
    released_with(
        |workspace| {
            for path in [COLLIE, FOXHOUND] {
                edit_json(workspace, path, |projection| {
                    let mut released = setting();
                    released["weakens"] = json!(true);
                    released["default"] = default.clone();
                    if let Some(weakening) = weakening {
                        released["weakening"] = weakening.clone();
                    }
                    projection["settings"]
                        .as_array_mut()
                        .unwrap()
                        .push(released);
                });
            }
        },
        "",
    )
}

/// Changes the tree's `sample.max_pages` with `change`.
fn edit_setting(workspace: &Workspace, change: impl FnOnce(&mut Value)) {
    edit_json(workspace, COLLIE, |projection| {
        change(entry(projection, "settings", "sample.max_pages"));
    });
}

#[rstest::rstest]
#[case::a_count_raised_away_from_weakening(json!(10), json!(20), json!({"toward": "lower"}), false)]
#[case::a_count_lowered_toward_weakening(json!(10), json!(5), json!({"toward": "lower"}), true)]
#[case::a_count_raised_toward_weakening(json!(10), json!(20), json!({"toward": "higher"}), true)]
#[case::a_count_lowered_away_from_weakening(json!(10), json!(5), json!({"toward": "higher"}), false)]
#[case::a_choice_moved_to_a_weakening_value(json!("detect"), json!("off"), json!({"values": ["off"]}), true)]
#[case::a_choice_moved_to_another_value(json!("detect"), json!("answer"), json!({"values": ["off"]}), false)]
#[case::a_switch_turned_on(json!(false), json!(true), json!({"values": [false]}), false)]
#[case::no_direction(json!(10), json!(20), Value::Null, true)]
fn a_changed_default_of_a_weakening_setting_breaks_by_its_direction(
    #[case] before: Value,
    #[case] after: Value,
    #[case] weakening: Value,
    #[case] breaks: bool,
) {
    let workspace = released_weakening(&before, Some(&weakening));
    edit_setting(&workspace, |setting| setting["default"] = after);

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        expected(breaks, "collie settings sample.max_pages"),
        "{}",
        outcome.stdout
    );
}

/// A release from before the projection carried `weakening` published no
/// direction, so the tree's own cannot pass a changed default: the change
/// that adds the direction may also move what weakens.
#[test]
fn a_release_without_a_direction_holds_every_changed_default() {
    let workspace = released_weakening(&json!(10), None);
    edit_setting(&workspace, |setting| {
        setting["weakening"] = json!({"toward": "lower"});
        setting["default"] = json!(20);
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        expected(true, "collie settings sample.max_pages"),
        "{}",
        outcome.stdout
    );
}

/// The direction itself, once a release has carried it, a null one
/// included: a change to it is a break.
#[rstest::rstest]
#[case::reversed(json!({"toward": "lower"}), json!({"toward": "higher"}))]
#[case::gained(Value::Null, json!({"toward": "lower"}))]
fn a_changed_weakening_direction_is_a_break(#[case] released: Value, #[case] changed: Value) {
    let workspace = released_weakening(&json!(10), Some(&released));
    edit_setting(&workspace, |setting| setting["weakening"] = changed);

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        expected(true, "collie settings sample.max_pages"),
        "{}",
        outcome.stdout
    );
}
