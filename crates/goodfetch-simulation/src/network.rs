//! turmoil's network adapter of `transport`: turmoil's TCP streams copied
//! to and from the in-memory network's Connections, so the engine's code
//! reads and writes a Connection as it does in production.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};

use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport::{self, Connection, Network};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use turmoil::ToSocketAddrs;
use turmoil::net::{TcpListener, TcpStream};

/// The network a host's Daemon listens on: each connection turmoil
/// accepts on `port` reaches the listener the Daemon opens on it
/// ([`Network::listen_at`]), whatever address that listener names. Its
/// copies run as `owner`'s tasks.
///
/// # Errors
///
/// When turmoil refuses the port, such as one this host already binds.
///
/// # Cancel safety
///
/// Dropping the future binds nothing.
pub async fn listen(owner: &TaskOwner, port: u16) -> io::Result<Network> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let (network, dialer) = transport::in_memory();
    let copies = owner.child();
    owner.spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(error) => {
                    // The listener's failure is the engine's accept's, as
                    // the OS's is; nothing listens on after it.
                    let _gone = dialer.fail_accept(error).await;
                    return;
                }
            };
            let Ok(connection) = dialer.connect().await else {
                // The Daemon no longer accepts.
                return;
            };
            carry(&copies, stream, connection);
        }
    });
    Ok(Network::InMemory(network))
}

/// The Client's end of a Connection to `address`, which turmoil carries.
/// Its copies run as `owner`'s tasks.
///
/// # Errors
///
/// When nothing listens at `address`, or the network refuses the
/// connection, such as across a partition.
///
/// # Cancel safety
///
/// Dropping the future opens no Connection.
pub async fn connect(owner: &TaskOwner, address: impl ToSocketAddrs) -> io::Result<Connection> {
    let stream = TcpStream::connect(address).await?;
    let (network, dialer) = transport::in_memory();
    // The in-memory network's listener is the one way to its far end: the
    // address it names is never bound.
    let (mut listener, _) = Network::InMemory(network)
        .listen_at(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await?;
    let client = dialer.connect().await.map_err(io::Error::other)?;
    let carried = listener.accept().await.map_err(io::Error::other)?;
    carry(owner, stream, carried);
    Ok(client)
}

/// Copies `stream`'s bytes into `connection` and back, until each side
/// ends: the end of one direction shuts the other side's writer down, so a
/// crash or a reset reaches the engine as the end of its Connection.
fn carry(owner: &TaskOwner, stream: TcpStream, connection: Connection) {
    let (mut stream_reader, mut stream_writer) = stream.into_split();
    let (mut reader, mut writer) = connection.split();
    owner.spawn(async move {
        copy(&mut stream_reader, &mut writer).await;
    });
    owner.spawn(async move {
        copy(&mut reader, &mut stream_writer).await;
    });
}

async fn copy(reader: &mut (impl AsyncRead + Unpin), writer: &mut (impl AsyncWrite + Unpin)) {
    // Either side's failure ends the copy as its end does: what the other
    // side reads next is the end of its stream.
    let _copied = tokio::io::copy(reader, writer).await;
    let _closed = writer.shutdown().await;
}
