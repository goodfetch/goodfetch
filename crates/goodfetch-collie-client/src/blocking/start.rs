//! A Run's starts for a program that does not use async (ADR 0081): the
//! twins of [`crate::Start`] and [`crate::HandlerlessStart`], whose start
//! ends in `send()`.

use std::future::IntoFuture;
use std::time::Duration;

use goodfetch_protocol::run::{Recording, RunId, Tags};

use super::Run;
use crate::connection::Core;
use crate::error::Error;

/// The start of a Run (`run.start`). Nothing is sent until
/// [`Start::send`].
#[must_use = "a start is sent only by send()"]
#[derive(Debug)]
pub struct Start(pub(super) crate::Start, pub(super) Core);

impl Start {
    /// Names the Run for good: a start with an existing Run ID resumes the
    /// Run if it was interrupted, or returns it as it ended (ADR 0102), as
    /// [`crate::Start::run_id`] says.
    pub fn run_id(self, run_id: &RunId) -> Self {
        Self(self.0.run_id(run_id), self.1)
    }

    /// Sets the Run's own Limits, as [`crate::Start::limits`] does.
    pub fn limits(self, limits: crate::Limits) -> Self {
        Self(self.0.limits(limits), self.1)
    }

    /// Tags the Run, as [`crate::Start::tags`] does.
    pub fn tags(self, tags: Tags) -> Self {
        Self(self.0.tags(tags), self.1)
    }

    /// Sets the Run's own settings, as [`crate::Start::settings`] does.
    pub fn settings(self, settings: crate::settings::Settings) -> Self {
        Self(self.0.settings(settings), self.1)
    }

    /// Bounds how long the Run may last, as [`crate::Start::timeout`]
    /// does.
    ///
    /// ```no_run
    /// # fn start(connection: &goodfetch_collie_client::blocking::ClientConnection) -> Result<(), goodfetch_collie_client::Error> {
    /// let run = connection.start().timeout(std::time::Duration::from_secs(600)).send()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn timeout(self, timeout: Duration) -> Self {
        Self(self.0.timeout(timeout), self.1)
    }

    /// Lets the Run last as long as it takes, as
    /// [`crate::Start::no_timeout`] does.
    ///
    /// ```no_run
    /// # fn start(connection: &goodfetch_collie_client::blocking::ClientConnection) -> Result<(), goodfetch_collie_client::Error> {
    /// let run = connection.start().no_timeout().send()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn no_timeout(self) -> Self {
        Self(self.0.no_timeout(), self.1)
    }

    /// Names the Plan file the Run follows, as [`crate::Start::plan`]
    /// does.
    ///
    /// ```no_run
    /// # fn start(connection: &goodfetch_collie_client::blocking::ClientConnection) -> Result<(), goodfetch_collie_client::Error> {
    /// let run = connection.start().plan("site.plan.json").send()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn plan(self, path: impl Into<String>) -> Self {
        Self(self.0.plan(path), self.1)
    }

    /// Sets the Response recording the Run replays or records, as
    /// [`crate::Start::recording`] does.
    ///
    /// ```no_run
    /// # fn start(connection: &goodfetch_collie_client::blocking::ClientConnection) -> Result<(), goodfetch_collie_client::Error> {
    /// use goodfetch_protocol::run::{Recording, RecordingMode};
    ///
    /// let recording = Recording { mode: RecordingMode::Replay, path: "site.warc".to_owned() };
    /// let run = connection.start().recording(recording).send()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn recording(self, recording: Recording) -> Self {
        Self(self.0.recording(recording), self.1)
    }

    /// Accepts a change of the Plugin `name`, as
    /// [`crate::Start::accept_plugin_change`] does.
    ///
    /// ```no_run
    /// # fn start(connection: &goodfetch_collie_client::blocking::ClientConnection) -> Result<(), goodfetch_collie_client::Error> {
    /// let run = connection.start().accept_plugin_change("shop-login").send()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn accept_plugin_change(self, name: impl Into<String>) -> Self {
        Self(self.0.accept_plugin_change(name), self.1)
    }

    /// Starts the Run and blocks the calling thread until it is started.
    ///
    /// # Errors
    ///
    /// What `run.start` fails with, and `setting_invalid` for a zero
    /// `timeout`, before anything is sent.
    pub fn send(self) -> Result<Run, Error> {
        let Self(start, connection) = self;
        let inner = connection.wait(start.into_future())?;
        Ok(Run { inner })
    }
}

/// The start of a Run with no handler in the program, a Recipe's run or a
/// code-free catalog run. Nothing is sent until
/// [`HandlerlessStart::send`].
#[must_use = "a start is sent only by send()"]
#[derive(Debug)]
pub struct HandlerlessStart(pub(super) crate::HandlerlessStart, pub(super) Core);

impl HandlerlessStart {
    /// Names the Run for good, as [`crate::HandlerlessStart::run_id`] does.
    pub fn run_id(self, run_id: &RunId) -> Self {
        Self(self.0.run_id(run_id), self.1)
    }

    /// Tags the Run, as [`crate::HandlerlessStart::tags`] does.
    pub fn tags(self, tags: Tags) -> Self {
        Self(self.0.tags(tags), self.1)
    }

    /// Sets the Run's own Limits, as [`crate::HandlerlessStart::limits`]
    /// does.
    pub fn limits(self, limits: crate::Limits) -> Self {
        Self(self.0.limits(limits), self.1)
    }

    /// Sets the Run's own settings, as
    /// [`crate::HandlerlessStart::settings`] does.
    pub fn settings(self, settings: crate::settings::Settings) -> Self {
        Self(self.0.settings(settings), self.1)
    }

    /// Sets the Response recording the Run replays or records, as
    /// [`crate::HandlerlessStart::recording`] does.
    pub fn recording(self, recording: Recording) -> Self {
        Self(self.0.recording(recording), self.1)
    }

    /// Bounds how long the Run may last, as
    /// [`crate::HandlerlessStart::timeout`] does.
    pub fn timeout(self, timeout: Duration) -> Self {
        Self(self.0.timeout(timeout), self.1)
    }

    /// Lets the Run last as long as it takes, as
    /// [`crate::HandlerlessStart::no_timeout`] does.
    pub fn no_timeout(self) -> Self {
        Self(self.0.no_timeout(), self.1)
    }

    /// Lets the Run outlive the program that started it, as
    /// [`crate::HandlerlessStart::detached`] does.
    pub fn detached(self) -> Self {
        Self(self.0.detached(), self.1)
    }

    /// Starts the Run and blocks the calling thread until it is started.
    ///
    /// # Errors
    ///
    /// What its opener fails with, and `setting_invalid` for a zero
    /// `timeout`, before anything is sent.
    pub fn send(self) -> Result<Run, Error> {
        let Self(start, connection) = self;
        let inner = connection.wait(start.into_future())?;
        Ok(Run { inner })
    }
}
