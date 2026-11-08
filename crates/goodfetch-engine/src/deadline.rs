//! The deadlines a call waits under (ADRs 0059, 0080 and 0083): each phase's
//! timeout is a duration setting, such as `timeout.first_byte` or
//! `timeout.navigation`, and a call past it ends with `deadline_exceeded`,
//! one Error code whatever the phase, whose `limit` names the setting, its
//! value and the layer that set it, so its fix is to raise that setting.
//! The fetch tiers and the page's driver (area 5) wait under it; time is the
//! `Clock`'s.

use std::time::Duration;

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails, WaitedFor};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::settings::Resolved;

/// A timeout's deadline, as its setting resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Deadline {
    limit: SettingLimit,
    duration: Duration,
}

impl Deadline {
    /// The deadline `settings` give the timeout `setting`; `None` when it is
    /// unset or no duration.
    pub fn of(settings: &Resolved, setting: &Setting) -> Option<Self> {
        let effective = settings.effective(setting)?;
        let duration = Duration::from_millis(effective.value.as_u64()?);
        Some(Self {
            limit: SettingLimit {
                key: setting.name.to_owned(),
                value: effective.value.clone(),
                layer: effective.layer,
            },
            duration,
        })
    }

    /// The deadline of `millis` the timeout `setting` takes from `layer`:
    /// the run layer for a call's own `timeout`, the default when nothing
    /// resolved it.
    pub fn given(setting: &Setting, millis: u64, layer: SettingsLayer) -> Self {
        Self {
            limit: SettingLimit {
                key: setting.name.to_owned(),
                value: millis.into(),
                layer,
            },
            duration: Duration::from_millis(millis),
        }
    }

    /// The deadline the timeout `setting` declares as its default, set by
    /// the default layer: a Run whose settings resolve none waits under it,
    /// so it follows the catalog when the default changes.
    ///
    /// # Panics
    ///
    /// When `setting` declares no default in milliseconds, which no timeout
    /// setting does.
    pub fn declared(setting: &Setting) -> Self {
        let millis = (setting.default)()
            .as_u64()
            .expect("invariant: a timeout setting declares its default in milliseconds");
        Self::given(setting, millis, SettingsLayer::Default)
    }

    /// The deadline a Run whose settings resolved as `settings` waits
    /// under for the timeout `setting`: as they resolve it, else the
    /// setting's declared default, as for a Run whose settings resolved
    /// none.
    pub fn of_resolved(settings: Option<&Resolved>, setting: &Setting) -> Self {
        settings
            .and_then(|settings| Self::of(settings, setting))
            .unwrap_or_else(|| Self::declared(setting))
    }

    /// The deadline of a call whose own `timeout` param is `timeout`, the
    /// timeout `setting` for that call alone (ADRs 0080 and 0083): `scope`,
    /// the deadline its scope's settings give, when absent; none when
    /// `null`; else the call's, which is that setting at the run layer.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a `timeout` of 0, which would end the call at
    /// once.
    pub fn of_call(
        scope: &Self,
        setting: &Setting,
        timeout: Option<Option<u64>>,
    ) -> Result<Option<Self>, CallError> {
        match timeout {
            None => Ok(Some(scope.clone())),
            Some(None) => Ok(None),
            Some(Some(0)) => Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "the call's timeout, its `{}`, is 0 ms, which would end it at once: give at least 1, or null for none",
                    setting.name
                ),
            )),
            Some(Some(millis)) => Ok(Some(Self::given(setting, millis, SettingsLayer::Run))),
        }
    }

    /// How long it allows.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// The setting that set it, its value and its layer, which an error
    /// at the deadline names.
    pub fn limit(&self) -> &SettingLimit {
        &self.limit
    }

    /// Waits for `work` on `clock` until the deadline passes, then ends it
    /// with `deadline_exceeded`, which says it waited for `waited_for`.
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` once the deadline passed first.
    ///
    /// # Cancel safety
    ///
    /// As cancel-safe as `work`.
    pub async fn within<T>(
        &self,
        clock: &Clock,
        waited_for: WaitedFor,
        work: impl Future<Output = T>,
    ) -> Result<T, CallError> {
        let started = clock.now();
        tokio::select! {
            biased;
            answer = work => Ok(answer),
            () = clock.sleep_until(started + self.duration) => {
                Err(self.exceeded(waited_for, clock.now() - started))
            }
        }
    }

    /// The `deadline_exceeded` of a call that waited `elapsed` under this
    /// deadline for `waited_for`, which did not come, for a caller that
    /// counts its own time, such as one that leaves some waits out of it.
    pub fn exceeded(&self, waited_for: WaitedFor, elapsed: Duration) -> CallError {
        CallError::new(
            &DEADLINE_EXCEEDED,
            format!(
                "`{}` ({} ms) passed before the {waited_for} came; raise it, or wait for the right thing.",
                self.limit.key, self.limit.value,
            ),
            &DeadlineExceededDetails {
                limit: self.limit.clone(),
                elapsed: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                waited_for,
            },
        )
    }
}

#[cfg(test)]
mod tests;
