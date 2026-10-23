//! The check finds each construct ADR 0128 refuses, and leaves a table's
//! own tests alone.

use rstest::rstest;

use super::refused_matches;

const DENIED: &str = "#![deny(clippy::wildcard_enum_match_arm)]\n";

/// Covers: story:3-151, story:3-152
#[rstest]
#[case::a_wildcard_arm(
    "fn apply(state: S) -> u8 { match state { S::A => 1, _ => 2 } }",
    vec!["a `_` arm"]
)]
#[case::a_wildcard_inside_an_or_pattern(
    "fn apply(state: S) -> u8 { match state { S::A => 1, S::B | _ => 2 } }",
    vec!["a `_` arm"]
)]
#[case::a_binding_that_catches_every_value(
    "fn apply(state: S) -> S { match state { S::A => S::B, other => other } }",
    vec!["an arm that binds every value"]
)]
#[case::a_match_on_a_tuple(
    "fn apply(state: S, event: E) -> u8 { match (state, event) { (S::A, E::X) => 1, (S::B, E::X) => 2 } }",
    vec!["a `match` on a tuple"]
)]
#[case::an_if_let(
    "fn apply(state: S) -> u8 { if let S::A = state { 1 } else { 2 } }",
    vec!["an `if let` or a `let` chain"]
)]
#[case::a_let_else(
    "fn apply(state: S) -> u8 { let S::A = state else { return 2 }; 1 }",
    vec!["a `let … else`"]
)]
#[case::a_matches_macro(
    "fn apply(state: S) -> bool { matches!(state, S::A) }",
    vec!["a `matches!`"]
)]
fn the_check_finds_each_match_that_could_let_a_new_case_through(
    #[case] table: &str,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(refused_matches(&format!("{DENIED}{table}")), expected);
}

/// Covers: story:3-151, story:3-152
#[test]
fn the_check_finds_a_module_that_does_not_deny_the_lint() {
    let source = "#![deny(clippy::pedantic)]\nfn apply(state: S) -> u8 { match state { S::A => 1, S::B => 2 } }";

    assert_eq!(
        refused_matches(source),
        ["the module does not deny clippy::wildcard_enum_match_arm"]
    );
}

/// The table's tests may match as they like.
///
/// Covers: story:3-151, story:3-152
#[test]
fn the_check_leaves_out_the_tables_tests() {
    let source = format!(
        "{DENIED}fn apply(state: S) -> u8 {{ match state {{ S::A => 1, S::B => 2 }} }}\n#[cfg(test)]\nmod tests {{ fn any(state: S) -> bool {{ matches!(state, _) }} }}"
    );

    assert_eq!(refused_matches(&source), Vec::<&str>::new());
}
