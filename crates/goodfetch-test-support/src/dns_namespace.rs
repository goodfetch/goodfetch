//! `DnsNamespace` (ADR 0067): a child started on Linux in a network and
//! mount namespace of its own, whose `/etc/resolv.conf` names a nameserver
//! on the namespace's loopback that relays every query to the Proving
//! ground's DNS fixture, so rebinding is tested end to end with no public
//! service, and the host's resolver is left as it was.
//!
//! The namespace is made by `goodfetch-test-env dns-namespace`, which opens
//! its socket to the fixture first, then unshares the namespaces, mounts
//! the test's `resolv.conf` (and `nsswitch.conf`, whose `hosts` line it
//! sets to `files dns`) over the host's, brings the loopback interface up,
//! listens on its port 53 and starts the child. It comes up one of two
//! ways, each a [`Mechanism`]: as the test's own user, in an unprivileged
//! user namespace, where the kernel allows one; otherwise through
//! `sudo -n`, as CI runners whose kernel refuses them (Ubuntu's
//! `AppArmor` restriction) allow, starting the child as the test's user again. A
//! runner may declare its mechanism in `GOODFETCH_TEST_DNS_NAMESPACE`, so a
//! silent fallback to the other fails. A namespace that cannot be made is
//! an error, never a skip.
#![expect(
    clippy::disallowed_methods,
    reason = "test support writes the namespace's files and starts its processes outside any async worker (ADR 0062)"
)]

mod inside;

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::env::TestEnv;
use crate::tools::{Tool, ToolsError, tools};
use crate::variables::{DNS_NAMESPACE_VARIABLE, Variables};

pub use inside::{Inside, InsideUser};

/// The subcommand of `goodfetch-test-env` that makes the namespace.
pub const DNS_NAMESPACE_COMMAND: &str = "dns-namespace";

/// The `resolv.conf` the child reads: the namespace's own nameserver, asked
/// once per query over TCP (`use-vc`), so each query takes exactly one turn
/// of an alternating name and cannot hang on a dropped loopback datagram.
const RESOLV_CONF: &str = "nameserver 127.0.0.1\noptions attempts:1 timeout:5 use-vc\n";

/// The host's file whose `hosts` line the namespace's copy replaces.
const HOST_NSSWITCH_CONF: &str = "/etc/nsswitch.conf";

/// How a namespace came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    /// As the test's own user, in an unprivileged user namespace.
    UserNamespace,
    /// Through `sudo -n`, the child started as the test's user again.
    Sudo,
}

impl Mechanism {
    /// Its name in `GOODFETCH_TEST_DNS_NAMESPACE`.
    pub fn name(self) -> &'static str {
        match self {
            Self::UserNamespace => "user_namespace",
            Self::Sudo => "sudo",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [Self::UserNamespace, Self::Sudo]
            .into_iter()
            .find(|mechanism| mechanism.name() == name)
    }
}

impl fmt::Display for Mechanism {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// Why one mechanism could not make the namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MechanismRefusal {
    /// The mechanism tried.
    pub mechanism: Mechanism,
    /// What refused it, as its process reported.
    pub reason: String,
}

impl fmt::Display for MechanismRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.mechanism, self.reason)
    }
}

/// Why no namespace was made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DnsNamespaceError {
    /// The test tool manifest names no `goodfetch-test-env`.
    #[error(transparent)]
    Tools(#[from] ToolsError),
    /// The namespace's files cannot be written into the test's folder.
    #[error("the DNS namespace's files cannot be written: {0}")]
    Files(io::Error),
    /// `GOODFETCH_TEST_DNS_NAMESPACE` names no mechanism.
    #[error("{DNS_NAMESPACE_VARIABLE} is {0:?}: it takes `user_namespace` or `sudo`")]
    Declared(String),
    /// Every mechanism tried was refused.
    #[error("no network and mount namespace could be made: {}", list(.0))]
    Unavailable(Vec<MechanismRefusal>),
}

