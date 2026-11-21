//! Which engine a connection reaches (ADRs 0024, 0078, 0083 and 0098).
//! [`EngineChoice::Auto`], the default, reaches the remote Daemon
//! `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token, else the Daemon of
//! the SDK's major running in the Goodfetch home, else the linked engine.
//! [`EngineChoice::Linked`] and [`EngineChoice::Daemon`] choose one of the
//! two, and [`EngineChoice::Remote`] a remote Daemon, whatever the
//! environment says; there is no `GOODFETCH_ENGINE`. A program linking the
//! engine never starts the Daemon, since it cannot run as one (ADR 0098).
//!
//! A remote Daemon's address is `ws://` on loopback, reached with
//! newline-delimited JSON-RPC over TCP, which an Exposed Daemon serves
//! beside WebSocket, as the CLI reaches it (ADR 0024's note of
//! 2026-10-23): `wss://` and addresses beyond loopback wait for the
//! Daemon's TLS, so a token never crosses the network in the clear. A
//! token goes into `hello` alone, never into a log, an error, a `Debug`
//! output or [`EngineInfo`] (ADR 0067).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use goodfetch_collie_client::{Error, ErrorCode};
use goodfetch_engine::Channel;
use goodfetch_engine::daemon::files::DaemonFiles;
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::deployment_variable;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{DaemonEndpoint, ProductVersion};
use goodfetch_protocol::daemon::DaemonToken;
use goodfetch_protocol::settings::{SettingInvalidDetails, TOKEN_VARIABLE, URL_VARIABLE};

/// The engine a connection reaches, which
/// [`ConnectOptions::engine`](crate::ConnectOptions::engine) sets.
#[derive(Clone, Debug, Default)]
pub enum EngineChoice {
    /// The remote Daemon `GOODFETCH_URL` names, else the Daemon running in
    /// the Goodfetch home, else the linked engine.
    #[default]
    Auto,
    /// The engine linked into the program, which the connection starts in
    /// its own process.
    Linked,
    /// The Daemon of the SDK's major running in the Goodfetch home, which
    /// the connection never starts (ADR 0098).
    Daemon,
    /// A remote Daemon: an Exposed Daemon or Goodfetch Cloud.
    Remote(RemoteDaemon),
}

/// A remote Daemon and the token its `hello` takes, which nothing the SDK
/// shows holds: its `Debug` output shows neither, as the URL may hold a
/// credential until the connection refuses it.
#[derive(Clone)]
pub struct RemoteDaemon {
    url: String,
    token: DaemonToken,
}

impl RemoteDaemon {
    /// The remote Daemon at `url`, `ws://` with a loopback IP address or
    /// `localhost` and a port, such as `ws://127.0.0.1:7878`, which takes
    /// `token`. The connection checks `url`, and refuses one that names no
    /// such address with `setting_invalid`.
    #[must_use]
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: DaemonToken::new(token),
        }
    }
}

impl fmt::Debug for RemoteDaemon {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteDaemon")
            .finish_non_exhaustive()
    }
}

/// How the engine a connection reached is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EngineKind {
    /// The engine linked into the program, which the connection started.
    Linked,
    /// The Daemon of this machine.
    Daemon,
    /// A remote Daemon.
    Remote,
}

impl EngineKind {
    /// Its name, as the other SDKs' `engine().kind` gives it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Daemon => "daemon",
            Self::Remote => "remote",
        }
    }
}

