//! The namespace's own side, which `goodfetch-test-env dns-namespace` runs:
//! it opens its socket to the DNS fixture while still in the host's network
//! namespace, unshares the network and mount namespaces (and a user
//! namespace, unless it runs as root for a user), mounts the given files
//! over the host's `/etc`, brings the loopback interface up and listens on
//! `127.0.0.1:53`. Then it starts the child and relays each query to the
//! fixture and each answer back, until the child exits. A query the fixture
//! cannot be reached for is answered `SERVFAIL` at once.
#![expect(
    unsafe_code,
    reason = "unsharing namespaces, mounting and bringing an interface up are raw system calls, which Rust's standard library does not offer (ADR 0067)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the namespace's relay opens its own UDP sockets, outside the engine's transport (ADR 0067)"
)]

use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::str::FromStr;

/// The most queries waiting for an answer (STYLE.md: every buffer is
/// bounded); past it, the oldest waiting query is forgotten.
const PENDING_LIMIT: usize = 1024;

/// The largest DNS message relayed over UDP.
const MESSAGE_LIMIT: usize = 65_535;

/// The namespace's second IPv4 address, on `lo:1`, which is not
/// `127.0.0.1` and so counts as an IPv4 address of the host.
const SECOND_ADDRESS: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

/// How long the relay waits for a datagram before it looks at the child.
const POLL_MILLISECONDS: libc::c_int = 20;

/// The user and group a root namespace starts its child as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsideUser {
    /// The user ID.
    pub uid: u32,
    /// The group ID.
    pub gid: u32,
}

impl fmt::Display for InsideUser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.uid, self.gid)
    }
}

impl FromStr for InsideUser {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("{text:?} is not <uid>:<gid>");
        let (uid, gid) = text.split_once(':').ok_or_else(invalid)?;
        Ok(Self {
            uid: uid.parse().map_err(|_| invalid())?,
            gid: gid.parse().map_err(|_| invalid())?,
        })
    }
}

/// What the namespace's side is asked to do.
#[derive(Clone, Debug)]
pub struct Inside {
    /// The DNS fixture's address, reached from the host's network
    /// namespace.
    pub dns: SocketAddr,
    /// A folder whose files are each mounted over `/etc/<their name>`.
    pub etc: PathBuf,
    /// The user to start the child as, when this runs as root (through
    /// sudo); without one, it unshares a user namespace too.
    pub user: Option<InsideUser>,
    /// The child's whole environment, as `NAME=VALUE`.
    pub env: Vec<String>,
    /// The child's program and arguments; with none, it makes the
    /// namespace and exits.
    pub child: Vec<OsString>,
}

impl Inside {
    /// Makes the namespace and runs the child in it, returning the exit
    /// status to exit with: the child's code, or 128 plus its signal.
    ///
    /// # Errors
    ///
    /// A step that failed, named in the error.
    pub fn run(&self) -> io::Result<i32> {
        let fixture = UdpSocket::bind(match self.dns {
            SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        })
        .and_then(|socket| socket.connect(self.dns).map(|()| socket))
        .map_err(step("the socket to the DNS fixture"))?;
        self.unshare()?;
        mount_private()?;
        mount_files(&self.etc)?;
        loopback_up()?;
        let nameserver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 53))
            .map_err(step("the nameserver on 127.0.0.1:53"))?;
        let Some((program, args)) = self.child.split_first() else {
            return Ok(0);
        };
        let mut command = Command::new(program);
        command.args(args).env_clear();
        for pair in &self.env {
            if let Some((name, value)) = pair.split_once('=') {
                command.env(name, value);
            }
        }
        if let Some(user) = self.user {
            command.uid(user.uid).gid(user.gid);
        }
        // SAFETY: `prctl` with integer arguments only, in the forked child
        // before its program starts.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().map_err(step("the child"))?;
        relay(&fixture, &nameserver, child)
    }

    fn unshare(&self) -> io::Result<()> {
        let mut flags = libc::CLONE_NEWNET | libc::CLONE_NEWNS;
        // SAFETY: `getuid` and `getgid` take no arguments and cannot fail.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        if self.user.is_none() {
            flags |= libc::CLONE_NEWUSER;
        }
        // SAFETY: `unshare` with integer flags only.
        if unsafe { libc::unshare(flags) } != 0 {
            return Err(io::Error::last_os_error()).map_err(step("unshare"));
        }
        if self.user.is_none() {
            // The user keeps its own IDs inside, so the child is the test's
            // user; only this process holds the namespace's capabilities.
            std::fs::write("/proc/self/setgroups", "deny")
                .and_then(|()| std::fs::write("/proc/self/uid_map", format!("{uid} {uid} 1")))
                .and_then(|()| std::fs::write("/proc/self/gid_map", format!("{gid} {gid} 1")))
                .map_err(step("the user namespace's ID maps"))?;
        }
        Ok(())
    }
}

