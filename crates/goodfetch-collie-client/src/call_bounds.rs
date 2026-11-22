//! The Client's own bounds on what one Connection holds (ADR 0038's note
//! of 2026-11-21; issue 3189): `connection.max_pending_calls`, the calls
//! waiting for their answer, sent or queued under the engine's cap on
//! calls in flight, and `connection.max_queued_write_bytes`, the bytes
//! waiting to be written to the engine. Both are read from the
//! Connection's own settings, those `hello` carries, before the engine is
//! reached; past either, a call fails at once, unsent, with
//! `connection_overloaded`.

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::connection::{
    CONNECTION_MAX_PENDING_CALLS, CONNECTION_MAX_QUEUED_WRITE_BYTES, CONNECTION_OVERLOADED,
    ConnectionOverloadedDetails, OverloadedBound,
};
use goodfetch_protocol::settings::{SettingInvalidDetails, SettingsLayer};
use serde_json::{Map, Value};

use crate::error::Error;

/// A Connection's bounds, each given in its own settings or its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallBounds {
    /// The most calls it holds waiting for their answer.
    max_pending_calls: usize,
    /// The most bytes it holds waiting to be written, past which only a
    /// call made while nothing waits is taken.
    max_queued_write_bytes: usize,
}

impl CallBounds {
    /// The bounds `settings`, the Connection's own as `hello` carries
    /// them nested by key, set, each default where left out.
    ///
    /// # Errors
    ///
    /// `setting_invalid` at the run layer for a bound that is no whole
    /// number of 1 or more: such a bound would refuse every call.
    pub(crate) fn of(settings: &Map<String, Value>) -> Result<Self, Error> {
        let connection = settings.get("connection");
        let bound = |setting: &Setting| {
            let name = setting
                .name
                .strip_prefix("connection.")
                .expect("invariant: a Connection's bound is a `connection.` setting");
            whole(
                setting,
                connection.and_then(|connection| connection.get(name)),
            )
        };
        Ok(Self {
            max_pending_calls: bound(&CONNECTION_MAX_PENDING_CALLS)?,
            max_queued_write_bytes: bound(&CONNECTION_MAX_QUEUED_WRITE_BYTES)?,
        })
    }

    /// The `connection_overloaded` a new call is refused with when the
    /// Connection already holds `pending` calls waiting for their answer.
    pub(crate) fn refuses_pending(self, pending: usize) -> Option<Error> {
        (pending >= self.max_pending_calls).then(|| {
            overloaded(
                &format!(
                    "the Connection already holds {pending} calls waiting for their answer, sent or queued, its bound, so this call was not sent"
                ),
                &format!(
                    "Wait for earlier calls to end, or raise `{}` in the connection's settings.",
                    CONNECTION_MAX_PENDING_CALLS.name
                ),
                OverloadedBound::PendingCalls,
                self.max_pending_calls,
            )
        })
    }

    /// The `connection_overloaded` a new call of `length` bytes is refused
    /// with when the Connection already holds `waiting` bytes not yet
    /// written; one is always taken while nothing waits.
    pub(crate) fn refuses_bytes(self, waiting: usize, length: usize) -> Option<Error> {
        (waiting > 0 && waiting.saturating_add(length) > self.max_queued_write_bytes).then(|| {
            overloaded(
                &format!(
                    "the Connection already holds {waiting} bytes waiting to be written to the engine, and this call's {length} would pass its bound of {}, so it was not sent",
                    self.max_queued_write_bytes
                ),
                &format!(
                    "Wait for the engine to read earlier calls, or raise `{}` in the connection's settings.",
                    CONNECTION_MAX_QUEUED_WRITE_BYTES.name
                ),
                OverloadedBound::QueuedWriteBytes,
                self.max_queued_write_bytes,
            )
        })
    }
}

/// `setting`'s value, `given` or its default when left out.
fn whole(setting: &Setting, given: Option<&Value>) -> Result<usize, Error> {
    let value = given.cloned().unwrap_or_else(|| (setting.default)());
    let Some(whole) = value.as_u64().filter(|value| *value >= 1) else {
        let reason = format!("{value} is no whole number of 1 or more");
        let default = (setting.default)();
        let fix = format!(
            "set `{}` to a whole number of 1 or more, or leave it out for its default of {default}",
            setting.name
        );
        return Err(Error::setting_invalid(
            format!("`{}` is not valid: {reason}; {fix}.", setting.name),
            &SettingInvalidDetails {
                key: setting.name.to_owned(),
                layer: Some(SettingsLayer::Run),
                reason,
                limit: None,
                fix: Some(fix),
            },
        ));
    };
    // A bound past what the machine addresses bounds nothing.
    Ok(usize::try_from(whole).unwrap_or(usize::MAX))
}

/// The `connection_overloaded` of the bound `bound` at `value`.
fn overloaded(what_failed: &str, fix: &str, bound: OverloadedBound, value: usize) -> Error {
    Error::refused_by_client(
        &CONNECTION_OVERLOADED,
        what_failed,
        fix,
        &ConnectionOverloadedDetails {
            bound,
            value: u64::try_from(value).unwrap_or(u64::MAX),
        },
    )
}
