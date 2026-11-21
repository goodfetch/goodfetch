use std::net::{Ipv4Addr, SocketAddr};

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::header::{HOST, ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::{ExposedDaemon, Link, open};

fn exposed() -> ExposedDaemon {
    ExposedDaemon {
        address: SocketAddr::from((Ipv4Addr::LOCALHOST, 7700)),
        authority: "127.0.0.1:7700".to_owned(),
    }
}

/// The client's end of an in-memory pair, as a Link, and the Daemon's.
fn pair() -> (Link, DuplexStream) {
    let (client, daemon) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(client);
    (Link::new(reader, writer), daemon)
}

/// The Daemon's side of the handshake on `end`, answering the offered
/// subprotocol, with the `Host`, `Origin` and subprotocol it was offered.
async fn accept(end: DuplexStream) -> (WebSocketStream<DuplexStream>, [String; 3]) {
    let mut heard = [String::new(), String::new(), String::new()];
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's handshake callback answers its own `ErrorResponse`"
    )]
    let answer = |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
        let header = |name| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        heard = [header(HOST), header(ORIGIN), header(SEC_WEBSOCKET_PROTOCOL)];
        if let Some(offered) = request.headers().get(SEC_WEBSOCKET_PROTOCOL) {
            response
                .headers_mut()
                .insert(SEC_WEBSOCKET_PROTOCOL, offered.clone());
        }
        Ok(response)
    };
    let socket = tokio_tungstenite::accept_hdr_async(end, answer)
        .await
        .expect("the Daemon's side of the handshake");
    (socket, heard)
}

#[test]
fn each_whole_line_written_goes_as_one_text_message_then_a_normal_close() {
    goodfetch_test_support::real_time_runtime().block_on(async {
        let (link, daemon_end) = pair();
        let client = async {
            let (_, mut writer, _) = open(link, &exposed()).await.expect("the handshake").split();
            writer
                .write_all(b"{\"id\":1}\n{\"i")
                .await
                .expect("a write");
            writer.write_all(b"d\":2}\n").await.expect("a write");
            writer.shutdown().await.expect("the close");
        };
        let daemon = async {
            let (mut socket, heard) = accept(daemon_end).await;
            let mut received = Vec::new();
            // Up to the client's close, whose answer the dropped Link
            // does not wait for.
            while let Some(message) = socket.next().await {
                let message = message.expect("a message");
                let closed = message.is_close();
                received.push(message);
                if closed {
                    break;
                }
            }
            (heard, received)
        };
        let ((), (heard, received)) = tokio::join!(client, daemon);
        assert_eq!(
            heard,
            [
                "127.0.0.1:7700".to_owned(),
                "http://127.0.0.1:7700".to_owned(),
                PROTOCOL_VERSION.websocket_subprotocol(),
            ]
        );
        let [first, second, Message::Close(Some(close))] = received.as_slice() else {
            panic!("two text messages and a close, not {received:?}");
        };
        assert_eq!(
            (first, second, close.code),
            (
                &Message::text("{\"id\":1}"),
                &Message::text("{\"id\":2}"),
                CloseCode::Normal
            )
        );
    });
}

#[test]
fn each_message_received_is_one_line_until_the_daemons_close_ends_them() {
    goodfetch_test_support::real_time_runtime().block_on(async {
        let (link, daemon_end) = pair();
        let client = async {
            let (mut reader, _writer, _) =
                open(link, &exposed()).await.expect("the handshake").split();
            let mut read = String::new();
            reader.read_to_string(&mut read).await.expect("the lines");
            read
        };
        let daemon = async {
            let (mut socket, _) = accept(daemon_end).await;
            for message in [
                Message::text("{\"id\":1}"),
                Message::Ping(Vec::new().into()),
                Message::binary(b"{\"id\":2}".to_vec()),
            ] {
                socket.send(message).await.expect("a message sent");
            }
            socket.close(None).await.expect("the Daemon's close");
            // The client's answer to the close.
            while socket.next().await.is_some() {}
        };
        let (read, ()) = tokio::join!(client, daemon);
        assert_eq!(read, "{\"id\":1}\n{\"id\":2}\n");
    });
}
