#![expect(
    clippy::disallowed_methods,
    reason = "a test's stand-in engine answers its one Connection on a thread of its own, which ends with it"
)]

use std::net::{SocketAddr, TcpListener, TcpStream};

use goodfetch_protocol::catalog::Product;

use super::*;

/// An engine on a loopback port, which `answer` handles the one Connection
/// of.
fn engine(answer: impl FnOnce(TcpStream) + Send + 'static) -> DaemonEndpoint {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the loopback port binds");
    let address: SocketAddr = listener.local_addr().expect("the listener has an address");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the Client connects");
        answer(stream);
    });
    DaemonEndpoint::Tcp(address)
}

/// Waits until the Client's whole `hello` waits unread in `stream`.
fn hello_arrived(stream: &TcpStream) {
    let mut seen = [0; 1024];
    loop {
        let found = stream.peek(&mut seen).expect("the Connection reads");
        if seen[..found].contains(&b'\n') {
            return;
        }
    }
}

fn hello() -> Hello {
    Hello::needing(Product::Collie)
}

/// An engine that closes with the `hello` unread resets the Connection, which
/// is how a refusal reads as often as an orderly close.
#[test]
fn a_connection_the_engine_resets_reads_as_a_refusal() {
    let endpoint = engine(|stream| {
        hello_arrived(&stream);
        drop(stream);
    });

    assert!(DaemonClient::refuses(&endpoint, &hello()));
}

#[test]
fn a_connection_the_engine_closes_after_reading_reads_as_a_refusal() {
    let endpoint = engine(|mut stream| {
        hello_arrived(&stream);
        let mut read = [0; 1024];
        let _ = stream.read(&mut read).expect("the hello reads");
        drop(stream);
    });

    assert!(DaemonClient::refuses(&endpoint, &hello()));
}

/// Bytes that are no text are an answer gone wrong, not a closed Connection,
/// so a test that asserts a refusal must not pass on them.
#[test]
#[should_panic(expected = "the engine's answer cannot be read")]
fn bytes_that_are_no_text_read_as_no_refusal() {
    let endpoint = engine(|mut stream| {
        hello_arrived(&stream);
        stream.write_all(b"\xff\xfe\n").expect("the Client reads");
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
    });

    let _ = DaemonClient::refuses(&endpoint, &hello());
}
