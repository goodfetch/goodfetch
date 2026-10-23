//! The deadlines a call waits under (ADRs 0059, 0080 and 0083): each phase's
//! timeout is a duration setting, such as `timeout.first_byte` or
//! `timeout.navigation`, and a call past it ends with `deadline_exceeded`,
//! one Error code whatever the phase, whose `limit` names the setting, its
//! value and the layer that set it, so its fix is to raise that setting.
//! The fetch tiers and the page's driver (area 5) wait under it; time is the
//! `Clock`'s.

use std::time::Duration;

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails};
use goodfetch_protocol::settings::SettingLimit;

use crate::clock::Clock;
use crate::composition::CallError;
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
        let effective = settings.get(setting.name)?;
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
        waited_for: &str,
        work: impl Future<Output = T>,
    ) -> Result<T, CallError> {
        let started = clock.now();
        tokio::select! {
            biased;
            answer = work => Ok(answer),
            () = clock.sleep_until(started + self.duration) => {
                let elapsed = u64::try_from((clock.now() - started).as_millis()).unwrap_or(u64::MAX);
                Err(CallError::new(
                    &DEADLINE_EXCEEDED,
                    format!(
                        "`{}` ({} ms) passed before the {waited_for} came; raise it, or wait for the right thing.",
                        self.limit.key, self.limit.value,
                    ),
                    &DeadlineExceededDetails {
                        limit: self.limit.clone(),
                        elapsed,
                        waited_for: waited_for.to_owned(),
                    },
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
