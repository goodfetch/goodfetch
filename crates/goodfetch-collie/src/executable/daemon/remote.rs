//! The remote Daemon a command's call reaches (`crate::executable::engine_choice`)
//! in place of its major's: the one `GOODFETCH_URL` names with
//! `GOODFETCH_TOKEN`'s token (ADRs 0049 and 0083), never a flag, so a token
//! never sits in a process list. A
//! `GOODFETCH_URL` must name a loopback address until the Daemon
//! terminates TLS, as `daemon.address` must (ADR 0049), since the token
//! would otherwise cross the network in the clear: a forwarder listens on
//! loopback, as an SSH tunnel does.

use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{SettingsError, deployment_variable};
use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonToken};
use goodfetch_protocol::jsonrpc::ErrorObject;
use goodfetch_protocol::settings::{TOKEN_VARIABLE, URL_VARIABLE};

use super::{Machine, Reach};

impl Machine<'_> {
    /// The remote Daemon `GOODFETCH_URL` names, with `GOODFETCH_TOKEN`'s
    /// token; `None` when the variable is unset.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a `GOODFETCH_URL` that names no loopback
    /// Daemon, or a `GOODFETCH_TOKEN` that holds no token.
    pub(super) fn remote(&self) -> Result<Option<Reach>, ErrorObject> {
        let Some(url) = deployment_variable(self.environment, URL_VARIABLE) else {
            return Ok(None);
        };
        let address = remote_address(&url.to_string_lossy())
            .map_err(|reason| self.invalid(URL_VARIABLE, reason))?;
        let token = deployment_variable(self.environment, TOKEN_VARIABLE).ok_or_else(|| {
            self.invalid(
                TOKEN_VARIABLE,
                format!("`{URL_VARIABLE}` names a Daemon, and `{TOKEN_VARIABLE}` holds no token for it: set it to the token that Daemon takes"),
            )
        })?;
        Ok(Some(Reach {
            endpoint: DaemonEndpoint::Tcp(address),
            token: DaemonToken::new(token.to_string_lossy()),
            service: false,
            remote: true,
        }))
    }

    /// `setting_invalid` for the deployment variable `variable`, `reason`
    /// saying why.
    fn invalid(&self, variable: &str, reason: String) -> ErrorObject {
        let error = SettingsError::Invalid {
            key: variable.to_owned(),
            layer: None,
            reason,
        };
        local_settings_error(&error, self.identity).to_error_object()
    }
}

/// The address of the Daemon `url`, a `GOODFETCH_URL`, names: `ws://`
/// with a loopback IP address or `localhost` and a port. The command
/// reaches it with newline-delimited JSON-RPC, which an Exposed Daemon
/// serves beside WebSocket on the same address (ADR 0049); `wss://` waits
/// for the Daemon's TLS (area 4), and until then an address beyond
/// loopback would carry the token in the clear.
fn remote_address(url: &str) -> Result<std::net::SocketAddr, String> {
    let refused = |why: &str| format!("`{url}` {why}");
    let Some(authority) = url.strip_prefix("ws://") else {
        return Err(refused(
            "names no Daemon: give `ws://<address>:<port>`, such as `ws://127.0.0.1:7878`; `wss://` waits for the Daemon's TLS",
        ));
    };
    let authority = authority.trim_end_matches('/');
    let address = match authority.strip_prefix("localhost:") {
        Some(port) => format!("127.0.0.1:{port}"),
        None => authority.to_owned(),
    };
    let address: std::net::SocketAddr = address.parse().map_err(|_| {
        refused("has no IP address or `localhost` with a port, such as `ws://127.0.0.1:7878`")
    })?;
    if !address.ip().is_loopback() {
        return Err(refused(
            "names an address beyond loopback, where its token would cross the network in the clear until the Daemon terminates TLS: reach it through a forwarder that listens on loopback, such as an SSH tunnel",
        ));
    }
    Ok(address)
}