/// An error's context: the step it came from.
fn step(name: &'static str) -> impl Fn(io::Error) -> io::Error {
    move |error| io::Error::new(error.kind(), format!("{name}: {error}"))
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

/// Keeps every mount made here inside the new mount namespace.
fn mount_private() -> io::Result<()> {
    // SAFETY: the paths are NUL-terminated literals, and the other pointers
    // null, as `mount` allows for a propagation change.
    let changed = unsafe {
        libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        )
    };
    if changed != 0 {
        return Err(io::Error::last_os_error()).map_err(step("making / private"));
    }
    Ok(())
}

/// Mounts each file of `etc` over `/etc/<its name>`.
fn mount_files(etc: &Path) -> io::Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(etc)
        .and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect()
        })
        .map_err(step("the folder of files to mount"))?;
    files.sort();
    for file in files {
        let Some(name) = file.file_name() else {
            continue;
        };
        let target = Path::new("/etc").join(name);
        let (source, target_text) = (c_path(&file)?, c_path(&target)?);
        // SAFETY: both paths are NUL-terminated strings on this frame, and
        // a bind mount reads no file system type or data.
        let mounted = unsafe {
            libc::mount(
                source.as_ptr(),
                target_text.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            )
        };
        if mounted != 0 {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(
                error.kind(),
                format!("mounting over {}: {error}", target.display()),
            ));
        }
    }
    Ok(())
}

/// Brings the new network namespace's `lo` up, which starts down, with a
/// second IPv4 address, [`SECOND_ADDRESS`], as `lo:1`: a resolver asked for
/// only the families the host has addresses for (`AI_ADDRCONFIG`, as
/// `getent ahosts` and browsers ask) counts no IPv4 family from
/// `127.0.0.1` alone, and would answer without asking the nameserver.
fn loopback_up() -> io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(step("bringing lo up"))?;
    let mut alias = interface_request(b"lo:1");
    let address = libc::sockaddr_in {
        sin_family: libc::sa_family_t::try_from(libc::AF_INET)
            .expect("invariant: AF_INET fits a family"),
        sin_port: 0,
        sin_addr: libc::in_addr {
            s_addr: u32::from(SECOND_ADDRESS).to_be(),
        },
        sin_zero: [0; 8],
    };
    let mut request = interface_request(b"lo");
    // SAFETY: each request is an `ifreq` naming an interface. A
    // `sockaddr_in` fits the `sockaddr` of `ifru_addr`, which `SIOCSIFADDR`
    // reads as one since its family says so; `SIOCGIFFLAGS` fills the flags
    // `SIOCSIFFLAGS` then reads.
    unsafe {
        std::ptr::write((&raw mut alias.ifr_ifru.ifru_addr).cast(), address);
        if libc::ioctl(
            socket.as_raw_fd(),
            request_code(libc::SIOCSIFADDR),
            &raw const alias,
        ) != 0
        {
            return Err(io::Error::last_os_error()).map_err(step("adding lo's second address"));
        }
        if libc::ioctl(
            socket.as_raw_fd(),
            request_code(libc::SIOCGIFFLAGS),
            &raw mut request,
        ) != 0
        {
            return Err(io::Error::last_os_error()).map_err(step("reading lo's flags"));
        }
        request.ifr_ifru.ifru_flags |=
            libc::c_short::try_from(libc::IFF_UP).expect("invariant: IFF_UP fits a short");
        if libc::ioctl(
            socket.as_raw_fd(),
            request_code(libc::SIOCSIFFLAGS),
            &raw const request,
        ) != 0
        {
            return Err(io::Error::last_os_error()).map_err(step("bringing lo up"));
        }
    }
    Ok(())
}

