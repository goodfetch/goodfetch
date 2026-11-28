//! Linux `proxy_tcp` relay inside the Plugin's network namespace (ADR 0033,
//! issue 833): brings up loopback (`lo`), binds `127.0.0.1:0` inside the
//! isolated network namespace, forks a lightweight relay process outside the
//! Plugin's seccomp filter, and bridges each accepted TCP connection to a
//! fresh engine-side stream received over the inherited `AF_UNIX` control
//! socket via `SCM_RIGHTS`.
#![expect(
    unsafe_code,
    reason = "bringing up loopback, passing file descriptors via SCM_RIGHTS, and forking the relay process are Linux system calls"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the Linux plugin-shim proxy_tcp relay runs synchronously outside Tokio inside the Plugin's network namespace (ADR 0033)"
)]

use std::io::{self, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

/// Standard inherited file descriptor number for the engine-to-shim relay
/// control socket on Linux.
pub const CONTROL_FD: libc::c_int = 3;

/// One-byte request sent by the relay over `CONTROL_FD` to ask the engine for
/// a fresh connected proxy channel file descriptor via `SCM_RIGHTS`.
pub const CONNECT_REQUEST: u8 = b'C';

/// Brings the `lo` loopback interface up inside a newly created network
/// namespace before capabilities are dropped.
///
/// # Errors
///
/// When the `AF_INET` ioctl socket or `SIOCSIFFLAGS` fails.
pub(crate) fn bring_up_loopback() -> io::Result<()> {
    // SAFETY: creates a plain datagram socket with no memory pointers.
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if sock == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `sock` is a freshly opened valid descriptor owned here.
    let sock = unsafe { OwnedFd::from_raw_fd(sock) };

    // SAFETY: zero-initializing `libc::ifreq` is valid for all its C fields.
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    ifr.ifr_name[0] = b'l'.cast_signed();
    ifr.ifr_name[1] = b'o'.cast_signed();
    ifr.ifr_ifru.ifru_flags = i16::try_from(libc::IFF_UP | libc::IFF_LOOPBACK | libc::IFF_RUNNING)
        .expect("invariant: IFF_UP|IFF_LOOPBACK|IFF_RUNNING fits i16");

    // SAFETY: `ifr` is initialized and valid for `SIOCSIFFLAGS` on `sock`.
    let rc = unsafe { libc::ioctl(sock.as_raw_fd(), libc::SIOCSIFFLAGS, &raw mut ifr) };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Sends `fd` over the connected `AF_UNIX` stream `control` using `SCM_RIGHTS`.
///
/// # Errors
///
/// When `sendmsg` fails or writes zero bytes.
pub fn send_fd(control: RawFd, fd: RawFd) -> io::Result<()> {
    let mut byte = [1_u8];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut cmsg_buf = [0_u8; 64];
    // SAFETY: `CMSG_SPACE` is a pure size calculation for one `RawFd`.
    let cmsg_space = unsafe { libc::CMSG_SPACE(u32::try_from(size_of::<RawFd>()).unwrap_or(4)) };
    assert!(
        (cmsg_space as usize) <= cmsg_buf.len(),
        "invariant: cmsg_buf holds one RawFd"
    );
    // SAFETY: zero-initialized `msghdr` with initialized `iov` and `cmsg_buf`.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr().cast();
    msg.msg_controllen = cmsg_space as _;

    // SAFETY: `msg` points into `cmsg_buf` which has at least `cmsg_space` bytes.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
        assert!(!cmsg.is_null(), "invariant: CMSG_FIRSTHDR fits in cmsg_buf");
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(u32::try_from(size_of::<RawFd>()).unwrap_or(4)) as _;
        std::ptr::copy_nonoverlapping(
            (&raw const fd).cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            size_of::<RawFd>(),
        );
    }

    loop {
        // SAFETY: `msg` and its pointed buffers outlive `sendmsg`.
        let sent = unsafe { libc::sendmsg(control, &raw const msg, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if sent == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "control socket closed while sending fd",
            ));
        }
        return Ok(());
    }
}

