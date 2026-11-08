//! The relay's one edit, at its interface over in-memory Connections: the
//! Client's `hello` reaches the Daemon with the Daemon's token, whatever else
//! it says.

use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::jsonrpc::META;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::relay;
use crate::transport::{self, Connection, Network};

const TOKEN: &str = "the-daemons-token";

/// A Connection's two ends.
async fn connection() -> (Connection, Connection) {
    let (network, dialer) = transport::in_memory();
    let near = dialer.connect().await.unwrap();
    let far = Network::InMemory(network)
        .attached_connection()
        .await
        .unwrap();
    (near, far)
}

/// The params of a current `hello`.
fn current_hello() -> Value {
    serde_json::to_value(HelloParams::new(
        Surface::Rust,
        ProductVersion::parse("0.0.0").unwrap(),
    ))
    .unwrap()
}

/// The params the Daemon reads of a `hello` the Client sent with `params`.
fn relayed(params: &Value) -> Value {
    goodfetch_test_support::runtime().block_on(async {
        let (client, client_end) = connection().await;
        let (daemon_end, daemon) = connection().await;
        let (_client_reader, mut to_relay) = client.split();
        let (from_relay, _daemon_writer) = daemon.split();
        let hello = json!({ "jsonrpc": "2.0", "id": 0, "method": HELLO, "params": params });
        let talking = async {
            to_relay
                .write_all(format!("{hello}\n").as_bytes())
                .await
                .unwrap();
            let mut lines = BufReader::new(from_relay).lines();
            lines.next_line().await.unwrap().unwrap()
        };
        tokio::select! {
            () = relay(client_end, daemon_end, TOKEN) => panic!("the relay ended first"),
            line = talking => {
                serde_json::from_str::<Value>(&line).unwrap()["params"].clone()
            }
        }
    })
}

/// A Client newer than the relay, in front of a Daemon that knows its field,
/// is served: the relay adds the token to a `hello` it cannot read.
#[test]
fn a_hello_with_a_field_this_build_does_not_know_reaches_the_daemon_with_the_token() {
    let mut params = current_hello();
    params["from_a_newer_client"] = json!(true);

    let read = relayed(&params);

    assert_eq!(
        (read["token"].clone(), read["from_a_newer_client"].clone()),
        (json!(TOKEN), json!(true))
    );
}

/// The token sits where the Daemon reads it, and the Trace context stays
/// beside the `hello`'s params.
#[test]
fn a_current_hello_reads_with_the_token_and_keeps_its_trace_context() {
    let mut params = current_hello();
    params[META] = json!({ "traceparent": "the-trace-context" });

    let mut read = relayed(&params);
    let meta = read.as_object_mut().unwrap().remove(META);
    let hello: HelloParams = serde_json::from_value(read).unwrap();

    assert_eq!(
        (hello.token.as_deref(), meta),
        (
            Some(TOKEN),
            Some(json!({ "traceparent": "the-trace-context" }))
        )
    );
}