/// An `ioctl` request number as the target's `ioctl` takes it: glibc's
/// `unsigned long`, musl's `int`.
fn request_code(code: libc::c_ulong) -> libc::Ioctl {
    libc::Ioctl::try_from(code).expect("invariant: an interface request number fits an int")
}

/// An `ifreq` naming the interface `name`, the rest zero.
fn interface_request(name: &[u8]) -> libc::ifreq {
    // SAFETY: `ifreq` is plain data, for which all zeroes is valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(name) {
        *slot = libc::c_char::from_ne_bytes([*byte]);
    }
    request
}

/// Relays queries from the namespace's nameserver to the fixture, and
/// answers back to whoever asked, until the child exits.
fn relay(fixture: &UdpSocket, nameserver: &UdpSocket, mut child: Child) -> io::Result<i32> {
    for socket in [fixture, nameserver] {
        socket.set_nonblocking(true).map_err(step("the relay"))?;
    }
    // Each waiting query by its ID: who asked, and the query itself.
    let mut pending: BTreeMap<u16, (SocketAddr, Vec<u8>)> = BTreeMap::new();
    let mut order: Vec<u16> = Vec::new();
    let mut buffer = vec![0_u8; MESSAGE_LIMIT];
    loop {
        if let Some(status) = child.try_wait().map_err(step("waiting for the child"))? {
            return Ok(status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)));
        }
        let mut sockets = [fixture, nameserver].map(|socket| libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        // SAFETY: `sockets` is an array of `pollfd`s on this frame, of the
        // length passed.
        let ready = unsafe { libc::poll(sockets.as_mut_ptr(), 2, POLL_MILLISECONDS) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).map_err(step("the relay"));
        }
        while let Ok((length, client)) = nameserver.recv_from(&mut buffer) {
            let query = &buffer[..length];
            let Some(id) = message_id(query) else {
                continue;
            };
            if pending.len() >= PENDING_LIMIT && !order.is_empty() {
                pending.remove(&order.remove(0));
            }
            order.retain(|waiting| *waiting != id);
            order.push(id);
            pending.insert(id, (client, query.to_vec()));
            // A refused send shows again as a refused receive below.
            let _ = fixture.send(query);
        }
        loop {
            match fixture.recv(&mut buffer) {
                Ok(length) => {
                    let answer = &buffer[..length];
                    if let Some((client, _)) = message_id(answer).and_then(|id| {
                        order.retain(|waiting| *waiting != id);
                        pending.remove(&id)
                    }) {
                        let _ = nameserver.send_to(answer, client);
                    }
                }
                // Nothing listens at the fixture's address: every waiting
                // query fails now rather than at the resolver's timeout.
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    for (client, query) in std::mem::take(&mut pending).into_values() {
                        let _ = nameserver.send_to(&server_failure(&query), client);
                    }
                    order.clear();
                }
                Err(_) => break,
            }
        }
    }
}

/// A DNS message's ID, its first two bytes.
fn message_id(message: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes([*message.first()?, *message.get(1)?]))
}

/// The answer `SERVFAIL` to `query`: the query itself, its question kept so
/// the resolver matches it, marked as a response with recursion available
/// and response code 2.
fn server_failure(query: &[u8]) -> Vec<u8> {
    let mut answer = query.to_vec();
    if let Some(flags) = answer.get_mut(2) {
        *flags |= 0x80;
    }
    if let Some(flags) = answer.get_mut(3) {
        *flags = 0x80 | 0x02;
    }
    answer
}
