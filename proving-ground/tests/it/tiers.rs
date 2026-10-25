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
