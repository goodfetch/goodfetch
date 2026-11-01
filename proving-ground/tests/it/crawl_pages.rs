//! The Crawl families' pages are generated on request from the seed and the
//! page's place (spec #152, Determinism): the same seed serves the same
//! bytes, whichever `serve` serves them.
//! Covers: story:2-30

use proptest::prelude::*;

use crate::proving_ground::ProvingGround;

/// A page of one of the Crawl families, on its family's own host name.
fn crawl_page() -> impl Strategy<Value = (&'static str, String)> {
    prop_oneof![
        (1..=40_u32).prop_map(|n| ("statuses.test", format!("/articles/A-{n:04}"))),
        (1..=30_u32).prop_map(|n| ("traps.test", format!("/items/I-{n:04}"))),
        (1..=3_u32).prop_map(|n| ("traps.test", format!("/categories/{n}"))),
        ((1..=48_u32), (1..=3_u32)).prop_map(|(n, colour)| (
            "parameters.test",
            format!("/product?id=P-{n:04}&colour={colour}&ref=listing")
        )),
        (1..=24_u32).prop_map(|n| ("limits.test", format!("/items/L-{n:04}"))),
        (1..=16_384_u32).prop_map(|n| ("scale.test", format!("/sections/{n}"))),
        (1..=1_000_u32).prop_map(|n| ("scale.test", format!("/sites/1000/{n}"))),
        (1..=8_u32).prop_map(|n| ("1.links.test", format!("/library/shelves/book/{n}"))),
        (1..=12_u32).prop_map(|n| ("3.links.test", format!("/rooms/R-{n:02}"))),
        (1..=24_u32).prop_map(|n| ("facets.test", format!("/products/{n}"))),
        (1..=24_u32).prop_map(|n| ("fields.test", format!("/listings/{n}"))),
        (1..=50_048_u32).prop_map(|n| ("sitemaps.test", format!("/sitemaps/{n}.xml"))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn the_same_seed_serves_the_pages_of_each_crawl_family_byte_for_byte(
        seed: u64,
        (host, path) in crawl_page(),
    ) {
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(seed);
        let page = |proving_ground: &ProvingGround| {
            proving_ground.get(&format!("http://{host}:{}{path}", proving_ground.port()))
        };

        let (one, other) = (page(&first), page(&second));

        prop_assert_eq!(one.status, 200, "{}{}", host, path);
        prop_assert_eq!(one.body, other.body);
    }
}
