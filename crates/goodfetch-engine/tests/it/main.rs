//! `goodfetch-engine` through its public interface, with real local
//! stand-ins (ADR 0062): what binds a socket or starts a process, such as
//! a name server on loopback.
#![expect(
    clippy::disallowed_types,
    reason = "the tests serve stand-ins on loopback sockets (ADR 0062)"
)]
// A disallowed method is expected only on the item that calls it, so a new
// call elsewhere, such as `Instant::now`, fails the lint.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod certificates;
mod children;
mod dns;
mod downloads;
mod egress;
mod governor;
mod happy_eyeballs;
mod held_clock;
mod name_server;
mod nothing_leaves;
mod resolved_address;
mod resolver_cache;
mod trust_on_first_use;
