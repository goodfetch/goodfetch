//! The bounds of a Connection's opening (#2324): how long a Client of a
//! listener that takes a token has to send its `hello`, and how long a line
//! it may send before that `hello` is accepted.

use serde_json::Value;

use super::ARRIVED_IN;
use crate::catalog::{Authority, Setting, ValueKind};

/// `timeout.hello`'s default, in milliseconds: 5 seconds.
const DEFAULT_HELLO_TIMEOUT_MS: u64 = 5_000;

/// `network.max_hello_size`'s default: 64 KiB, many times any `hello`.
const DEFAULT_MAX_HELLO_SIZE: u64 = 64 * 1024;

/// The largest `network.max_hello_size` takes: the 16 MiB every message
/// of a Connection is held to, past which a larger opening limit could
/// not apply.
const MAX_HELLO_SIZE_CEILING: u64 = 16 * 1024 * 1024;

fn five_seconds() -> Value {
    Value::from(DEFAULT_HELLO_TIMEOUT_MS)
}

fn sixty_four_kibibytes() -> Value {
    Value::from(DEFAULT_MAX_HELLO_SIZE)
}

/// `timeout.hello` (#2324).
pub const TIMEOUT_HELLO: Setting = Setting {
    name: "timeout.hello",
    description: "How long a Connection to a listener that takes a token, a Daemon's or an Attached engine's private endpoint, may take to send its complete `hello`, from its accept: 5 s. One that has not by then is closed unanswered, and the refusal logged and recorded as `hello_timed_out`. An Attached engine's stdio takes no token and has no such deadline. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: five_seconds,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `network.max_hello_size` (#2324).
pub const NETWORK_MAX_HELLO_SIZE: Setting = Setting {
    name: "network.max_hello_size",
    description: "The most bytes of one line a Connection to a listener that takes a token may send before its `hello` is accepted, up to the 16 MiB (16777216) of any message: 64 KiB (65536) by default, many times any `hello`. A longer line closes the Connection unanswered as soon as it passes the limit, without being decoded, and the refusal is logged and recorded as `hello_too_large`. Set in user configuration or an Operator policy only.",
    kind: ValueKind::CountUpTo(MAX_HELLO_SIZE_CEILING),
    default: sixty_four_kibibytes,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
