//! An Attached engine in the per-install layout (`run_state.layout`, ADR
//! 0002), which hands its Runs to the Daemon: it relays its Client's
//! Connection to the Daemon of its major, byte for byte, but for the
//! Client's `hello`, to which it adds the Daemon's token. Each Run the
//! Client starts is then the Daemon's, written into its install database,
//! and stops when the Client's Connection ends, as an Attached engine's
//! does (ADR 0078).

use goodfetch_protocol::connection::{HELLO, HelloParams};
use goodfetch_protocol::jsonrpc::META;
use serde_json::Value;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};

use crate::rpc::framing::MAX_MESSAGE_BYTES;
use crate::transport::Connection;

/// Relays `client` to `daemon` until either ends, adding `token` to the
/// Client's first message when it is a `hello`.
///
/// # Cancel safety
///
/// Dropping the future closes both Connections.
pub(crate) async fn relay(client: Connection, daemon: Connection, token: &str) {
    let (client_reader, mut client_writer) = client.split();
    let (mut daemon_reader, mut daemon_writer) = daemon.split();
    let mut client_reader = BufReader::new(client_reader);
    let Some(first) = first_line(&mut client_reader).await else {
        return;
    };
    let up = async {
        daemon_writer.write_all(&with_token(&first, token)).await?;
        tokio::io::copy_buf(&mut client_reader, &mut daemon_writer).await?;
        daemon_writer.shutdown().await
    };
    let down = async {
        tokio::io::copy(&mut daemon_reader, &mut client_writer).await?;
        client_writer.shutdown().await
    };
    // The Daemon closing ends the relay; the Client closing ends its side,
    // after which the Daemon closes once it has stopped the Client's Runs.
    let (_, closed) = tokio::join!(up, down);
    if let Err(error) = closed {
        tracing::debug!(%error, "the relay to the Daemon ended");
    }
}

/// The Client's first message, its line ending kept; `None` when it closed
/// first.
async fn first_line(reader: &mut (impl AsyncBufRead + Unpin)) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    let limit = u64::try_from(MAX_MESSAGE_BYTES).unwrap_or(u64::MAX);
    let read = reader.take(limit).read_until(b'\n', &mut line).await.ok()?;
    (read > 0).then_some(line)
}

/// `line` with `token` in its `params` when it is a `hello`, its params
/// read as `HelloParams` (its `_meta` kept beside them); as it came
/// otherwise, for the Daemon to refuse.
fn with_token(line: &[u8], token: &str) -> Vec<u8> {
    let Ok(mut message) = serde_json::from_slice::<Value>(line) else {
        return line.to_vec();
    };
    if message.get("method").and_then(Value::as_str) != Some(HELLO) {
        return line.to_vec();
    }
    let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) else {
        return line.to_vec();
    };
    let meta = params.remove(META);
    let Ok(mut hello) = serde_json::from_value::<HelloParams>(Value::Object(params.clone())) else {
        return line.to_vec();
    };
    hello.token = Some(token.to_owned());
    let Value::Object(mut typed) =
        serde_json::to_value(&hello).expect("invariant: `hello`'s params serialize")
    else {
        return line.to_vec();
    };
    if let Some(meta) = meta {
        typed.insert(META.to_owned(), meta);
    }
    *params = typed;
    let mut rewritten = serde_json::to_vec(&message).expect("invariant: a JSON value encodes");
    rewritten.push(b'\n');
    rewritten
}
