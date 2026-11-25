//! A Connection's Schedules for a program that does not use async (ADR
//! 0081): the twin of [`crate::Schedules`], whose calls end in `send()`,
//! and the blocking face of the [`Schedule`] handle they answer.

use goodfetch_protocol::schedule::{
    ScheduleCreateParams, ScheduleDefinition, ScheduleGetParams, ScheduleId, ScheduleInfo,
    ScheduleListParams, ScheduleWarning,
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
        let call = self.0.create(definition).map(|created| {
            let (schedule, warnings) = created.into_parts();
            CreatedSchedule {
                schedule: Schedule::from(schedule),
                warnings,
            }
        });
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
}

/// What [`Schedules::create`] answers: the created Schedule, and what its
/// program should know.
#[derive(Clone, Debug)]
pub struct CreatedSchedule {
    schedule: Schedule,
    warnings: Vec<ScheduleWarning>,
}

impl CreatedSchedule {
    /// The Schedule, under the ID given or made.
    #[must_use]
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// What did not stop its creation but its program should know.
    #[must_use]
    pub fn warnings(&self) -> &[ScheduleWarning] {
        &self.warnings
    }

    /// The Schedule and its warnings.
    #[must_use]
    pub fn into_parts(self) -> (Schedule, Vec<ScheduleWarning>) {
        (self.schedule, self.warnings)
    }
}
