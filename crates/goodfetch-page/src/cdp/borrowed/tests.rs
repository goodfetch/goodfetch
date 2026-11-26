//! The Borrowed browser's bridge (finding 563-2): it carries both ways at
//! once, so a large command the engine writes and a large message the
//! browser sends cross without either waiting on the other, and it ends at
//! the Run's stop whatever it was writing.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_util::sync::CancellationToken;

use super::bridge;

/// Each side's message: past what either in-memory pipe buffers.
const MESSAGE: usize = 2 * 1024 * 1024;

/// What the pipe between the engine's end and the bridge buffers, as the
/// bridge's own pipe does, far less than [`MESSAGE`].
const BUFFER: usize = 64 * 1024;

/// What the browser's socket buffers: a loopback TCP connection takes
/// several MiB before a writer waits.
const SOCKET_BUFFER: usize = 32 * 1024 * 1024;

/// How long the crossing may take.
const DEADLINE: Duration = Duration::from_secs(10);

/// A CDP command of about [`MESSAGE`] bytes, as the driver frames it: its
/// JSON, then a NUL.
fn large_command() -> Vec<u8> {
    let mut command = br#"{"id":1,"method":"Runtime.evaluate","params":{"expression":""#.to_vec();
    command.extend(std::iter::repeat_n(b'a', MESSAGE));
    command.extend_from_slice(br#""}}"#);
    command.push(0);
    command
}

#[test]
fn a_large_command_and_a_large_incoming_message_cross_and_the_bridge_ends_at_the_stop() {
    goodfetch_test_support::runtime().block_on(async {
        let (bridge_socket, browser_socket) = tokio::io::duplex(SOCKET_BUFFER);
        let socket = WebSocketStream::from_raw_socket(bridge_socket, Role::Client, None).await;
        let (mut to_bridge, mut from_bridge) =
            WebSocketStream::from_raw_socket(browser_socket, Role::Server, None)
                .await
                .split();
        let (mut engine, bridge_end) = tokio::io::duplex(BUFFER);
        let stopping = CancellationToken::new();
        let command = large_command();
        let sent = command.len() - 1;

        let crossing = async {
            let engine_side = async {
                // The driver writes a command whole before it reads again.
                engine.write_all(&command).await.expect("the bridge reads");
                let mut incoming = vec![0; MESSAGE + 1];
                engine
                    .read_exact(&mut incoming)
                    .await
                    .expect("the bridge writes");
                incoming.len()
            };
            // The browser reads and writes apart, as Chromium does.
            let browser_side = async {
                let ((), received) = tokio::join!(
                    async {
                        to_bridge
                            .send(Message::text("b".repeat(MESSAGE)))
                            .await
                            .expect("the bridge reads");
                    },
                    from_bridge.next()
                );
                match received {
                    Some(Ok(Message::Text(text))) => text.len(),
                    other => panic!("expected the command, got {other:?}"),
                }
            };
            let crossed =
                tokio::time::timeout(DEADLINE, async { tokio::join!(engine_side, browser_side) })
                    .await;
            stopping.cancel();
            crossed
        };
        let ended = tokio::time::timeout(2 * DEADLINE, async {
            tokio::join!(bridge(socket, bridge_end, stopping.clone()), crossing)
        })
        .await;

        let stopped = ended.is_ok();
        let crossed = ended.ok().and_then(|((), crossed)| crossed.ok());
        assert_eq!(
            (crossed, stopped),
            (Some((MESSAGE + 1, sent)), true),
            "both messages crossed, and the bridge ended at the stop"
        );
    });
}
