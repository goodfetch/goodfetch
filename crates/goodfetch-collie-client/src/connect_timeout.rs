//! How long opening a Connection waits for `hello`'s answer (ADRs 0080 and
//! 0083): `timeout.connect`, 30 s, or the connection's own `timeout`, in
//! its three states. Past it, the opening fails with `deadline_exceeded`,
//! whose `limit` names `timeout.connect` and the layer that set it.

use std::time::Duration;

use goodfetch_protocol::catalog::ErrorCodeDefinition;
use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails, WaitedFor};
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::settings::{
    RunOrigin, SETTING_INVALID, SettingInvalidDetails, SettingLimit, SettingsLayer, TIMEOUT_CONNECT,
};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::call::wire_millis;
use crate::error::Error;

/// What a refused connect `timeout` tells its program to give instead.
const TIMEOUT_FIX: &str = "give at least 1 ms, or `.no_timeout()` for no timeout";

/// How long opening a Connection waits for the engine to answer `hello`
/// (ADR 0080): `timeout.connect` when left as it is, no deadline, or a
/// duration of the connection's own, given as an argument.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectTimeout {
    /// `timeout.connect`'s default, 30 s.
    #[default]
    Default,
    /// As long as it takes.
    Off,
    /// At most this long: `timeout.connect` at the run layer, given as an
    /// argument.
    After(Duration),
}

impl ConnectTimeout {
    /// Refuses a deadline that would end the opening at once, before
    /// anything starts.
    ///
    /// # Errors
    ///
    /// [`Error::Failed`] with `setting_invalid` for a zero duration, its
    /// `limit` naming `timeout.connect` at the run layer, given as an
    /// argument, and its `fix` the form with no timeout.
    pub fn check(self) -> Result<(), Error> {
        if self != Self::After(Duration::ZERO) {
            return Ok(());
        }
        let reason = "a timeout of 0 ms would end the opening at once";
        let details = SettingInvalidDetails {
            key: TIMEOUT_CONNECT.name.to_owned(),
            layer: Some(SettingsLayer::Run),
            reason: reason.to_owned(),
            limit: Some(argument(Duration::ZERO)),
            fix: Some(TIMEOUT_FIX.to_owned()),
        };
        Err(failed(
            &SETTING_INVALID,
            format!(
                "The connection's `timeout`, its `{}`: {reason}; {TIMEOUT_FIX}.",
                TIMEOUT_CONNECT.name
            ),
            &details,
        ))
    }

    /// The deadline of `hello`'s answer and the setting that set it; none
    /// for [`ConnectTimeout::Off`].
    pub(crate) fn deadline(self) -> Option<(Duration, SettingLimit)> {
        match self {
            Self::Default => {
                let value = (TIMEOUT_CONNECT.default)();
                let after = Duration::from_millis(
                    value
                        .as_u64()
                        .expect("invariant: `timeout.connect`'s default is whole milliseconds"),
                );
                Some((
                    after,
                    SettingLimit {
                        key: TIMEOUT_CONNECT.name.to_owned(),
                        value,
                        layer: SettingsLayer::Default,
                        origin: None,
                    },
                ))
            }
            Self::Off => None,
            Self::After(after) => Some((after, argument(after))),
        }
    }
}

/// The `deadline_exceeded` of an opening whose `hello` was not answered
/// within `after`, set by `limit`.
pub(crate) fn exceeded(after: Duration, limit: SettingLimit) -> Error {
    let message = format!(
        "`{}` ({} ms) passed before the engine answered `hello`; raise it, or find why the engine does not answer.",
        limit.key, limit.value,
    );
    let details = DeadlineExceededDetails {
        limit,
        elapsed: wire_millis(after),
        waited_for: WaitedFor::HELLO,
        ended_by: None,
        observed: None,
    };
    failed(&DEADLINE_EXCEEDED, message, &details)
}

/// `timeout.connect` set to `after` at the run layer, given as an
/// argument.
fn argument(after: Duration) -> SettingLimit {
    SettingLimit {
        key: TIMEOUT_CONNECT.name.to_owned(),
        value: Value::from(wire_millis(after)),
        layer: SettingsLayer::Run,
        origin: Some(RunOrigin::Argument),
    }
}

/// The failure of `code` with `message` and its `details`, as the engine
/// would answer it.
fn failed(code: &ErrorCodeDefinition, message: String, details: &impl Serialize) -> Error {
    let details: Map<String, Value> = serde_json::to_value(details)
        .and_then(serde_json::from_value)
        .expect("invariant: an Error code's details serialize as an object");
    Error::Failed(Box::new(ErrorObject {
        code: code.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: code.name.to_owned(),
            retryable: code.retryable,
            details,
        },
    }))
}
