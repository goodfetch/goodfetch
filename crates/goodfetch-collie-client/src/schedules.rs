//! A Connection's Schedules (spec 589, ADR 0099): `connection.schedules()`,
//! reached without I/O, whose calls act on no one Schedule, and the
//! [`Schedule`] handle `create` and `get` answer, whose members the catalog
//! generates. A Schedule handle holds its ID and has no `close()`: it names
//! a Schedule its store keeps, so dropping it frees nothing in the engine.
//! TypeScript's `schedules.ts` and Python's `schedules.py` are its twins.

use goodfetch_protocol::schedule::{
    SCHEDULE_CREATE, SCHEDULE_GET, SCHEDULE_LIST, ScheduleCreateParams, ScheduleCreated,
    ScheduleDefinition, ScheduleGetParams, ScheduleId, ScheduleInfo, ScheduleList,
    ScheduleListParams, ScheduleWarning,
};

use crate::call::Call;
use crate::connection::ClientConnection;
use crate::generated::handles::Schedule;
use crate::host::ConnectionHost;

/// A Connection's Schedules: `connection.schedules()`. Each call acts on
/// the Schedule store of the engine's major, and none on one Schedule: a
/// [`Schedule`]'s own members do. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct Schedules {
    connection: ClientConnection,
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

impl ClientConnection {
    /// The Connection's Schedules, reached without I/O (spec 589).
    #[must_use]
    pub fn schedules(&self) -> Schedules {
        Schedules {
            connection: self.clone(),
        }
    }
}

impl Schedules {
    /// Records a Schedule in the Schedule store, which the Daemon keeps, and answers its ID, as given or made (`s` and a lowercase ULID), with warnings. Nothing fires yet. A taken ID fails with `schedule_exists`, a trigger that does not parse with `schedule_trigger_invalid`. Its time zone is the Daemon host's when the definition names none. One store transaction: a cancel stops it only before it commits.
    ///
    /// Answers the [`Schedule`] under that ID, which holds it.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// Once sent, `closed` when the Connection is closed, the engine's
    /// error, `deadline_exceeded` once its timeout runs out, and
    /// `internal` when its reply does not fit its type.
    pub fn create(
        &self,
        definition: ScheduleDefinition,
    ) -> Call<CreatedSchedule, ScheduleCreateParams> {
        crate::markers::warn_operation(SCHEDULE_CREATE.name);
        let connection = self.connection.clone();
        let call: Call<ScheduleCreated, ScheduleCreateParams> = self.host().call(
            SCHEDULE_CREATE.name,
            ScheduleCreateParams {
                definition,
                timeout: None,
            },
        );
        call.wire_timeout("timeout.wait")
            .map(move |created| CreatedSchedule {
                schedule: schedule(connection, created.schedule_id),
                warnings: created.warnings,
            })
    }

    /// Lists every Schedule of the Schedule store, by ID.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// Once sent, `closed` when the Connection is closed, the engine's
    /// error, `deadline_exceeded` once its timeout runs out, and
    /// `internal` when its reply does not fit its type.
    pub fn list(&self) -> Call<Vec<ScheduleInfo>, ScheduleListParams> {
        crate::markers::warn_operation(SCHEDULE_LIST.name);
        let call: Call<ScheduleList, ScheduleListParams> = self
            .host()
            .call(SCHEDULE_LIST.name, ScheduleListParams::default());
        call.wire_timeout("timeout.wait")
            .map(|listed| listed.schedules)
    }

    /// Reads the Schedule of an ID. An ID the store does not hold fails with `schedule_not_found`.
    ///
    /// Answers the [`Schedule`] of that ID once the store said it holds
    /// one; its `info()` reads it again.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// Once sent, `closed` when the Connection is closed, the engine's
    /// error, `deadline_exceeded` once its timeout runs out, and
    /// `internal` when its reply does not fit its type.
    pub fn get(&self, id: &ScheduleId) -> Call<Schedule, ScheduleGetParams> {
        crate::markers::warn_operation(SCHEDULE_GET.name);
        let connection = self.connection.clone();
        let call: Call<ScheduleInfo, ScheduleGetParams> = self.host().call(
            SCHEDULE_GET.name,
            ScheduleGetParams {
                id: id.clone(),
                timeout: None,
            },
        );
        call.wire_timeout("timeout.wait")
            .map(move |info| schedule(connection, info.schedule_id))
    }

    /// The host the Schedules' own calls go through, which names no
    /// Schedule.
    fn host(&self) -> ConnectionHost<()> {
        ConnectionHost::new(self.connection.clone(), ())
    }
}

/// The handle of the Schedule `id` names, whose calls go through
/// `connection`.
fn schedule(connection: ClientConnection, id: ScheduleId) -> Schedule {
    Schedule {
        host: ConnectionHost::new(connection, id),
    }
}

/// C-SEND-SYNC: the Schedules and their handles move between threads, are
/// shared between them and outlive the borrow that made them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Schedules>();
    send_sync_static::<Schedule>();
    send_sync_static::<CreatedSchedule>();
};
