//! How the Proving ground ends a connection on purpose: a refused TLS
//! handshake, or an HTTP/2 connection after its `GOAWAY`. It closes its
//! sending side, then reads and drops what the client still sends, until the
//! client closes its side or [`LINGER`] passes. A socket closed with bytes
//! unread resets its connection, and on Windows the reset drops what the
//! client had received and not yet read: a refused handshake's alert
//! (issue 3535), or a `GOAWAY` and the answers before it (issues 2916 and
//! 2900).

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// How long an ended connection waits for its client to close before
/// closing it.
const LINGER: Duration = Duration::from_secs(1);

/// The most one read drops: the largest frame payload an HTTP/2 client
/// sends by default, and more than a TLS record.
const READ_SIZE: usize = 16_384;

/// Ends `stream` as the module's comment says.
pub(crate) async fn linger<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) {
    let _ = stream.shutdown().await;
    let drained = async {
        let mut sink = vec![0; READ_SIZE];
        while matches!(stream.read(&mut sink).await, Ok(read) if read > 0) {}
    };
    let _ = tokio::time::timeout(LINGER, drained).await;
}
