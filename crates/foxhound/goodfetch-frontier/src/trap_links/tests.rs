//! `trap_links`' visibility rules in process (spec #600's Testing
//! Decisions), over what [`READ`](super::READ) reads of each fixture link:
//! the honeypot forms of the Proving ground's `traps` family and the links
//! a person sees.

use goodfetch_engine::egress::Policy;
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::json;
use url::Url;

use super::{Mark, Markup, Seen, TABLES, Trap, Verdict, judge, seen_again, suspect, suspected};
use crate::{Bounds, Candidate, Declared, Invariants, Patterns, Scope, ScopeStrategy, admit};

const SEEN: Seen = Seen {
    no_box: false,
    outside_page: false,
    visibility_hidden: false,
    transparent: false,
    aria_hidden: false,
    clipped: false,
    nofollow: false,
    cosmetic: false,
    revealable: false,
};

/// Each fixture link, as the page reads it, and the judgement.
///
/// Covers: story:8-226
#[rstest]
#[case::a_link_a_person_sees(SEEN, Verdict::Seen)]
#[case::a_visible_nofollow_link_is_followed(Seen { nofollow: true, ..SEEN }, Verdict::Seen)]
#[case::hidden_or_display_none_has_no_box(Seen { no_box: true, ..SEEN }, Verdict::Trap(Trap::Invisible))]
#[case::moved_off_the_page(Seen { outside_page: true, ..SEEN }, Verdict::Trap(Trap::Invisible))]
#[case::visibility_hidden(Seen { visibility_hidden: true, ..SEEN }, Verdict::Trap(Trap::Invisible))]
#[case::opacity_zero(Seen { transparent: true, ..SEEN }, Verdict::Trap(Trap::Invisible))]
#[case::in_an_aria_hidden_subtree(Seen { aria_hidden: true, ..SEEN }, Verdict::Trap(Trap::Invisible))]
#[case::clipped_to_nothing(Seen { clipped: true, ..SEEN }, Verdict::Trap(Trap::Invisible))]
#[case::aria_hidden_and_off_the_page(
    Seen { aria_hidden: true, outside_page: true, ..SEEN },
    Verdict::Trap(Trap::Invisible)
)]
#[case::invisible_nofollow(
    Seen { no_box: true, nofollow: true, ..SEEN },
    Verdict::Trap(Trap::InvisibleNofollow)
)]
fn a_link_a_person_cannot_see_is_a_trap(#[case] seen: Seen, #[case] expected: Verdict) {
    assert_eq!(judge(&seen), expected);
}

/// A link hidden in a menu, a tab or a disclosure, which a person opens to
/// see it, is followed, marked as a suspected trap; an invisible `nofollow`
/// one there is still a trap, the signal Cloudflare's AI Labyrinth gives
/// (ADR 0035, owner decision O26); a visible one there is seen.
///
/// Covers: story:8-226
#[rstest]
#[case::hidden_in_a_menu(Seen { no_box: true, revealable: true, ..SEEN }, Verdict::Revealable)]
#[case::off_the_page_in_a_menu(Seen { outside_page: true, revealable: true, ..SEEN }, Verdict::Revealable)]
#[case::aria_hidden_in_a_menu(Seen { aria_hidden: true, revealable: true, ..SEEN }, Verdict::Revealable)]
#[case::invisible_nofollow_in_a_menu(
    Seen { no_box: true, nofollow: true, revealable: true, ..SEEN },
    Verdict::Trap(Trap::InvisibleNofollow)
)]
#[case::visible_in_a_menu(Seen { revealable: true, ..SEEN }, Verdict::Seen)]
fn a_link_a_person_reveals_is_followed_and_marked(#[case] seen: Seen, #[case] expected: Verdict) {
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
        Verdict::Seen
    );
}

