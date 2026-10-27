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

/// One element's attributes, as markup writes them.
fn attributes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// What a served document's markup alone says of each fixture link of the
/// Proving ground's `traps` pages, the link's attributes first, then its
/// holder's: a page fetched over HTTP has no layout, so only what its
/// attributes and inline styles state is read, and a link placed off the
/// page or clipped by a style sheet reads as seen.
///
/// Covers: story:8-228
#[rstest]
#[case::a_plain_link(&[("href", "/items/I-0001")], &[], SEEN)]
#[case::a_visible_nofollow_link(&[("href", "/x"), ("rel", "nofollow noopener")], &[], Seen { nofollow: true, ..SEEN })]
#[case::hidden(&[("href", "/hp/1"), ("hidden", "")], &[], Seen { no_box: true, ..SEEN })]
#[case::display_none(&[("href", "/hp/2"), ("style", "display:none")], &[], Seen { no_box: true, ..SEEN })]
#[case::display_none_spaced_and_upper_case(&[("href", "/hp/2"), ("style", "color: red; DISPLAY : None !important")], &[], Seen { no_box: true, ..SEEN })]
#[case::visibility_hidden(&[("href", "/hp/3"), ("style", "visibility:hidden")], &[], Seen { visibility_hidden: true, ..SEEN })]
#[case::opacity_zero(&[("href", "/hp/5"), ("style", "opacity:0")], &[], Seen { transparent: true, ..SEEN })]
#[case::opacity_zero_point_zero(&[("href", "/hp/5"), ("style", "opacity: 0.0")], &[], Seen { transparent: true, ..SEEN })]
#[case::opacity_half_is_seen(&[("href", "/x"), ("style", "opacity:0.5")], &[], SEEN)]
#[case::in_an_aria_hidden_holder(
    &[("href", "/hp/4"), ("tabindex", "-1")],
    &[("aria-hidden", "true"), ("style", "position:absolute;left:-10000px")],
    Seen { aria_hidden: true, ..SEEN }
)]
#[case::in_a_hidden_holder(&[("href", "/x")], &[("style", "display: none")], Seen { no_box: true, ..SEEN })]
#[case::aria_hidden_false_is_seen(&[("href", "/x"), ("aria-hidden", "false")], &[], SEEN)]
#[case::invisible_nofollow(
    &[("href", "/hp/7"), ("rel", "nofollow"), ("style", "display:none")],
    &[],
    Seen { no_box: true, nofollow: true, ..SEEN }
)]
#[case::moved_off_the_page_reads_as_seen(&[("href", "/hp/8"), ("style", "position:absolute;top:-5000px")], &[], SEEN)]
fn a_served_documents_markup_states_what_it_can_of_a_link(
    #[case] link: &[(&str, &str)],
    #[case] holder: &[(&str, &str)],
    #[case] expected: Seen,
) {
    let (link, holder) = (attributes(link), attributes(holder));

    assert_eq!(
        Seen::in_markup([link.as_slice(), holder.as_slice()]),
        expected
    );
}
