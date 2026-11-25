//! The major's Daemon as an Attached engine reaches it for the calls the
//! Daemon keeps, such as `schedule.*` (spec 589): the running Daemon, or
//! the pick this executable starts when none runs, as a relay reaches it
//! (ADR 0002).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::{Resolved, SettingsError};
use goodfetch_engine::{DaemonReach, Identity};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::jsonrpc::ErrorObject;

use super::Machine;

/// What [`Machine`] holds, owned, for an engine that outlives `main`'s
/// borrows.
pub(in crate::executable) struct DaemonOfTheMajor {
    /// The executable's Product, version and channel.
    pub(in crate::executable) identity: Identity,
    /// Goodfetch home.
    pub(in crate::executable) home: Home,
    /// The environment `main` read once, which a started Daemon gets.
    pub(in crate::executable) environment: BTreeMap<OsString, OsString>,
    /// This executable's path, as the OS started it.
    pub(in crate::executable) executable: Option<PathBuf>,
    /// The engine's Clock.
    pub(in crate::executable) clock: Clock,
    /// The machine's settings, which `main` read once, or why they cannot
    /// be read.
    pub(in crate::executable) settings: Result<Resolved, SettingsError>,
}

impl DaemonReach for DaemonOfTheMajor {
    fn running_or_started(&self) -> Result<(DaemonEndpoint, String), ErrorObject> {
        let machine = Machine {
            identity: &self.identity,
            home: &self.home,
            environment: &self.environment,
            executable: self.executable.clone(),
            clock: self.clock,
            settings: &self.settings,
        };
        machine
            .reach_for_relay()
            .map(|(endpoint, token)| (endpoint, token.expose().to_owned()))
            .map_err(super::Failure::into_error)
    }
}