/// What the page answers reads back fact by fact; an answer missing one
/// reads as none.
#[test]
fn the_pages_answer_reads_fact_by_fact() {
    let answered = json!({
        "noBox": false, "outsidePage": true, "visibilityHidden": false,
        "transparent": false, "ariaHidden": true, "clipped": false,
        "nofollow": true, "cosmetic": false, "revealable": true,
    });

    assert_eq!(
        Seen::read(&answered),
        Some(Seen {
            outside_page: true,
            aria_hidden: true,
            nofollow: true,
            revealable: true,
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
#[case::aria_hidden_spaced_and_upper_case(&[("href", "/hp/9"), ("aria-hidden", " TRUE ")], &[], Seen { aria_hidden: true, ..SEEN })]
#[case::aria_hidden_past_a_no_break_space_is_seen(&[("href", "/x"), ("aria-hidden", "\u{a0}true")], &[], SEEN)]
fn a_served_documents_markup_states_what_it_can_of_a_link(
    #[case] link: &[(&str, &str)],
    #[case] holder: &[(&str, &str)],
    #[case] expected: Seen,
) {
    let (link, holder) = (attributes(link), attributes(holder));

    assert_eq!(
        Seen::in_markup([
            Markup {
                name: "a",
                attributes: &link,
                controlled: false,
            },
            Markup {
                name: "div",
                attributes: &holder,
                controlled: false,
            },
        ]),
        expected
    );
}

/// A link a holder's markup hides inside a menu, a tab or a disclosure is
/// one a person reveals by opening it: a `nav` or `menu` element, a menu,
/// navigation or tab role, a `details` element, a `popover`, a `hidden`
/// that a find in the page opens (`until-found`), or an element another
/// names in `aria-controls`; a hidden plain holder reveals nothing.
///
/// Covers: story:8-228
#[rstest]
#[case::a_nav("nav", &[("hidden", "")], false, true)]
#[case::a_menu_element("menu", &[("style", "display:none")], false, true)]
#[case::a_details("details", &[], false, true)]
#[case::a_menu_role("ul", &[("role", "menu"), ("hidden", "")], false, true)]
#[case::a_navigation_role_among_others("div", &[("role", "presentation Navigation"), ("hidden", "")], false, true)]
#[case::a_tab_panel("div", &[("role", "tabpanel"), ("hidden", "")], false, true)]
#[case::a_popover("div", &[("popover", "auto")], false, true)]
#[case::hidden_until_found("div", &[("hidden", "UNTIL-FOUND")], false, true)]
#[case::named_by_aria_controls("div", &[("id", "more"), ("hidden", "")], true, true)]
#[case::a_plain_holder("div", &[("hidden", "")], false, false)]
#[case::a_dialog_role_is_no_menu("div", &[("role", "dialog"), ("hidden", "")], false, false)]
fn a_link_hidden_in_a_menu_tab_or_disclosure_reads_revealable(
    #[case] name: &str,
    #[case] holder: &[(&str, &str)],
    #[case] controlled: bool,
    #[case] revealable: bool,
) {
    let (link, holder) = (attributes(&[("href", "/x")]), attributes(holder));

    let seen = Seen::in_markup([
        Markup {
            name: "a",
            attributes: &link,
            controlled: false,
        },
        Markup {
            name,
            attributes: &holder,
            controlled,
        },
    ]);

    assert_eq!(seen.revealable, revealable, "{seen:?}");
}

/// A run file holding the Frontier's tables and the marks, in memory, with
/// `https://shop.test/p/1` added as crawl request 1 and marked as reached
/// through a suspected trap.
fn marked_run_file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in crate::identity::TABLES
        .statements
        .iter()
        .chain(crate::scope::TABLES.statements)
        .chain(crate::bounds::TABLES.statements)
        .chain(crate::rules::TABLES.statements)
        .chain(TABLES.statements)
    {
        connection.execute(statement, []).unwrap();
    }
    let scope = Scope::new(
        ScopeStrategy::SameHostname,
        Patterns::default(),
        Patterns::default(),
    );
    let invariants = Invariants::new(scope, Bounds::default()).unwrap();
    let policy = Policy::new(&["*"], &[] as &[&str]).unwrap();
    let get = HttpMethod::get();
    let added = admit(
        &connection,
        &invariants,
        &policy,
        Candidate::Seed {
            url: "https://shop.test/p/1",
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        },
        |_| Ok("1".to_owned()),
    )
    .unwrap();
    assert!(matches!(added, Enqueued::Added { .. }), "{added:?}");
    suspect(&connection, "1", Mark::Hidden).unwrap();
    connection
}

/// A link a person sees that reaches a marked request again, which the
/// Frontier answers `duplicate`, clears its mark: the request is no longer
/// reached through suspected traps alone (ADR 0035's #1153 note). One to a
/// URL the Frontier never admitted clears nothing.
///
/// Covers: story:8-228
#[rstest]
#[case::the_same_url("/p/1", false)]
#[case::its_fragment_dropped("https://shop.test/p/1#reviews", false)]
#[case::another_url("/p/2", true)]
fn a_link_a_person_sees_to_a_marked_request_clears_its_mark(
    #[case] href: &str,
    #[case] still_marked: bool,
) {
    let connection = marked_run_file();
    let base = Url::parse("https://shop.test/list").unwrap();

    seen_again(&connection, href, &base, encoding_rs::UTF_8, false, false).unwrap();

    assert_eq!(
        suspected(&connection, "1").unwrap(),
        still_marked.then_some(Mark::Hidden)
    );
}

/// A link a person reveals in a menu, tab or disclosure that reaches a
/// request marked through markup alone again leaves it marked, as one a
/// browser tier follows: it is reached through a link a person can open.
///
/// Covers: story:8-228
#[test]
fn a_link_a_person_reveals_to_a_marked_request_lets_the_browser_follow_it() {
    let connection = marked_run_file();
    let base = Url::parse("https://shop.test/list").unwrap();

    seen_again(&connection, "/p/1", &base, encoding_rs::UTF_8, false, true).unwrap();

    assert_eq!(suspected(&connection, "1").unwrap(), Some(Mark::Revealable));
}
