//! The endpoint rule (ADRs 0017, 0049, 0050 and 0067; spec #564): the one
//! rule set every listener `transport` opens holds what reaches it to. A
//! listener binds to loopback unless its configuration names another
//! address. Its token is 256 random bits from the OS generator, never from
//! `Entropy`, compared in time that depends only on its length, and taken
//! as `Authorization: Bearer` on HTTP and in `hello` on a socket, never
//! from a query string. An HTTP request's `Host` must name an IP address or
//! `localhost`, never a name a DNS-rebinding page could point at it, and a
//! request carrying an `Origin` must carry an allowed one: a missing
//! `Origin` is accepted only with a header token, which a browser page
//! cannot send cross-site. Every refusal, of a token, a `Host` or an
//! `Origin`, is logged under [`REFUSAL_TARGET`] with its listener and its
//! reason, and recorded in the Audit log as an endpoint authentication
//! failure, `{listener, reason}`; neither ever holds a token. Listeners of
//! other areas (the HTTP listener, MCP over HTTP, the Live view, raw CDP,
//! the peer link) are wrapped with this module by their own tickets.

use std::fmt::Write as _;
use std::net::IpAddr;
use std::sync::Arc;

use goodfetch_protocol::audit::{Actor, ENDPOINT_AUTHENTICATION_FAILED};
use jiff::Timestamp;
use serde_json::json;
use subtle::ConstantTimeEq as _;

use crate::audit::{AuditLog, Origin};
use crate::clock::Clock;
use crate::tasks::TaskOwner;

/// The `tracing` target of a listener's refusals.
pub const REFUSAL_TARGET: &str = "goodfetch::rpc::refusal";

/// How many bytes of the OS generator a token takes: 256 bits.
const TOKEN_BYTES: usize = 32;

/// A new token from the OS generator, never from `Entropy` (ADR 0059): 32
/// bytes, in lowercase hex.
///
/// # Errors
///
/// When the OS generator gives nothing.
pub fn new_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().fold(String::new(), |mut token, byte| {
        let _ = write!(token, "{byte:02x}");
        token
    }))
}

/// Whether `given` is the secret `expected`, such as a token or a MAC, in
/// time that depends only on their lengths, so a Client cannot guess one
/// byte at a time.
pub fn same_token(given: &str, expected: &str) -> bool {
    bool::from(given.as_bytes().ct_eq(expected.as_bytes()))
}

/// Whether an HTTP request's `Host`, with or without a port, names an IP
/// address or `localhost`: a name a DNS-rebinding page could point at the
/// listener does not, and neither does a missing one.
pub(crate) fn names_an_address(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let name = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok()
}

/// Where a listener records its refusals: the user's Audit log, each entry
/// written on the blocking policy of the engine's owner at the clock's wall
/// time. A listener outside `transport`, such as a Session's proxy relay,
/// takes its Run's ([`crate::runs::Run::refusals`]).
#[derive(Clone)]
pub struct Refusals {
    audit: AuditLog,
    owner: Arc<TaskOwner>,
    clock: Clock,
}

impl Refusals {
    /// Refusals recorded in `audit`, written on `owner`'s blocking policy
    /// at `clock`'s wall time.
    pub fn new(audit: AuditLog, owner: Arc<TaskOwner>, clock: Clock) -> Self {
        Self {
            audit,
            owner,
            clock,
        }
    }

    /// Logs and records that `listener`, by its endpoint table name,
    /// refused an attempt for `reason`, such as `token_wrong`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the entry recorded.
    pub async fn refused(&self, listener: &'static str, reason: &'static str) {
        tracing::warn!(target: REFUSAL_TARGET, listener, reason, "a listener refused an attempt");
        let audit = self.audit.clone();
        let now = Timestamp::try_from(self.clock.wall()).unwrap_or(Timestamp::UNIX_EPOCH);
        self.owner
            .blocking(move || {
                let origin = Origin {
                    actor: Actor::Engine,
                    surface: None,
                    layer: None,
                    run_id: None,
                };
                if let Err(error) = audit.record(
                    &origin,
                    &ENDPOINT_AUTHENTICATION_FAILED,
                    json!({ "listener": listener, "reason": reason }),
                    now,
                ) {
                    tracing::error!(%error, "the Audit log did not record a listener's refusal");
                }
            })
            .await;
    }
}

#[cfg(test)]
mod tests;
