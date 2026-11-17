//! Every transition table of the crate matches its state, then its event,
//! with no wildcard (ADR 0128), as `goodfetch-test-support`'s
//! `refused_matches` reads its module.

use goodfetch_test_support::refused_matches;
use rstest::rstest;

/// Each module that holds a transition table, by its path in the crate.
const TABLES: [(&str, &str); 3] = [
    ("runs::lifecycle", include_str!("../runs/lifecycle.rs")),
    ("effect::lifecycle", include_str!("../effect/lifecycle.rs")),
    ("transition::engine", include_str!("engine.rs")),
];

/// Covers: story:3-151, story:3-152
#[rstest]
#[case::the_runs_table(TABLES[0])]
#[case::the_effects_table(TABLES[1])]
#[case::the_engines_table(TABLES[2])]
fn a_transition_table_matches_its_state_and_event_with_no_wildcard(
    #[case] (module, source): (&str, &str),
) {
    assert_eq!(refused_matches(source), Vec::<&str>::new(), "{module}");
}
