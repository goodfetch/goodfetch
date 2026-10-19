//! The `catalog` family's pages: generated on request from the seed and the
//! page's place, so the same seed always serves the same bytes and another
//! seed other ones (spec #152, Determinism).
//! Covers: story:2-28, story:2-30

use proptest::prelude::*;

use crate::proving_ground::ProvingGround;

/// A listing page (`/catalog/1` to `/catalog/10`) or a product page
/// (`/products/P-0001` to `/products/P-0100`) of the `catalog` family.
fn catalog_page() -> impl Strategy<Value = String> {
    prop_oneof![
        (1..=10_u32).prop_map(|page| format!("/catalog/{page}")),
        (1..=100_u32).prop_map(|product| format!("/products/P-{product:04}")),
    ]
}

fn page(proving_ground: &ProvingGround, path: &str) -> String {
    let answer = proving_ground.get(&format!(
        "http://catalog.test:{}{path}",
        proving_ground.port()
    ));
    assert_eq!(answer.status, 200, "{path}: {}", answer.body);
    answer.body
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn the_same_seed_serves_byte_identical_pages(seed: u64, path in catalog_page()) {
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(seed);

        let pages = (page(&first, &path), page(&second, &path));

        prop_assert_eq!(pages.0, pages.1);
    }

    #[test]
    fn another_seed_serves_other_pages(seed: u64, other: u64, path in catalog_page()) {
        prop_assume!(seed != other);
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(other);

        let pages = (page(&first, &path), page(&second, &path));

        prop_assert_ne!(pages.0, pages.1);
    }
}