impl fmt::Display for EngineKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The engine a connection reached, fixed when it opened (G6). It never
/// holds a Daemon's token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineInfo {
    /// How it is reached.
    pub kind: EngineKind,
    /// The engine's Product: Foxhound's serves a Collie connection too
    /// (ADR 0095).
    pub product: Product,
    /// The engine's Product version, reported, never compared (ADR 0090).
    pub version: ProductVersion,
    /// The protocol version `major.minor` it speaks.
    pub protocol_version: ProtocolVersion,
    /// The project file the engine read, or `None` when it read none.
    pub project_file: Option<PathBuf>,
    /// The Goodfetch home the engine runs in, or `None` for a remote
    /// Daemon, whose home is its own machine's.
    pub home: Option<PathBuf>,
    /// The engine executable the connection started, or `None` when it
    /// started none, as for the linked engine or a Daemon.
    pub engine_path: Option<PathBuf>,
    /// Where a Daemon was reached: a remote one's URL, or this machine's
    /// socket path or loopback address; `None` for the linked engine.
    pub url: Option<String>,
    /// Why `auto` reached another engine than the Daemon it found, and what
    /// to do; `None` when it reached the one it chose.
    pub fallback: Option<EngineFallback>,
}

/// Why `auto` reached another engine than the Daemon it found (ADR 0098).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineFallback {
    /// Why the Daemon was not reached.
    pub cause: String,
    /// What to do so it is.
    pub fix: String,
}

/// The engine chosen, before `hello`.
pub(crate) enum Reach {
    /// The linked engine, which the connection starts.
    Linked,
    /// A Daemon, of this machine or remote.
    Daemon(DaemonReach),
}

/// A Daemon chosen: where it listens and the token its `hello` takes.
pub(crate) struct DaemonReach {
    pub(crate) endpoint: DaemonEndpoint,
    pub(crate) token: DaemonToken,
    /// The remote Daemon's URL and what named it, `engine` or
    /// `GOODFETCH_URL`; `None` for the Daemon running in the home.
    remote: Option<(String, &'static str)>,
}

impl DaemonReach {
    /// Its kind.
    pub(crate) fn kind(&self) -> EngineKind {
        if self.remote.is_some() {
            EngineKind::Remote
        } else {
            EngineKind::Daemon
        }
    }

    /// The Goodfetch home it runs in, as [`EngineInfo::home`] gives it:
    /// `home`, whose Daemon it is, or `None` for a remote Daemon.
    pub(crate) fn home(&self, home: Option<&Home>) -> Option<PathBuf> {
        match self.remote {
            Some(_) => None,
            None => home.map(|home| home.root().to_owned()),
        }
    }

    /// Where it listens, as [`EngineInfo::url`] gives it.
    pub(crate) fn url(&self) -> String {
        match &self.remote {
            Some((url, _)) => url.clone(),
            None => self.endpoint.to_string(),
        }
    }