fn list(refusals: &[MechanismRefusal]) -> String {
    refusals
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// A network and mount namespace whose resolver is the DNS fixture at its
/// address; each child it runs gets a namespace of its own.
#[derive(Clone, Debug)]
pub struct DnsNamespace {
    helper: PathBuf,
    dns: SocketAddr,
    etc: PathBuf,
    /// The test's user and group: the owner of its folder.
    user: InsideUser,
    mechanism: Mechanism,
}

impl DnsNamespace {
    /// Makes a namespace once to choose its mechanism: the declared one, or
    /// an unprivileged user namespace and then `sudo -n`. `dns_address` is
    /// the DNS fixture's, as the Proving ground's description gives it.
    ///
    /// # Errors
    ///
    /// [`DnsNamespaceError::Unavailable`] names each mechanism's refusal.
    pub fn enter(env: &TestEnv, dns_address: SocketAddr) -> Result<Self, DnsNamespaceError> {
        let helper = tools()?.path(Tool::TestEnv)?.to_owned();
        let candidates = match Variables::from_process().get(DNS_NAMESPACE_VARIABLE) {
            None => vec![Mechanism::UserNamespace, Mechanism::Sudo],
            Some(name) => vec![
                Mechanism::named(name)
                    .ok_or_else(|| DnsNamespaceError::Declared(name.to_owned()))?,
            ],
        };
        let etc = env.folder().join(env.unique_name("dns-namespace"));
        write_files(&etc).map_err(DnsNamespaceError::Files)?;
        let folder = std::fs::metadata(env.folder()).map_err(DnsNamespaceError::Files)?;
        let user = InsideUser {
            uid: folder.uid(),
            gid: folder.gid(),
        };
        let mut refusals = Vec::new();
        for mechanism in candidates {
            let namespace = Self {
                helper: helper.clone(),
                dns: dns_address,
                etc: etc.clone(),
                user,
                mechanism,
            };
            // With no program, the helper makes the namespace and exits.
            match namespace.command(env, None).output() {
                Ok(output) if output.status.success() => return Ok(namespace),
                Ok(output) => refusals.push(MechanismRefusal {
                    mechanism,
                    reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                }),
                Err(error) => refusals.push(MechanismRefusal {
                    mechanism,
                    reason: format!("cannot start: {error}"),
                }),
            }
        }
        Err(DnsNamespaceError::Unavailable(refusals))
    }

    /// How it came up.
    pub fn mechanism(&self) -> Mechanism {
        self.mechanism
    }

    /// Runs `program`, an executable's path, with `args` in a namespace of
    /// its own, with the test's environment, until it exits, and returns its
    /// status and output. A namespace that cannot be made then exits 125
    /// with the reason on stderr.
    ///
    /// # Errors
    ///
    /// The namespace's process could not start, as [`Command::output`]
    /// reports.
    pub fn output<I, A>(
        &self,
        env: &TestEnv,
        program: impl Into<OsString>,
        args: I,
    ) -> io::Result<Output>
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        let mut child = vec![program.into()];
        child.extend(args.into_iter().map(Into::into));
        self.command(env, Some(child)).output()
    }

    /// The command that makes a namespace and, given one, runs `child`
    /// (its program and arguments) in it.
    fn command(&self, env: &TestEnv, child: Option<Vec<OsString>>) -> Command {
        let mut command = match self.mechanism {
            Mechanism::UserNamespace => env.process_command(&self.helper),
            Mechanism::Sudo => {
                let mut command = env.process_command(sudo());
                command.arg("-n").arg("--").arg(&self.helper);
                command
            }
        };
        command
            .arg(DNS_NAMESPACE_COMMAND)
            .arg("--dns")
            .arg(self.dns.to_string())
            .arg("--etc")
            .arg(&self.etc);
        if self.mechanism == Mechanism::Sudo {
            // sudo's child starts the program as the test's user again.
            command.arg("--user").arg(self.user.to_string());
        }
        // sudo clears the environment, so the child's is passed as
        // arguments either way.
        for (name, value) in env.env() {
            command.arg("--env").arg(format!("{name}={value}"));
        }
        if let Some(child) = child {
            command.arg("--").args(child);
        }
        command
    }
}

/// `sudo`'s path, where an OS keeps it.
fn sudo() -> &'static str {
    ["/usr/bin/sudo", "/bin/sudo", "/usr/local/bin/sudo"]
        .into_iter()
        .find(|path| Path::new(path).exists())
        .unwrap_or("sudo")
}

/// Writes the files the namespace mounts over the host's `/etc`, each by
/// its name there: `resolv.conf`, and `nsswitch.conf` when the host has one.
fn write_files(etc: &Path) -> io::Result<()> {
    std::fs::create_dir_all(etc)?;
    std::fs::write(etc.join("resolv.conf"), RESOLV_CONF)?;
    // A host without the file is never mounted over; glibc then resolves
    // through DNS by default.
    if let Ok(host) = std::fs::read_to_string(HOST_NSSWITCH_CONF) {
        std::fs::write(etc.join("nsswitch.conf"), hosts_from_files_and_dns(&host))?;
    }
    Ok(())
}

/// The host's `nsswitch.conf` with its `hosts` line set to `files dns`, so
/// no other source (systemd-resolved's, mDNS) answers a name.
fn hosts_from_files_and_dns(host: &str) -> String {
    let mut lines: Vec<&str> = host
        .lines()
        .filter(|line| !line.trim_start().starts_with("hosts:"))
        .collect();
    lines.push("hosts: files dns");
    let mut text = lines.join("\n");
    text.push('\n');
    text
}
