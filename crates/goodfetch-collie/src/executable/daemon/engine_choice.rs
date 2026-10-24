//! A generated command's side of the Daemon (`goodfetch_cli::engine_choice`):
//! the remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token
//! (ADRs 0049 and 0083), never a flag, so a token never sits in a process
//! list; else the running Daemon of the major, or the pick this starts; and
//! the `fix` of a refusal, naming this executable's `daemon restart`.

use goodfetch_cli::engine_choice::{Daemons, Reached};
use goodfetch_engine::settings::deployment_variable;
use goodfetch_protocol::daemon::DaemonEndpoint;

use super::{Failure, Machine};

/// The deployment variable that names a remote Daemon (ADRs 0049 and
/// 0083), which a command reaches instead of its major's.
const URL_VARIABLE: &str = "GOODFETCH_URL";

/// The deployment variable that holds the remote Daemon's token, which
/// nothing prints (ADR 0083): never a flag, so it never sits in a process
/// list.
const TOKEN_VARIABLE: &str = "GOODFETCH_TOKEN";

impl Machine<'_> {
    /// The remote Daemon `GOODFETCH_URL` names, with `GOODFETCH_TOKEN`'s
    /// token; `None` when the variable is unset.
    fn remote(&self) -> Result<Option<Reached>, String> {
        let Some(url) = deployment_variable(self.environment, URL_VARIABLE) else {
            return Ok(None);
        };
        let address = remote_address(&url.to_string_lossy())?;
        let token = deployment_variable(self.environment, TOKEN_VARIABLE).ok_or_else(|| {
            format!("`{URL_VARIABLE}` names a Daemon, and `{TOKEN_VARIABLE}` holds no token for it: set `{TOKEN_VARIABLE}` to the token the Daemon takes")
        })?;
        Ok(Some(Reached {
            endpoint: DaemonEndpoint::Tcp(address),
            token: token.to_string_lossy().into_owned(),
            remote: true,
        }))
    }
}

/// A generated command's side of the Daemon (`engine_choice`): the remote
/// one `GOODFETCH_URL` names, else the running Daemon of the major or the
/// pick this starts.
impl Daemons for Machine<'_> {
    fn find(&self, start: bool) -> Result<Option<Reached>, String> {
        if let Some(remote) = self.remote()? {
            return Ok(Some(remote));
        }
        let found = if start {
            Some(self.running_or_started(false).map_err(Failure::message)?.0)
        } else {
            self.running().map_err(Failure::message)?
        };
        Ok(found.map(|found| Reached {
            endpoint: found.endpoint,
            token: found.token,
            remote: false,
        }))
    }

    fn handed_over(&self) -> Result<(), String> {
        // Once the start lock is free, the pick runs.
        let start = self
            .files()
            .start_lock()
            .map_err(|error| format!("the Daemon's start lock cannot be taken: {error}"))?;
        drop(start);
        Ok(())
    }

    fn refusal_fix(&self, given: &str) -> String {
        self.fix_of(given)
    }
}

/// The address of the Daemon `url`, a `GOODFETCH_URL`, names: `ws://`
/// with an IP address or `localhost` and a port. The command reaches it
/// with newline-delimited JSON-RPC, which an Exposed Daemon serves beside
/// WebSocket on the same address (ADR 0049); `wss://` waits for the
/// Daemon's TLS (area 4).
fn remote_address(url: &str) -> Result<std::net::SocketAddr, String> {
    let refused = |why: &str| format!("`{URL_VARIABLE}` is `{url}`, {why}");
    let Some(authority) = url.strip_prefix("ws://") else {
        return Err(refused(
            "which names no Daemon: give `ws://<address>:<port>`, such as `ws://127.0.0.1:7878`; `wss://` waits for the Daemon's TLS",
        ));
    };
    let authority = authority.trim_end_matches('/');
    let address = match authority.strip_prefix("localhost:") {
        Some(port) => format!("127.0.0.1:{port}"),
        None => authority.to_owned(),
    };
    address.parse().map_err(|_| {
        refused("whose address is no IP address or `localhost` with a port, such as `ws://127.0.0.1:7878`")
    })
}
