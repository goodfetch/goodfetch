//! The `catalog` family's pages: generated on request from the seed and the
//! page's place, so the same seed always serves the same bytes and another
//! seed other ones (spec #152, Determinism), each page at one URL.

use proptest::prelude::*;
use rstest::rstest;

use crate::proving_ground::{Answer, ProvingGround};

/// A listing page (`/catalog/1` to `/catalog/10`) or a product page
/// (`/products/P-0001` to `/products/P-0100`) of the `catalog` family.
fn catalog_page() -> impl Strategy<Value = String> {
    prop_oneof![
        (1..=10_u32).prop_map(|page| format!("/catalog/{page}")),
        (1..=100_u32).prop_map(|product| format!("/products/P-{product:04}")),
    ]
}

fn fetch(proving_ground: &ProvingGround, path: &str) -> Answer {
    proving_ground.get(&format!(
        "http://catalog.test:{}{path}",
        proving_ground.port()
    ))
}

fn page(proving_ground: &ProvingGround, path: &str) -> String {
    let answer = fetch(proving_ground, path);
    assert_eq!(answer.status, 200, "{path}: {}", answer.body);
    answer.body
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// A page depends on the seed and its place only: the first Proving
    /// ground serves another page before the compared one, the second none.
    ///
    /// Covers: story:2-28, story:2-30
    #[test]
    fn the_same_seed_serves_byte_identical_pages(
        seed: u64,
        earlier in catalog_page(),
        path in catalog_page(),
    ) {
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(seed);
        page(&first, &earlier);

        let pages = (page(&first, &path), page(&second, &path));

        prop_assert_eq!(pages.0, pages.1);
    }

    /// Covers: story:2-28, story:2-30
    #[test]
    fn another_seed_serves_other_pages(seed: u64, other: u64, path in catalog_page()) {
        prop_assume!(seed != other);
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(other);

        let pages = (page(&first, &path), page(&second, &path));

        prop_assert_ne!(pages.0, pages.1);
    }
}

#[rstest]
#[case::listing_before_the_first("/catalog/0")]
#[case::listing_after_the_last("/catalog/11")]
#[case::listing_with_a_leading_zero("/catalog/01")]
#[case::product_after_the_last("/products/P-0101")]
#[case::product_with_a_sign("/products/P-+001")]
#[case::product_with_five_digits("/products/P-00001")]
#[case::outside_the_family("/tasks")]
fn a_page_the_family_does_not_have_is_not_found(#[case] path: &str) {
    let proving_ground = ProvingGround::start(158);

    let answer = fetch(&proving_ground, path);

    assert_eq!(answer.status, 404, "{}", answer.body);
}

#[rstest]
#[case::the_crawl_tasks_host("1.catalog.test")]
#[case::the_extract_tasks_host("2.catalog.test")]
fn a_tasks_host_serves_the_pages_of_its_family(#[case] host: &str) {
    let proving_ground = ProvingGround::start(158);

    let answer = proving_ground.get(&format!("http://{host}/catalog/3"));

    assert_eq!(answer.body, page(&proving_ground, "/catalog/3"));
}

#[test]
fn a_task_the_family_does_not_have_has_no_host() {
    let proving_ground = ProvingGround::start(158);

    let answer = proving_ground.get("http://99.catalog.test/catalog/3");

    assert_eq!(answer.status, 404, "{}", answer.body);
}
