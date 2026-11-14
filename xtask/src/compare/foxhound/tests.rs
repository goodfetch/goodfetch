use super::settings;
use crate::compare::matrix::Scenario;
use crate::rss::crawl::Site;

const SITE: Site<'static> = Site {
    address: "127.0.0.1:8080",
    pages: 10_000,
    concurrency: 16,
};

/// The polite scenario's main row forces the `http` tier, as the other
/// tools fetch; its `auto` row leaves `fetch.tier` at its default, so the
/// crawl samples each domain in a browser (#2283).
#[test]
fn the_polite_auto_row_leaves_the_tier_at_its_default() {
    let forced = settings(&SITE, Scenario::Polite, ("full", None), false);
    let auto = settings(&SITE, Scenario::Polite, ("full", None), true);

    assert_eq!(forced["fetch"]["tier"], "http");
    assert!(auto["fetch"].get("tier").is_none());
    assert_eq!(auto["politeness"], forced["politeness"]);
}
