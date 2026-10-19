use goodfetch_engine::{Channel, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::entries::operations_of;

use super::composition;

#[test]
fn collies_composition_has_exactly_one_handler_for_each_operation_of_collies_catalog() {
    let mut handled: Vec<&str> = composition().operations().collect();
    let mut catalog: Vec<&str> = operations_of(Product::Collie).map(|op| op.name).collect();

    handled.sort_unstable();
    catalog.sort_unstable();
    assert_eq!(handled, catalog);
}

#[test]
fn collies_composition_starts_an_engine() {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").unwrap(),
        channel: Channel::CratesIo,
    };

    assert!(Engine::new(identity, &composition()).is_ok());
}
