//! A Connection to a Daemon over the OS's network (spec #606, ADR 0024):
//! the client's `transport` opens the Unix socket the Daemon's endpoint
//! names, and the Connection's lines cross it both ways.
//! Covers: story:11-60

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{Connection, ConnectionRuntime};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams, ProductVersion, Surface};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

#[test]
fn a_connection_to_a_daemon_crosses_its_unix_socket() {
    let folder = tempfile::tempdir().unwrap();
    let socket = folder.path().join("daemon.sock");
    let version = ProductVersion::parse("0.0.0").unwrap();
    let hello = HelloParams {
        needs: Some(Product::Collie),
        token: Some("token".to_owned()),
        ..HelloParams::new(Surface::Rust, version)
    };
    let runtime = goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        #[expect(
            clippy::disallowed_types,
            reason = "the test stands in for the Daemon, which listens on its socket"
        )]
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let daemon = async {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut lines = BufReader::new(reader).lines();
            let said: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let mut reply = serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": said["id"],
                "result": {
                    "protocol_version": "0.1",
                    "product": "collie",
                    "product_version": "0.0.0",
                    "engine_instance": "01J0000000000000000000000",
                },
            }))
            .unwrap();
            reply.push(b'\n');
            writer.write_all(&reply).await.unwrap();
            assert!(
                lines.next_line().await.unwrap().is_none(),
                "closing ends the input"
            );
            writer.shutdown().await.unwrap();
            said
        };
        let client = async {
            let (connection, said) = Connection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &Network::Os,
                &Endpoint::Daemon(DaemonEndpoint::Unix(socket.clone())),
                hello,
            )
            .await
            .unwrap();
            connection.close().await.unwrap();
            said
        };
        let (heard, said) = tokio::join!(daemon, client);

        assert_eq!(heard["method"], "hello");
        assert_eq!(heard["params"]["token"], "token");
        assert_eq!(said.product, Product::Collie);
    });
}