    /// What fails the connect when opening the connection failed with
    /// `error` before `hello` was answered: `daemon_unreachable` when
    /// nothing answered where it listens, or it closed the connection
    /// before it answered, as a Daemon does for a token it does not take;
    /// `error` itself otherwise. `home` is the one whose Daemon it is.
    pub(crate) fn unreached(&self, error: Error, home: Option<&Home>) -> Error {
        let closed = error.code() == ErrorCode::CLOSED;
        if !closed && error.code() != ErrorCode::CONNECTION_LOST {
            return error;
        }
        let url = self.url();
        let (message, fix) = match (&self.remote, closed) {
            (_, true) => (
                format!(
                    "The Daemon at {url} closed the connection before it answered hello, as one does for a token it does not take"
                ),
                match &self.remote {
                    Some((_, URL_VARIABLE)) => {
                        format!("Set {TOKEN_VARIABLE} to the token the Daemon at {url} takes.")
                    }
                    Some(_) => format!("Give `engine` the token the Daemon at {url} takes."),
                    None => format!(
                        "Restart the Daemon with `daemon restart`, which writes a new runtime file{}.",
                        home.map_or_else(String::new, |home| format!(
                            " in {}",
                            home.root().display()
                        ))
                    ),
                },
            ),
            (Some((_, source)), false) => (
                format!("Nothing answers at {url}, the Daemon {source} names"),
                format!(
                    "Start the Daemon {url} names, reach it through a forwarder that listens on loopback, such as an SSH tunnel, or leave `engine` at `Auto` with {URL_VARIABLE} unset to use this machine's engine."
                ),
            ),
            (None, false) => (
                format!(
                    "Nothing answers at {url}, where the Daemon's runtime file says it listens"
                ),
                "Restart the Daemon with `daemon restart`.".to_owned(),
            ),
        };
        Error::daemon_unreachable(message, fix)
    }
}

/// Chooses the engine `choice` names for a connection whose engine is
/// `product`'s at `version`, reading `GOODFETCH_URL` and `GOODFETCH_TOKEN`
/// of `environment` at `Auto` alone, and the runtime file of the Daemon of
/// `home`.
///
/// # Errors
///
/// `setting_invalid` when a remote Daemon's URL names no loopback `ws://`
/// address, or `GOODFETCH_URL` comes without `GOODFETCH_TOKEN`;
/// `daemon_unreachable` when `choice` is `Daemon` and none runs, or the
/// Daemon's runtime file cannot be read.
pub(crate) fn reach(
    choice: EngineChoice,
    environment: &BTreeMap<OsString, OsString>,
    home: Option<&Home>,
    product: Product,
    version: &ProductVersion,
) -> Result<Reach, Error> {
    match choice {
        EngineChoice::Linked => Ok(Reach::Linked),
        EngineChoice::Remote(remote) => remote_reach(remote, "engine"),
        EngineChoice::Daemon => {
            let Some(home) = home else {
                return Err(Error::daemon_unreachable(
                    "No Goodfetch home names where the Daemon is found".to_owned(),
                    "Set GOODFETCH_HOME to the Goodfetch home, or a home folder the OS knows."
                        .to_owned(),
                ));
            };
            running(home, version)?.ok_or_else(|| none_running(home, product, version))
        }
        EngineChoice::Auto => {
            if let Some(remote) = remote_of_environment(environment)? {
                return remote_reach(remote, URL_VARIABLE);
            }
            match home {
                Some(home) => Ok(running(home, version)?.unwrap_or(Reach::Linked)),
                None => Ok(Reach::Linked),
            }
        }
    }
}

/// The remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token;
/// `None` when it is unset or empty. Its URL is judged first, as the CLI
/// judges it.
fn remote_of_environment(
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Option<RemoteDaemon>, Error> {
    let Some(url) = deployment_variable(environment, URL_VARIABLE) else {
        return Ok(None);
    };
    let url = url.to_string_lossy().into_owned();
    remote_address(&url, URL_VARIABLE)?;
    let Some(token) = deployment_variable(environment, TOKEN_VARIABLE) else {
        return Err(Error::setting_invalid(
            format!("{URL_VARIABLE} names a Daemon, and {TOKEN_VARIABLE} holds no token for it"),
            &SettingInvalidDetails {
                key: TOKEN_VARIABLE.to_owned(),
                layer: None,
                reason: format!("{URL_VARIABLE} names a Daemon, and it holds no token"),
                limit: None,
                fix: Some(format!(
                    "Set {TOKEN_VARIABLE} to the token the Daemon {URL_VARIABLE} names takes, or unset {URL_VARIABLE} to use this machine's engine."
                )),
            },
        ));
    };
    Ok(Some(RemoteDaemon::new(url, token.to_string_lossy())))
}

/// The remote Daemon `remote`, which `source` named, once its URL is
/// checked.
fn remote_reach(remote: RemoteDaemon, source: &'static str) -> Result<Reach, Error> {
    let address = remote_address(&remote.url, source)?;
    Ok(Reach::Daemon(DaemonReach {
        endpoint: DaemonEndpoint::Tcp(address),
        token: remote.token,
        remote: Some((remote.url, source)),
    }))
}

/// The address a remote Daemon's URL, which `source` gave, names: `ws://`
/// with a loopback IP address or `localhost` and a port, and nothing else,
/// so no credential in it reaches a log, an error or [`EngineInfo`].
///
/// # Errors
///
/// `setting_invalid` naming `source` otherwise, without the URL when it is
/// no URL or holds more than a scheme and an address: a user, a password,
/// a path, a query or a fragment, which may be a secret.
fn remote_address(url: &str, source: &str) -> Result<SocketAddr, Error> {
    let refused = |why: &str, shown: bool| {
        let what = if shown {
            format!("{url} {why}")
        } else {
            why.to_owned()
        };
        Error::setting_invalid(
            format!("{source} {what}"),
            &SettingInvalidDetails {
                key: source.to_owned(),
                layer: None,
                reason: what,
                limit: None,
                fix: Some(format!(
                    "Give {source} a Daemon's address on loopback, ws://<address>:<port>, such as ws://127.0.0.1:7878: a Daemon beyond loopback is reached through a forwarder that listens on it, such as an SSH tunnel, until the Daemon terminates TLS."
                )),
            },
        )
    };
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(refused("is no URL", false));
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.contains(['@', '?', '#', '/']) {
        return Err(refused(
            "holds more than an address: a Daemon's address has no user, password, path, query or fragment, and its token goes in its own setting",
            false,
        ));
    }
    if !scheme.eq_ignore_ascii_case("ws") {
        return Err(refused(
            "names no Daemon: give ws://<address>:<port>; wss:// waits for the Daemon's TLS",
            true,
        ));
    }
    let no_port = || refused("has no port, where a Daemon's address has one", true);
    let address = match authority.rsplit_once(':') {
        Some((host, port)) if host.eq_ignore_ascii_case("localhost") => port
            .parse::<u16>()
            .map(|port| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .map_err(|_| no_port())?,
        Some(_) => authority.parse::<SocketAddr>().map_err(|_| {
            refused(
                "has no IP address or `localhost` with a port, such as ws://127.0.0.1:7878",
                true,
            )
        })?,
        None => return Err(no_port()),
    };
    if !address.ip().is_loopback() {
        return Err(refused(
            "names an address beyond loopback, where its token would cross the network in the clear until the Daemon terminates TLS",
            true,
        ));
    }
    Ok(address)
}

/// The Daemon of `version`'s major running in `home`, whose runtime file
/// names it and which holds its serving lock; `None` when none runs.
///
/// # Errors
///
/// `daemon_unreachable` when its files cannot be read.
fn running(home: &Home, version: &ProductVersion) -> Result<Option<Reach>, Error> {
    let files = DaemonFiles::of(home, version);
    match files.running() {
        Ok(found) => Ok(found.map(|found| {
            Reach::Daemon(DaemonReach {
                endpoint: found.endpoint,
                token: found.token,
                remote: None,
            })
        })),
        Err(error) => {
            let file = files.runtime_file();
            Err(Error::daemon_unreachable(
                format!(
                    "The Daemon's runtime file {} cannot be read: {error}",
                    file.display()
                ),
                format!(
                    "Stop the Daemon with `daemon stop`, or remove {} when none runs, then connect again.",
                    file.display()
                ),
            ))
        }
    }
}

/// `daemon_unreachable` for `Daemon` when none of `version`'s major runs in
/// `home`: a program linking the engine never starts one (ADR 0098), so
/// its fix installs the Product's executable at `version` and starts it.
fn none_running(home: &Home, product: Product, version: &ProductVersion) -> Error {
    let executable = product.executable_name();
    Error::daemon_unreachable(
        format!(
            "No Daemon of {} {} runs in {}, and a program linking the engine starts none",
            product.name(),
            version,
            home.root().display()
        ),
        format!(
            "Install {}'s executable at {version} (`{}`), start the Daemon with `{executable} daemon start`, then connect again; or leave `engine` at `Auto` to link the engine.",
            product.name(),
            Channel::CratesIo.install_command(product)
        ),
    )
}

#[cfg(test)]
mod tests;
