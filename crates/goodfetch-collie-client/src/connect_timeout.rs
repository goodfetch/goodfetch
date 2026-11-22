//! How long opening a Connection waits for `hello`'s answer (ADRs 0080 and
//! 0083): `timeout.connect`, 30 s or as the project file sets it, or the
//! connection's own `timeout`, in its three states. Past it, the opening fails with `deadline_exceeded`,
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
/// (ADR 0080): `timeout.connect` when left as it is, its default or as the
/// project file sets it, no deadline, or a duration of the connection's
/// own, given as an argument.
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
    /// At most this long: `timeout.connect` as the project file sets it,
    /// the one layer that sets it (ADR 0083).
    Project(Duration),
}

impl ConnectTimeout {
    /// Refuses a deadline that would end the opening at once, before
    /// anything starts.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a zero duration, its
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

    /// It, once the project file's `timeout.connect`, `project`, is
    /// taken: that value in place of the default, while a connection's own
    /// `timeout` and no timeout win over it.
    ///
    /// # Errors
    ///
    /// `setting_invalid` at `project_configuration` for a value that is no
    /// whole number of 1 ms or more, whichever state it is in, as the
    /// engine would refuse that value with the rest of the file.
    pub fn with_project(self, project: Option<&Value>) -> Result<Self, Error> {
        let Some(value) = project else {
            return Ok(self);
        };
        let after = match value.as_u64() {
            Some(0) => Err("a timeout of 0 ms would end the opening at once"),
            Some(millis) => Ok(Duration::from_millis(millis)),
            None => Err("it is no whole number of milliseconds"),
        };
        match (after, self) {
            (Ok(after), Self::Default) => Ok(Self::Project(after)),
            (Ok(_), kept) => Ok(kept),
            (Err(reason), _) => Err(project_refused(value, reason)),
        }
    }

    /// What is left of it once `elapsed` of an opening has passed, which a
    /// later step of the same opening waits within, such as a second
    /// engine's `hello` (ADR 0098).
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` when nothing is left, its `limit` naming
    /// `timeout.connect` and the layer that set it, as
    /// [`ConnectTimeout::exceeded`] gives it.
    pub fn rest(self, elapsed: Duration) -> Result<Self, Error> {
        let Some((after, limit)) = self.deadline() else {
            return Ok(self);
        };
        match after.checked_sub(elapsed) {
            Some(rest) if !rest.is_zero() => Ok(Self::After(rest)),
            _ => Err(exceeded(after, limit)),
        }
    }

    /// The `deadline_exceeded` an opening fails with once it has passed:
    /// its `limit` names `timeout.connect` and the layer that set it, and
    /// its `waited_for` `hello`. `None` for [`ConnectTimeout::Off`], which
    /// never passes.
    #[must_use]
    pub fn exceeded(self) -> Option<Error> {
        self.deadline().map(|(after, limit)| exceeded(after, limit))
    }

    /// [`ConnectTimeout::exceeded`] for a step of an opening that passed
    /// it waiting for `awaited` rather than for the engine's answer, such
    /// as the Daemon an idle one hands over to (ADR 0098): its message
    /// names what it waited for, and its `fix` is `fix`.
    #[must_use]
    pub fn exceeded_awaiting(self, awaited: &str, fix: impl Into<String>) -> Option<Error> {
        self.deadline().map(|(after, limit)| {
            let message = format!(
                "`{}` ({} ms) passed before {awaited}.",
                limit.key, limit.value
            );
            deadline_exceeded(after, limit, message).with_fix(fix)
        })
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
            Self::Project(after) => Some((
                after,
                SettingLimit {
                    key: TIMEOUT_CONNECT.name.to_owned(),
                    value: Value::from(wire_millis(after)),
                    layer: SettingsLayer::ProjectConfiguration,
                    origin: None,
                },
            )),
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
    deadline_exceeded(after, limit, message)
}

/// The `deadline_exceeded` with `message` of an opening that waited
/// `after`, set by `limit`, for `hello`'s answer.
fn deadline_exceeded(after: Duration, limit: SettingLimit, message: String) -> Error {
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

/// The `setting_invalid` of the project file's `timeout.connect` set to
/// `value`, which is wrong for `reason`.
fn project_refused(value: &Value, reason: &str) -> Error {
    let key = TIMEOUT_CONNECT.name;
    let fix = format!("set `{key}` in the project file to at least 1 ms, or remove it for 30 s");
    let details = SettingInvalidDetails {
        key: key.to_owned(),
        layer: Some(SettingsLayer::ProjectConfiguration),
        reason: reason.to_owned(),
        limit: Some(SettingLimit {
            key: key.to_owned(),
            value: value.clone(),
            layer: SettingsLayer::ProjectConfiguration,
            origin: None,
        }),
        fix: Some(fix.clone()),
    };
    failed(
        &SETTING_INVALID,
        format!("The project file's `{key}`, {value}: {reason}; {fix}."),
        &details,
    )
}

/// The failure of `code` with `message` and its `details`, as the engine
/// would answer it.
fn failed(code: &ErrorCodeDefinition, message: String, details: &impl Serialize) -> Error {
    let details: Map<String, Value> = serde_json::to_value(details)
        .and_then(serde_json::from_value)
        .expect("invariant: an Error code's details serialize as an object");
    Error::answered(ErrorObject {
        code: code.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: code.name.to_owned(),
            retryable: code.retryable,
            details,
        },
    })
}
