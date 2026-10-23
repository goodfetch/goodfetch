//! The Consent prompt runner's bundle (ADR 0048): autoconsent's library,
//! heuristics and rules with Goodfetch's runner and snippet table, as the
//! engine evaluates it in the guard's isolated world of every frame.

use goodfetch_clean_up_rules::consent::{self, Runner, World};
use rstest::rstest;

/// Each snippet runs in the world the snippet table classes it in: those
/// that read or call a vendor's own JavaScript in the main world, those
/// that reach only the DOM, cookies or storage in the isolated world, and
/// one that clicks through a synthetic event nowhere.
/// Covers: story:5-100
#[rstest]
#[case::cookiebots_withdraw("EVAL_COOKIEBOT_3", World::Main)]
#[case::onetrusts_active_groups("EVAL_ONETRUST_1", World::Main)]
#[case::usercentrics_api("EVAL_USERCENTRICS_API_2", World::Main)]
#[case::moove_checkboxes("EVAL_MOOVE_0", World::Isolated)]
#[case::a_cookie_read("EVAL_BORLABS_0", World::Isolated)]
#[case::a_synthetic_click("EVAL_COOKIE_POPUP_CLOSE", World::Dropped)]
fn each_snippet_runs_in_the_world_the_table_classes_it_in(
    #[case] snippet: &str,
    #[case] world: World,
) {
    assert_eq!(consent::world(snippet), Some(world));
}

/// A snippet the pinned library does not have is in no world.
/// Covers: story:5-100
#[test]
fn a_snippet_the_library_does_not_have_is_in_no_world() {
    assert_eq!(consent::world("EVAL_NOT_A_SNIPPET"), None);
}

/// A rule that names a dropped snippet is left out of the bundle, by the
/// snippet it names, and every other rule is in it.
/// Covers: story:5-100
#[test]
fn a_rule_that_names_a_dropped_snippet_is_left_out_of_the_bundle() {
    let runner = Runner::embedded().expect("the embedded runner decompresses");
    assert_eq!(
        (
            consent::dropped_rules(),
            runner.source().contains("\"cookie-popup-close-cookies\""),
            runner.source().contains("\"didomi\""),
        ),
        (&["cookie-popup-close-cookies"][..], false, true)
    );
}

/// autoconsent's library and heuristics reach the bundle as their authors
/// published them (ADR 0018), followed by Goodfetch's runner.
/// Covers: story:5-100
#[test]
fn the_bundle_carries_autoconsents_files_unmodified_and_the_runner() {
    let runner = Runner::embedded().expect("the embedded runner decompresses");
    let source = runner.source();
    assert!(source.contains(include_str!("../../consent/autoconsent/autoconsent.cjs.js")));
    assert!(source.contains(include_str!("../../consent/autoconsent/heuristics.cjs.js")));
    assert!(source.contains(include_str!("../../consent/runner.js")));
}
