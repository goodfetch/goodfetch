//! `goodfetch-engine` through its public interface, with real local
//! stand-ins (ADR 0062): what binds a socket or starts a process.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests serve a stand-in on a loopback socket and lay out files by hand (ADR 0062)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod downloads;
