//! A forward proxy on loopback in front of another, on threads of its own, so
//! a test can make a request of the Session it proxies lose a race: it calls
//! the test's closure with each connection's first line before it hands the
//! connection on.
#![expect(
    clippy::disallowed_types,
    reason = "a loopback proxy on the test's own threads, never an async worker: the only sockets here (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "each connection has a plain thread of its own, for as long as the test's process runs, and joins no owner's tracker"
)]

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

/// Starts a forward proxy in front of the one at `upstream`, answering its
/// address: each connection goes on to `upstream` as it came, once `hold`
/// returned for the first line of the request it opens with, such as
/// `GET http://site.test:1234/b HTTP/1.1`. A connection whose request never
/// ends its head goes nowhere.
///
/// # Panics
///
/// When it cannot listen on loopback. A connection `upstream` refuses closes
/// the browser's.
pub fn proxy_in_front(upstream: String, hold: impl Fn(&str) + Send + Sync + 'static) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a test can listen on loopback");
    let address = listener
        .local_addr()
        .expect("a listening socket has an address");
    let hold = Arc::new(hold);
    std::thread::spawn(move || {
        for browser in listener.incoming().map_while(Result::ok) {
            let (upstream, hold) = (upstream.clone(), Arc::clone(&hold));
            std::thread::spawn(move || {
                let _ = hand_on(browser, &upstream, &*hold);
            });
        }
    });
    address
}

/// Reads the head of the request `browser` opens with, holds it as `hold`
/// says, then copies each way between it and `upstream` until they close.
fn hand_on(mut browser: TcpStream, upstream: &str, hold: &dyn Fn(&str)) -> io::Result<()> {
    let mut head = Vec::new();
    let mut chunk = [0; 4096];
    while !head.windows(4).any(|end| end == b"\r\n\r\n") {
        let read = browser.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..read]);
    }
    hold(String::from_utf8_lossy(&head).lines().next().unwrap_or(""));
    let mut proxy = TcpStream::connect(upstream)?;
    proxy.write_all(&head)?;
    let mut browser_side = browser.try_clone()?;
    let mut proxy_side = proxy.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut browser_side, &mut proxy_side);
        let _ = proxy_side.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut proxy, &mut browser);
    let _ = browser.shutdown(Shutdown::Write);
    Ok(())
}
