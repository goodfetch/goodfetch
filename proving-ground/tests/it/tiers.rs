//! The `tiers` family: on one host, `/products/<n>` lists its products in
//! its own markup, `/blog/<n>` leaves its list empty for its script to
//! fill, and `/mixed/<n>` is one or the other by the parity of `n`; any
//! other path answers 404.
//! Covers: story:2-33

use crate::crawler::field;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 926;

#[test]
fn products_are_in_the_markup_and_blog_posts_only_in_the_script() {
    let proving_ground = ProvingGround::start(SEED);
    let page = |path: &str| {
        let answer = proving_ground.get(&proving_ground.url("tiers.test", path));
        (
            answer.status,
            answer.body.matches("<li class=\"product\">").count(),
            field(&answer.body, r#"data-names="([^"]+)""#).map(|names| names.split('|').count()),
        )
    };

    assert_eq!(
        [
            page("/products/1"),
            page("/blog/1"),
            page("/mixed/1"),
            page("/mixed/2"),
            page("/products/01"),
            page("/products/100"),
            page("/other/1"),
        ],
        [
            (200, 3, None),
            (200, 0, Some(3)),
            (200, 3, None),
            (200, 0, Some(3)),
            (404, 0, None),
            (404, 0, None),
            (404, 0, None),
        ]
    );
}

/// `?hold=<ms>`, from 1 to 5000, holds the answer that long and names the
/// hold in `Server-Timing`, as the `pacing` family does, so a fetch on one
/// tier is still in flight when a fetch on another starts; a hold out of
/// range is ignored.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test measures how long the Proving ground holds a request, outside any engine's Clock (ADR 0059)"
)]
fn a_hold_in_the_query_holds_the_answer_and_names_it_in_server_timing() {
    let proving_ground = ProvingGround::start(SEED);
    let asked = std::time::Instant::now();
    let held = proving_ground.get(&proving_ground.url("tiers.test", "/blog/2?hold=400"));
    let waited = asked.elapsed();
    let unheld = |path: &str| {
        let answer = proving_ground.get(&proving_ground.url("tiers.test", path));
        (
            answer.status,
            answer.header("server-timing").map(str::to_owned),
        )
    };

    assert_eq!(
        (
            held.status,
            held.header("server-timing"),
            field(&held.body, r#"data-names="([^"]+)""#).is_some(),
            waited >= std::time::Duration::from_millis(400),
            unheld("/products/2"),
            unheld("/products/2?hold=0"),
            unheld("/products/2?hold=5001"),
        ),
        (
            200,
            Some("hold;dur=400"),
            true,
            true,
            (200, None),
            (200, None),
            (200, None),
        )
    );
}

/// The states page holds one visible heading and, in its own markup, an
/// element hidden by each of the `hidden` attribute, an ancestor's inline
/// `display: none` and an inline `visibility:hidden`, beside a link to a
/// static listing; it has no script.
#[test]
fn the_states_page_hides_its_elements_in_its_own_markup() {
    let proving_ground = ProvingGround::start(SEED);
    let answer = proving_ground.get(&proving_ground.url("tiers.test", "/states"));
    let holds = |text: &str| answer.body.contains(text);

    assert_eq!(
        (
            answer.status,
            answer.body.matches("<h1>").count(),
            holds("id=\"hidden-attribute\" hidden"),
            holds("display : none\"><span id=\"in-display-none\""),
            holds("id=\"visibility-hidden\" style=\"visibility:hidden\""),
            holds("href=\"/products/2\""),
            holds("<script"),
        ),
        (200, 1, true, true, true, true, false)
    );
}

/// The hash-routed app holds no view in its markup, only an empty
/// `h1.view` its script fills from the fragment, links to `#/a` and `#/b`,
/// and answers the same document whatever fragment the URL had, since a
/// fragment never reaches the server (pitfall U05).
#[test]
fn the_routes_page_leaves_its_view_to_its_script_on_every_fragment() {
    let proving_ground = ProvingGround::start(SEED);
    let answer = proving_ground.get(&proving_ground.url("tiers.test", "/routes"));
    let holds = |text: &str| answer.body.contains(text);

    assert_eq!(
        (
            answer.status,
            holds("<h1 class=\"view\"></h1>"),
            holds("href=\"#/a\""),
            holds("href=\"#/b\""),
            holds("location.hash"),
            holds("hashchange"),
        ),
        (200, true, true, true, true, true)
    );
}

/// `/painted/<n>` holds no text of its own in `p.painted`: only its
/// script, painting a canvas, writes the alpha it reads there.
#[test]
fn a_painted_page_leaves_its_text_to_its_canvas_script() {
    let proving_ground = ProvingGround::start(SEED);
    let answer = proving_ground.get(&proving_ground.url("tiers.test", "/painted/1"));

    assert_eq!(
        (
            answer.status,
            answer.body.contains("<p class=\"painted\"></p>"),
            answer.body.contains("getImageData(0, 0, 1, 1).data[3]"),
        ),
        (200, true, true)
    );
}
