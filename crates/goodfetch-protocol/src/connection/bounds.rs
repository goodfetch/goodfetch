//! A Client's own bounds on its transport (ADRs 0079 and 0081), which
//! `connect()`'s own `settings` set (ADR 0083; issue 2518): how many calls
//! it holds waiting for their answer, how many bytes it holds waiting to be
//! written to the engine, and how many events it holds for a listener. The
//! Client applies them from its first call, `hello` included; the engine
//! only explains them, and refuses them from a surface whose Client has no
//! such bound, where they could not take effect.

use serde_json::Value;

use super::{ARRIVED_IN, Surface};
use crate::catalog::{Authority, Setting, ValueKind};

/// `connection.max_pending_calls`'s default.
const DEFAULT_MAX_PENDING_CALLS: u64 = 10_000;

/// `connection.max_queued_write_bytes`'s default: 64 MiB.
const DEFAULT_MAX_QUEUED_WRITE_BYTES: u64 = 64 * 1024 * 1024;

/// `connection.max_listener_queue`'s default.
const DEFAULT_MAX_LISTENER_QUEUE: u64 = 1_024;

fn ten_thousand() -> Value {
    Value::from(DEFAULT_MAX_PENDING_CALLS)
}

fn sixty_four_mebibytes() -> Value {
    Value::from(DEFAULT_MAX_QUEUED_WRITE_BYTES)
}

fn one_thousand_and_twenty_four() -> Value {
    Value::from(DEFAULT_MAX_LISTENER_QUEUE)
}

/// `connection.max_pending_calls` (issue 2518).
pub const CONNECTION_MAX_PENDING_CALLS: Setting = Setting {
    name: "connection.max_pending_calls",
    description: "The most calls the Client holds waiting for their answer, a whole number from 1: 10000 by default. A call past it is refused at once, unsent, with `connection_overloaded` and the bound `pending_calls`; the calls made before it go on. The TypeScript Client applies it to its own transport from its first call, `hello` included. Set in `connect()`'s own `settings` only.",
    kind: ValueKind::CountFromOne,
    default: ten_thousand,
    clearable: false,
    authority: Authority::ConnectionAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `connection.max_queued_write_bytes` (issue 2518).
pub const CONNECTION_MAX_QUEUED_WRITE_BYTES: Setting = Setting {
    name: "connection.max_queued_write_bytes",
    description: "The most bytes the Client holds waiting to be written to the engine, a whole number from 1: 64 MiB (67108864) by default. A call whose message would pass it is refused at once, unsent, with `connection_overloaded` and the bound `queued_write_bytes`; the calls made before it go on. The TypeScript Client applies it to its own transport from its first call, `hello` included. Set in `connect()`'s own `settings` only.",
    kind: ValueKind::CountFromOne,
    default: sixty_four_mebibytes,
    clearable: false,
    authority: Authority::ConnectionAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `connection.max_listener_queue` (issues 2986 and 2518).
pub const CONNECTION_MAX_LISTENER_QUEUE: Setting = Setting {
    name: "connection.max_listener_queue",
    description: "The most events the Client holds for one listener that has not taken them yet, a whole number from 1: 1024 by default. An event past it removes the listener, which ends with `listener_events_lost` naming how many it lost. The TypeScript Client applies it to its own transport. Set in `connect()`'s own `settings` only.",
    kind: ValueKind::CountFromOne,
    default: one_thousand_and_twenty_four,
    clearable: false,
    authority: Authority::ConnectionAlone,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Whether the Client of `surface` applies the settings of
/// [`Authority::ConnectionAlone`] to its transport: the TypeScript SDK's
/// alone, until the Python and Rust Clients bound theirs.
pub fn applies_client_bounds(surface: Surface) -> bool {
    match surface {
        Surface::SdkTypescript => true,
        Surface::SdkPython
        | Surface::Rust
        | Surface::Cli
        | Surface::Mcp
        | Surface::Kennel
        | Surface::Cloud
        | Surface::BrowserHost
        | Surface::Worker => false,
    }
}
