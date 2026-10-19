//! What the server pushes on a connection a page holds open (#1536): a
//! server-sent events stream (`text/event-stream`) and a WebSocket
//! (RFC 6455), each sending one message every [`INTERVAL`] until the page
//! closes it or the server stops. Neither ever ends on its own, so a page
//! that holds one is never idle by a count of open connections, only by
//! one that leaves them out.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use hyper::body::{Bytes, Frame};
use sha1::{Digest as _, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::time::{Interval, MissedTickBehavior};

/// How often each held-open connection sends a message.
pub(crate) const INTERVAL: Duration = Duration::from_millis(250);

/// A connection a page holds open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Push {
    /// A server-sent events stream.
    Events,
    /// A WebSocket.
    Socket,
}

/// A server-sent events body: one `data:` event every [`INTERVAL`], for as
/// long as the client reads it.
pub(crate) struct EventStream {
    interval: Interval,
    sent: u64,
}

impl EventStream {
    pub(crate) fn new() -> Self {
        let mut interval = tokio::time::interval(INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self { interval, sent: 0 }
    }
}

impl hyper::body::Body for EventStream {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let stream = self.get_mut();
        ready!(stream.interval.poll_tick(context));
        stream.sent += 1;
        let event = format!("data: {{\"event\":{}}}\n\n", stream.sent);
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(event)))))
    }
}

/// The GUID RFC 6455 appends to a client's key (section 1.3).
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The `Sec-WebSocket-Accept` value that answers a client's
/// `Sec-WebSocket-Key`.
pub(crate) fn accept(key: &str) -> String {
    let mut hash = Sha1::new();
    hash.update(key.trim().as_bytes());
    hash.update(WEBSOCKET_GUID.as_bytes());
    data_encoding::BASE64.encode(&hash.finalize())
}

const TEXT: u8 = 0x1;
const CLOSE: u8 = 0x8;
const PING: u8 = 0x9;
const PONG: u8 = 0xA;

/// The most bytes of client frames held before they are read whole; a
/// client that sends more is closed on (STYLE.md: every buffer is bounded).
const CLIENT_LIMIT: usize = 64 * 1024;

/// Serves an upgraded WebSocket connection: a text message every
/// [`INTERVAL`], a pong for each ping, until the client closes it, sends a
/// frame the server cannot read, or goes away.
///
/// # Cancel safety
///
/// Dropping the future drops the connection, which the client sees closed.
pub(crate) async fn socket<S: AsyncRead + AsyncWrite + Unpin>(io: S) {
    let (mut reader, mut writer) = tokio::io::split(io);
    let mut interval = tokio::time::interval(INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut held = Vec::new();
    let mut chunk = [0; 4096];
    let mut sent: u64 = 0;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                sent += 1;
                let message = format!("{{\"message\":{sent}}}");
                if writer.write_all(&frame(TEXT, message.as_bytes())).await.is_err() {
                    return;
                }
            }
            read = reader.read(&mut chunk) => {
                let Ok(count @ 1..) = read else {
                    return;
                };
                held.extend_from_slice(&chunk[..count]);
                if held.len() > CLIENT_LIMIT {
                    return;
                }
                loop {
                    match take_frame(&mut held) {
                        Taken::Frame(CLOSE, _) => {
                            let _ = writer.write_all(&frame(CLOSE, &[])).await;
                            return;
                        }
                        Taken::Frame(PING, payload) => {
                            if writer.write_all(&frame(PONG, &payload)).await.is_err() {
                                return;
                            }
                        }
                        Taken::Frame(..) => {}
                        Taken::Partial => break,
                        Taken::Unreadable => return,
                    }
                }
            }
        }
    }
}