/// Receives one file descriptor over the connected `AF_UNIX` stream `control`
/// using `SCM_RIGHTS`.
///
/// # Errors
///
/// When `recvmsg` fails, reaches EOF, or carries no `SCM_RIGHTS` descriptor.
pub fn recv_fd(control: RawFd) -> io::Result<OwnedFd> {
    let mut byte = [0_u8];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut cmsg_buf = [0_u8; 64];
    // SAFETY: zero-initialized `msghdr` pointing to stack buffers `iov` and `cmsg_buf`.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr().cast();
    msg.msg_controllen = cmsg_buf.len() as _;

    let received = loop {
        // SAFETY: `msg` and its pointed buffers outlive `recvmsg`.
        let n = unsafe { libc::recvmsg(control, &raw mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        break n;
    };
    if received == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "control socket closed before receiving fd",
        ));
    }

    // SAFETY: iterates over kernel-written control messages inside `cmsg_buf`.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let mut fd: RawFd = -1;
                std::ptr::copy_nonoverlapping(
                    libc::CMSG_DATA(cmsg),
                    (&raw mut fd).cast::<u8>(),
                    size_of::<RawFd>(),
                );
                if fd >= 0 {
                    return Ok(OwnedFd::from_raw_fd(fd));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&raw const msg, cmsg);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "engine control message carried no file descriptor",
    ))
}

/// Binds `127.0.0.1:0` inside the current (isolated) network namespace and
/// returns the listener together with its chosen loopback port.
///
/// # Errors
///
/// When binding loopback fails.
pub(crate) fn bind_loopback() -> io::Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

/// Forks the relay child inside the isolated network namespace before the
/// Plugin process applies Landlock and seccomp. The parent (soon-to-be Plugin)
/// closes `listener` and `control_fd` and returns immediately; the child
/// serves TCP connections until its parent exits.
///
/// # Errors
///
/// When `fork` fails.
pub(crate) fn spawn_in_netns(control_fd: RawFd, listener: TcpListener) -> io::Result<()> {
    // SAFETY: `plugin-shim` has one thread at this point, so `fork` inherits
    // no locks held by other threads.
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => run_relay_child(control_fd, &listener),
        _child => {
            drop(listener);
            // SAFETY: parent no longer needs the engine control socket; closing
            // it ensures the confined Plugin never holds it even before `exec`.
            unsafe {
                libc::close(control_fd);
            }
            Ok(())
        }
    }
}

/// Entry point of the forked relay child process: detaches from the Plugin's
/// stdio pipes, ties its lifetime to its parent via `PR_SET_PDEATHSIG`, and
/// bridges every accepted TCP connection to the engine via `SCM_RIGHTS`.
#[expect(
    clippy::disallowed_methods,
    reason = "the plugin-shim Linux proxy_tcp relay spawns OS threads in its dedicated non-Tokio process (ADR 0033)"
)]
fn run_relay_child(control_fd: RawFd, listener: &TcpListener) -> ! {
    // SAFETY: `prctl` takes scalar flags only.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } == -1 {
        // SAFETY: immediate child exit with no destructors.
        unsafe { libc::_exit(1) };
    }
    // Close inherited stdin/stdout so the relay never keeps the Plugin's
    // JSON-RPC pipes open or touches them.
    // SAFETY: closing standard descriptors in the relay child.
    unsafe {
        libc::close(libc::STDIN_FILENO);
        libc::close(libc::STDOUT_FILENO);
    }
    // SAFETY: `control_fd` is owned exclusively by the relay child after fork.
    let control = Arc::new(Mutex::new(unsafe { UnixStream::from_raw_fd(control_fd) }));
    for accepted in listener.incoming() {
        let Ok(client) = accepted else {
            break;
        };
        let control = Arc::clone(&control);
        let _ = std::thread::spawn(move || {
            let upstream = {
                let Ok(mut guard) = control.lock() else {
                    return;
                };
                if guard.write_all(&[CONNECT_REQUEST]).is_err() {
                    return;
                }
                let Ok(fd) = recv_fd(guard.as_raw_fd()) else {
                    return;
                };
                // SAFETY: `fd` is a connected AF_UNIX stream received via SCM_RIGHTS.
                unsafe { UnixStream::from_raw_fd(fd.into_raw_fd()) }
            };
            bridge(client, upstream);
        });
    }
    // SAFETY: relay child terminates immediately when the listener closes.
    unsafe { libc::_exit(0) }
}

/// Copies bytes bidirectionally between the Plugin's TCP socket and the
/// engine's Unix stream until both directions close.
#[expect(
    clippy::disallowed_methods,
    reason = "synchronous relay child process outside Tokio (ADR 0033)"
)]
fn bridge(client: TcpStream, upstream: UnixStream) {
    let Ok(mut client_read) = client.try_clone() else {
        return;
    };
    let Ok(mut upstream_write) = upstream.try_clone() else {
        return;
    };
    let mut upstream_read = upstream;
    let mut client_write = client;

    let handle = std::thread::spawn(move || {
        let mut buf = [0_u8; 16 * 1024];
        loop {
            match client_read.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if upstream_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = upstream_write.shutdown(std::net::Shutdown::Write);
    });

    let mut buf = [0_u8; 16 * 1024];
    loop {
        match upstream_read.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if client_write.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = client_write.shutdown(std::net::Shutdown::Write);
    let _ = handle.join();
}
