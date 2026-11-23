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

// The executables' count of the live heap, around the system's allocator,
// which `heap.rs` reads (#1768); every other test allocates as before.
#[global_allocator]
static ALLOCATOR: goodfetch_engine::heap::Counting<std::alloc::System> =
    goodfetch_engine::heap::Counting::new(std::alloc::System);

// B3's Audit entry scenario, which `counted_audit` checks.
#[path = "../../benches/audit.rs"]
#[expect(
    clippy::disallowed_methods,
    reason = "the scenario writes its home's settings file before the log opens"
)]
#[expect(dead_code, reason = "what only the counts cell reads of the scenario")]
mod audit_scenario;
mod charge_counts;
mod children;
mod counted_audit;
mod counted_governor;
mod counted_messages;
mod dns;
mod downloads;
mod extra_loopback;
mod governor;
// B3's governor scenario, which `counted_governor` checks.
#[path = "../../benches/governor.rs"]
mod governor_scenario;
mod happy_eyeballs;
mod heap;
mod held_clock;
#[cfg(target_os = "linux")]
mod lazyfs;
mod ledger_look;
// B3's message scenario, which `counted_messages` checks.
#[path = "../../benches/messages.rs"]
#[expect(
    clippy::disallowed_methods,
    reason = "the scenario runs on the test Clock and Entropy (ADR 0059)"
)]
#[expect(
    dead_code,
    reason = "what only the Valgrind targets read of the scenario"
)]
mod message_scenario;
mod name_server;
mod nothing_leaves;
mod operator_endpoints;
#[cfg(target_os = "linux")]
mod politeness_store;
mod resolver_cache;
