//! `trap_links`' visibility rules in process (spec #600's Testing
//! Decisions), over what [`READ`](super::READ) reads of each fixture link:
//! the honeypot forms of the Proving ground's `traps` family and the links
//! a person sees.

use rstest::rstest;
use serde_json::json;

use super::{Seen, Trap, judge};

const SEEN: Seen = Seen {
    no_box: false,
    outside_page: false,
    visibility_hidden: false,
    transparent: false,
    aria_hidden: false,
    clipped: false,
    nofollow: false,
    cosmetic: false,
};

/// Each fixture link, as the page reads it, and the judgement.
///
/// Covers: story:8-226
#[rstest]
#[case::a_link_a_person_sees(SEEN, None)]
#[case::a_visible_nofollow_link_is_followed(Seen { nofollow: true, ..SEEN }, None)]
#[case::hidden_or_display_none_has_no_box(Seen { no_box: true, ..SEEN }, Some(Trap::Invisible))]
#[case::moved_off_the_page(Seen { outside_page: true, ..SEEN }, Some(Trap::Invisible))]
#[case::visibility_hidden(Seen { visibility_hidden: true, ..SEEN }, Some(Trap::Invisible))]
#[case::opacity_zero(Seen { transparent: true, ..SEEN }, Some(Trap::Invisible))]
#[case::in_an_aria_hidden_subtree(Seen { aria_hidden: true, ..SEEN }, Some(Trap::Invisible))]
#[case::clipped_to_nothing(Seen { clipped: true, ..SEEN }, Some(Trap::Invisible))]
#[case::aria_hidden_and_off_the_page(
    Seen { aria_hidden: true, outside_page: true, ..SEEN },
    Some(Trap::Invisible)
)]
#[case::invisible_nofollow(
    Seen { no_box: true, nofollow: true, ..SEEN },
    Some(Trap::InvisibleNofollow)
)]
fn a_link_a_person_cannot_see_is_a_trap(#[case] seen: Seen, #[case] expected: Option<Trap>) {
    assert_eq!(judge(&seen), expected);
}

/// A link inside an element a cosmetic rule matched, which live hiding
/// hides, is left out of the judgement: a hidden ad's link is no trap.
///
/// Covers: story:8-229
#[rstest]
#[case::visible(SEEN)]
#[case::hidden_by_the_rule(Seen { no_box: true, ..SEEN })]
#[case::hidden_nofollow(Seen { no_box: true, nofollow: true, ..SEEN })]
fn a_link_a_cosmetic_rule_hides_is_judged_as_seen_without_the_rule(#[case] seen: Seen) {
    assert_eq!(
        judge(&Seen {
            cosmetic: true,
            ..seen
        }),
        None
    );
}

/// What the page answers reads back fact by fact; an answer missing one
/// reads as none.
#[test]
fn the_pages_answer_reads_fact_by_fact() {
    let answered = json!({
        "noBox": false, "outsidePage": true, "visibilityHidden": false,
        "transparent": false, "ariaHidden": true, "clipped": false,
        "nofollow": true, "cosmetic": false,
    });

    assert_eq!(
        Seen::read(&answered),
        Some(Seen {
            outside_page: true,
            aria_hidden: true,
            nofollow: true,
            ..SEEN
        })
    );
    assert_eq!(Seen::read(&json!({ "noBox": true })), None);
}
