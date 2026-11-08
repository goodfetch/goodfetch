//! How `serve` stops: a caller may send `SIGTERM` as soon as it reads the
//! first output line, so `serve` listens for it before it writes the line's
//! first byte, and stops cleanly.
#![expect(
    clippy::disallowed_types,
    reason = "the test gives `serve` a socket pair as its stdout, to hold the line's end back"
)]

use std::io::Read as _;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use goodfetch_test_support::{LOOPBACK_PORT_0, UNSHAPED, serve_arguments};

/// `serve`'s stdout is a socket whose buffers hold less than the line (5.7 KB
/// on seed 1), so `serve` is still writing it, waiting for the test to read,
/// when `SIGTERM` comes.
#[test]
fn serve_stops_cleanly_on_a_sigterm_sent_while_it_writes_its_line() {
    let (mut line_end, stdout) = UnixStream::pair().expect("a socket pair opens");
    socket2::SockRef::from(&stdout)
        .set_send_buffer_size(1024)
        .expect("the send buffer shrinks");
    socket2::SockRef::from(&line_end)
        .set_recv_buffer_size(1024)
        .expect("the receive buffer shrinks");
    let mut child = Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
        .args(serve_arguments(1, LOOPBACK_PORT_0, UNSHAPED))
        .env_clear()
        .stdin(Stdio::null())
        .stdout(OwnedFd::from(stdout))
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the Proving ground starts");
    let mut line = vec![0];
    line_end
        .read_exact(&mut line)
        .expect("serve starts its line");

    let stopped = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("`kill` runs");

    line_end.read_to_end(&mut line).expect("stdout is readable");
    let exited = child.wait().expect("the Proving ground exits");
    assert!(stopped.success() && exited.success(), "{exited}");
    assert!(line.ends_with(b"}\n"), "{}", String::from_utf8_lossy(&line));
}
