//! The Rust client crate's Connection over the OS's network (spec #606):
//! its `transport` starts Collie's executable as an Attached engine, says
//! `hello`, makes a call and closes, and the engine exits at the end of
//! its input.
//! Covers: story:11-60

use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};
use goodfetch_test_support::TestEnv;

use crate::COLLIE;

#[test]
fn a_connection_starts_an_attached_engine_calls_it_and_closes() {
    let env = TestEnv::new();
    let engine = Endpoint::Attached(AttachedEngine {
        executable: COLLIE.into(),
        environment: env
            .env()
            .iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
    });
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let hello = HelloParams {
        needs: Some(Product::Collie),
        ..HelloParams::new(Surface::Rust, version.clone())
    };
    let runtime = goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (connection, said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &engine,
            hello,
        )
        .await
        .unwrap();
        let info: EngineInfo = connection
            .call(ENGINE_INFO.name, &EngineInfoParams::default())
            .await
            .unwrap();
        connection.close().await.unwrap();

        assert_eq!(said.product, Product::Collie);
        assert_eq!(info.product_version, version);
    });
}
