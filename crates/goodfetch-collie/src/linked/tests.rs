//! A linked connection whose `hello` needs a Product its engine does not
//! contain, as a Foxhound program that met a Collie engine would: the
//! engine refuses it at connect with `not_in_product` (ADRs 0087, 0090 and
//! 0095).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie_client::ErrorCode;
use goodfetch_engine::Channel;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NotInProductDetails;
use goodfetch_test_support::{TestEnv, real_time_runtime};

use super::{Composed, ConnectOptions, ProductConnection, version};

/// The engine fails `hello` before any other call, with its Product and
/// version, the Product needed, and a fix that names the install command
/// of the crates.io channel the crate came from.
/// Covers: story:11-1005, error:not_in_product
#[test]
fn a_connection_that_needs_foxhound_fails_on_a_collie_engine_with_not_in_product() {
    let env = TestEnv::new();
    let options = ConnectOptions::default().environment(env.env_os());

    let failed = real_time_runtime().block_on(ProductConnection::connect_needing(
        Composed::COLLIE,
        Product::Foxhound,
        options,
    ));

    let Err(error) = failed else {
        panic!("connected: {failed:?}");
    };
    assert_eq!(error.code(), ErrorCode::NOT_IN_PRODUCT);
    let details: NotInProductDetails = error.details().unwrap();
    let install = Channel::CratesIo.install_command(Product::Foxhound);
    assert_eq!(
        (
            details.product,
            &details.product_version,
            details.needs,
            details.requested.as_deref(),
            details.install.as_deref(),
        ),
        (
            Product::Collie,
            &version(),
            Product::Foxhound,
            None,
            Some(install.as_str()),
        )
    );
    assert!(details.fix.contains(&install), "{details:?}");
}