/// A server frame: final, unmasked, with `payload` (RFC 6455, section 5.2).
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        length @ 0..=125 => {
            frame.push(u8::try_from(length).expect("invariant: a length below 126 fits a byte"));
        }
        length @ 126..=0xFFFF => {
            frame.push(126);
            frame.extend_from_slice(
                &u16::try_from(length)
                    .expect("invariant: the length fits 16 bits")
                    .to_be_bytes(),
            );
        }
        length => {
            frame.push(127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

/// What the client's bytes held so far begin with.
#[derive(Debug, PartialEq, Eq)]
enum Taken {
    /// A whole frame, taken off the front: its opcode and unmasked payload.
    Frame(u8, Vec<u8>),
    /// Not yet a whole frame.
    Partial,
    /// A frame no client may send: unmasked, or longer than the limit.
    Unreadable,
}

/// Takes the first whole client frame off `held`.
fn take_frame(held: &mut Vec<u8>) -> Taken {
    let [first, second, ..] = held[..] else {
        return Taken::Partial;
    };
    if second & 0x80 == 0 {
        return Taken::Unreadable;
    }
    let (length, mut at) = match second & 0x7F {
        126 => match held.get(2..4) {
            Some(bytes) => (u64::from(u16::from_be_bytes([bytes[0], bytes[1]])), 4),
            None => return Taken::Partial,
        },
        127 => match held
            .get(2..10)
            .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        {
            Some(bytes) => (u64::from_be_bytes(bytes), 10),
            None => return Taken::Partial,
        },
        length => (u64::from(length), 2),
    };
    let Ok(length) = usize::try_from(length) else {
        return Taken::Unreadable;
    };
    if length > CLIENT_LIMIT {
        return Taken::Unreadable;
    }
    let Some(mask) = held
        .get(at..at + 4)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
    else {
        return Taken::Partial;
    };
    at += 4;
    let Some(masked) = held.get(at..at + length) else {
        return Taken::Partial;
    };
    let payload = masked
        .iter()
        .zip(mask.iter().cycle())
        .map(|(byte, key)| byte ^ key)
        .collect();
    held.drain(..at + length);
    Taken::Frame(first & 0x0F, payload)
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "each test runs on the paused runtime `tokio::test` builds, TESTING.md's test Clock (ADR 0060)"
)]
mod tests {
    use std::time::Duration;

    use http_body_util::BodyExt as _;
    use tokio::io::{DuplexStream, duplex};

    use super::*;

    /// Long enough for any exchange of these tests, on the paused clock.
    const DEADLINE: Duration = Duration::from_secs(5);

    /// A masked client frame (RFC 6455, section 5.3), with a fixed mask.
    fn client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0x37, 0xfa, 0x21, 0x3d];
        let mut frame = vec![0x80 | opcode, 0x80 | u8::try_from(payload.len()).unwrap()];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .zip(mask.iter().cycle())
                .map(|(byte, key)| byte ^ key),
        );
        frame
    }

    /// The next server frame on `client`: its first byte and payload.
    async fn server_frame(client: &mut DuplexStream) -> (u8, Vec<u8>) {
        let mut head = [0; 2];
        client.read_exact(&mut head).await.unwrap();
        assert_eq!(head[1] & 0x80, 0, "a server frame is unmasked");
        let mut payload = vec![0; usize::from(head[1] & 0x7F)];
        client.read_exact(&mut payload).await.unwrap();
        (head[0], payload)
    }

    #[test]
    fn the_accept_value_is_rfc_6455s() {
        // RFC 6455, section 1.3.
        assert_eq!(
            accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_socket_sends_a_numbered_message_at_each_interval() {
        let (mut client, server) = duplex(4096);

        let (first, second) = tokio::select! {
            () = socket(server) => panic!("the socket ended on its own"),
            frames = async { (server_frame(&mut client).await, server_frame(&mut client).await) } => frames,
        };

        assert_eq!(first, (0x81, b"{\"message\":1}".to_vec()));
        assert_eq!(second, (0x81, b"{\"message\":2}".to_vec()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_socket_answers_a_ping_and_ends_when_the_client_closes_it() {
        let (mut client, server) = duplex(4096);
        let talk = async {
            let _message = server_frame(&mut client).await;
            client.write_all(&client_frame(PING, b"hi")).await.unwrap();
            let pong = server_frame(&mut client).await;
            client.write_all(&client_frame(CLOSE, &[])).await.unwrap();
            (pong, server_frame(&mut client).await)
        };

        let ((), (pong, close)) =
            tokio::time::timeout(DEADLINE, async { tokio::join!(socket(server), talk) })
                .await
                .expect("the socket ends once it answered the close");

        assert_eq!(pong, (0x80 | PONG, b"hi".to_vec()));
        assert_eq!(close, (0x80 | CLOSE, Vec::new()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_socket_ends_on_an_unmasked_client_frame() {
        let (mut client, server) = duplex(4096);
        let talk = async {
            let _message = server_frame(&mut client).await;
            client.write_all(&[0x81, 0x02, b'H', b'i']).await.unwrap();
            let mut rest = Vec::new();
            client.read_to_end(&mut rest).await.unwrap();
            rest
        };

        let ended =
            tokio::time::timeout(DEADLINE, async { tokio::join!(socket(server), talk) }).await;

        assert!(
            ended.is_ok(),
            "the socket ends on a frame no client may send"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_event_stream_sends_a_numbered_event_at_each_interval() {
        let mut stream = EventStream::new();

        let first = stream.frame().await.unwrap().unwrap().into_data().unwrap();
        let second = stream.frame().await.unwrap().unwrap().into_data().unwrap();

        assert_eq!(first, "data: {\"event\":1}\n\n");
        assert_eq!(second, "data: {\"event\":2}\n\n");
    }
}
