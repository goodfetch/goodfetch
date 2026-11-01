use std::collections::BTreeSet;

use goodfetch_engine::secrets;
use goodfetch_engine::{Channel, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::entries::operations_of;
use goodfetch_protocol::secret_reference::Provider;

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

/// ADR 0015's list of V1 providers, every scheme a Secret reference names,
/// against those Collie's engine registers: what it carries and what its
/// composition builds for each Run. Only the caller's (#652) and a Secret
/// provider Plugin's (area 7) come from elsewhere; a scheme missing from
/// both fails here.
/// Covers: story:4-50
#[test]
fn collies_engine_registers_a_provider_for_each_scheme_of_adr_0015s_list() {
    let elsewhere = [Provider::Caller, Provider::Plugin];
    let registered: BTreeSet<Provider> = secrets::CARRIED
        .into_iter()
        .chain(
            composition()
                .secret_providers()
                .into_iter()
                .map(|(provider, _)| provider),
        )
        .collect();

    let missing: Vec<&str> = Provider::ALL
        .into_iter()
        .filter(|provider| !elsewhere.contains(provider) && !registered.contains(provider))
        .map(Provider::as_str)
        .collect();

    assert_eq!(missing, Vec::<&str>::new());
}
