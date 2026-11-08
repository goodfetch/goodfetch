//! The endpoint rule (ADRs 0017, 0049, 0050 and 0067; spec #564): the one
//! rule set every listener `transport` opens holds what reaches it to. A
//! listener binds to loopback unless its configuration names another
//! address. Its token is 256 random bits from the OS generator, never from
//! `Entropy`, compared in time that depends only on its length, and taken
//! as `Authorization: Bearer` on HTTP and in `hello` on a socket, never
//! from a query string. An HTTP request's `Host` must name an IP address or
//! `localhost`, or one of the names the listener is reached at, never a
//! name a DNS-rebinding page could point at it, and a request carrying an
//! `Origin` must carry an allowed one: a missing `Origin` is accepted only
//! where a browser page cannot send the request cross-site, such as a
//! same-origin read or a request with a header token. [`refusal`] holds
//! both rules, each listener naming what it admits. Every refusal, of a
//! token, a `Host` or an `Origin`, is logged under [`REFUSAL_TARGET`] with
//! its [`Listener`] and its reason, and recorded in the Audit log as an
//! endpoint authentication failure, `{listener, reason}`; neither ever
//! holds a token. Listeners of other areas (the HTTP listener, MCP over
//! HTTP, the Live view, raw CDP, the peer link) are wrapped with this
//! module by their own tickets.

use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use goodfetch_protocol::audit::{
    Actor, ENDPOINT_AUTHENTICATION_FAILED, EndpointRefusal, Listener, RefusalReason,
};
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

/// The names a listener is reached at, as `host:port`, or `host` for a
/// default port: a request's `Host` names one of them, and its `Origin`
/// one of them over `http` (ADR 0017).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorities {
    names: Vec<String>,
}

impl Authorities {
    /// The names of a listener at the loopback `address`: the address
    /// itself and `localhost`, each with its port.
    pub fn loopback(address: SocketAddr) -> Self {
        let port = address.port();
        Self {
            names: vec![address.to_string(), format!("localhost:{port}")],
        }
    }

    /// These names and `name`.
    #[must_use]
    pub(crate) fn with_name(mut self, name: impl Into<String>) -> Self {
        self.names.push(name.into());
        self
    }

    /// Only `name`.
    pub(crate) fn only(name: impl Into<String>) -> Self {
        Self {
            names: vec![name.into()],
        }
    }

    /// The origins of these names over `http`, as a CSP source list does.
    pub(crate) fn origins(&self) -> Vec<String> {
        self.names
            .iter()
            .map(|name| format!("http://{name}"))
            .collect()
    }

    /// Whether `host`, a request's `Host`, is one of these names.
    pub(crate) fn admits_host(&self, host: &str) -> bool {
        self.names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(host))
    }

    /// Whether `origin`, a request's `Origin`, is one of these names over
    /// `http`.
    fn admits_origin(&self, origin: &str) -> bool {
        origin
            .strip_prefix("http://")
            .is_some_and(|authority| self.admits_host(authority))
    }
}

/// What a listener admits as a request's `Host`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Hosts<'a> {
    /// An IP address or `localhost`, with any port: a listener whose names
    /// its operator chooses, such as the ops listener a pod's address
    /// reaches.
    Address,
    /// One of the names it is reached at.
    Named(&'a Authorities),
}

/// What a listener admits as a request's `Origin`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Origins<'a> {
    /// None: no browser page calls the listener.
    Forbidden,
    /// One of these names over `http`, and a missing one is refused: a
    /// request a page of another origin could send, such as a WebSocket
    /// handshake or a write.
    Required(&'a [&'a Authorities]),
    /// One of these names over `http`, or none, which a same-origin read
    /// sends.
    Allowed(&'a [&'a Authorities]),
}

/// Why a request was refused for its `Host` or its `Origin`, by its
/// endpoint table's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    HostForeign,
    OriginMissing,
    OriginForeign,
}

impl Refusal {
    /// The rule, whose name the refusal's log record and Audit log entry
    /// carry.
    pub(crate) fn reason(self) -> RefusalReason {
        match self {
            Self::HostForeign => RefusalReason::HostForeign,
            Self::OriginMissing => RefusalReason::OriginMissing,
            Self::OriginForeign => RefusalReason::OriginForeign,
        }
    }
}

/// The rule a request with `host` and `origin` breaks at a listener that
/// admits `hosts` and `origins`, if any: its `Host` first, then its
/// `Origin`.
pub(crate) fn refusal(
    host: Option<&str>,
    origin: Option<&str>,
    hosts: Hosts<'_>,
    origins: Origins<'_>,
) -> Option<Refusal> {
    let host_admitted = match hosts {
        Hosts::Address => names_an_address(host),
        Hosts::Named(authorities) => host.is_some_and(|host| authorities.admits_host(host)),
    };
    if !host_admitted {
        return Some(Refusal::HostForeign);
    }
    let admitted = |origin: &str, among: &[&Authorities]| {
        among
            .iter()
            .any(|authorities| authorities.admits_origin(origin))
    };
    match (origins, origin) {
        (Origins::Forbidden | Origins::Allowed(_), None) => None,
        (Origins::Required(_), None) => Some(Refusal::OriginMissing),
        (Origins::Forbidden, Some(_)) => Some(Refusal::OriginForeign),
        (Origins::Required(among) | Origins::Allowed(among), Some(origin)) => {
            (!admitted(origin, among)).then_some(Refusal::OriginForeign)
        }
    }
}

/// Whether an HTTP request's `Host`, with or without a port, names an IP
/// address or `localhost`: a name a DNS-rebinding page could point at the
/// listener does not, and neither does a missing one.
fn names_an_address(host: Option<&str>) -> bool {
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

    /// Logs and records that `listener` refused an attempt for `reason`,
    /// such as `token_wrong`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the entry recorded.
    pub async fn refused(&self, listener: Listener, reason: RefusalReason) {
        tracing::warn!(
            target: REFUSAL_TARGET,
            listener = listener.as_str(),
            reason = reason.as_str(),
            "a listener refused an attempt"
        );
        let audit = self.audit.clone();
        let now = self.clock.timestamp();
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
                    serde_json::to_value(EndpointRefusal { listener, reason })
                        .expect("invariant: a refusal's details serialize"),
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
