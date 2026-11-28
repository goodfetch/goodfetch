//! A Connection's Schedules for a program that does not use async (ADR
//! 0081): the twin of [`crate::Schedules`], whose calls end in `send()`,
//! and the blocking face of the [`Schedule`] handle they answer.

use std::time::SystemTime;

use goodfetch_protocol::schedule::{
    ScheduleCreateParams, ScheduleDefinition, ScheduleGetParams, ScheduleId, ScheduleInfo,
    ScheduleListParams, SchedulePreview, SchedulePreviewParams, ScheduleTrigger,
};

use super::{Call, ClientConnection, Schedule};

impl ClientConnection {
    /// The Connection's Schedules, as
    /// [`crate::ClientConnection::schedules`]'s, reached without I/O.
    #[must_use]
    pub fn schedules(&self) -> Schedules {
        Schedules(self.0.schedules(), self.0.clone())
    }
}

/// The blocking face of `schedule`, whose calls block.
impl From<crate::Schedule> for Schedule {
    fn from(inner: crate::Schedule) -> Self {
        Self { inner }
    }
}

/// A Connection's Schedules, whose calls block: the twin of
/// [`crate::Schedules`]. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct Schedules(crate::Schedules, crate::ClientConnection);

impl Schedules {
    /// Records a Schedule in the Schedule store, which the Daemon keeps, and answers its ID, as given or made (`s` and a lowercase ULID), with warnings. Nothing fires yet. A taken ID fails with `schedule_exists`, a trigger that does not parse with `schedule_trigger_invalid`. Its time zone is the Daemon host's when the definition names none. One store transaction: a cancel stops it only before it commits.
    ///
    /// Answers the [`Schedule`] under that ID, which holds it, as
    /// [`crate::Schedules::create`] does, which `send()` sends.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn create(
        &self,
        definition: ScheduleDefinition,
    ) -> Call<CreatedSchedule, ScheduleCreateParams> {
        let call = self
            .0
            .create(definition)
            .map(|created| created.map_schedule(Schedule::from));
        Call::new(call, &self.1)
    }

    /// Lists every Schedule of the Schedule store, by ID.
    ///
    /// As [`crate::Schedules::list`] does, which `send()` sends.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn list(&self) -> Call<Vec<ScheduleInfo>, ScheduleListParams> {
        Call::new(self.0.list(), &self.1)
    }

    /// Reads the Schedule of an ID. An ID the store does not hold fails with `schedule_not_found`.
    ///
    /// Answers the [`Schedule`] of that ID, as [`crate::Schedules::get`]
    /// does, which `send()` sends.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn get(&self, id: &ScheduleId) -> Call<Schedule, ScheduleGetParams> {
        Call::new(self.0.get(id).map(Schedule::from), &self.1)
    }

    /// Describes a Schedule's trigger in plain words and lists its next
    /// Occurrences in a time zone strictly after `after` (now when absent;
    /// 5 by default, at most 100), including DST transitions. A trigger
    /// that does not parse fails with `schedule_trigger_invalid`.
    ///
    /// As [`crate::Schedules::preview`] does, which `send()` sends.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn preview(
        &self,
        trigger: ScheduleTrigger,
    ) -> Call<SchedulePreview, SchedulePreviewParams> {
        Call::new(self.0.preview(trigger), &self.1)
    }
}

crate::schedules::preview_call_options!(Call);

/// What [`Schedules::create`] answers, its Schedule's calls blocking.
pub type CreatedSchedule = crate::schedules::CreatedSchedule<Schedule>;
