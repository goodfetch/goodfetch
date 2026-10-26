use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use rstest::rstest;

use super::hello;

/// Covers: story:12-232
#[rstest]
#[case::collies_engine(Product::Collie)]
#[case::foxhounds_engine(Product::Foxhound)]
fn the_gateways_hello_says_it_is_the_gateway_and_announces_the_engines_product(
    #[case] product: Product,
) {
    let sent = hello(product, None, None, None);

    assert_eq!((sent.surface, sent.needs), (Surface::Cloud, Some(product)));
}
