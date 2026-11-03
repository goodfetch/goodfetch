//! The `scale.test/sites/<size>/` sites (ADR 0134): a site of any size,
//! each page linking only its own site's, which `just compare`
//! crawls with Foxhound and Spider at 1,000 pages. They belong to no task
//! and declare no Feature (#1574).

use std::collections::BTreeSet;

use crate::crawler::Crawler;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 164;

/// The pages a crawl from `/sites/<size>/1` reached, as sorted URLs.
fn site_crawl(proving_ground: &ProvingGround, size: u64) -> (usize, BTreeSet<String>) {
    let origin = format!("http://scale.test:{}", proving_ground.port());
    let mut crawler = Crawler::new(proving_ground);
    // The site's pages have no script, so the crawl reads only `<a href>`s.
    crawler.follow_scripts = false;
    let pages = crawler.crawl(&format!("{origin}/sites/{size}/1"));
    let urls = pages.iter().map(|page| page.url.clone()).collect();
    (pages.len(), urls)
}

/// The URLs of the pages of the site of `size` pages.
fn site_pages(proving_ground: &ProvingGround, size: u64) -> BTreeSet<String> {
    (1..=size)
        .map(|page| {
            format!(
                "http://scale.test:{}/sites/{size}/{page}",
                proving_ground.port()
            )
        })
        .collect()
}

/// The 1,000-page site Spider's comparison crawls (ADR 0134): a crawl of
/// its origin from `/sites/1000/1` reaches each of its pages once, each
/// answered `200` with no redirect, and nothing else; the page past its
/// last is none.
#[test]
fn a_crawl_of_the_thousand_page_site_reaches_each_page_once_and_nothing_else() {
    let proving_ground = ProvingGround::start(SEED);

    let crawled = site_crawl(&proving_ground, 1000);
    let past = proving_ground.get(&format!(
        "http://scale.test:{}/sites/1000/1001",
        proving_ground.port()
    ));

    assert_eq!(
        (crawled, past.status),
        ((1000, site_pages(&proving_ground, 1000)), 404)
    );
}

/// A site's size is in its path, so a smaller site holds fewer pages,
/// each linking only its own site's.
#[test]
fn a_site_of_another_size_holds_that_many_pages() {
    let proving_ground = ProvingGround::start(SEED);

    let crawled = site_crawl(&proving_ground, 25);

    assert_eq!(crawled, (25, site_pages(&proving_ground, 25)));
}
