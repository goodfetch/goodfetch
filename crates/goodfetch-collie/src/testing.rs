//! Collie's testing kit (ADRs 0069, 0090 and 0105), behind the `test-util`
//! feature, which builds test values alone (ADR 0087): [`TestEngine`], a
//! real engine linked in the test's process, whose connection is a
//! [`Collie`], with the client crate's [`TestEngineOptions`].
//!
//! ```no_run
//! use goodfetch_collie::testing::{TestEngine, TestEngineOptions};
//!
//! # async fn test() -> Result<(), goodfetch_collie::Error> {
//! let engine = TestEngine::start(TestEngineOptions::new()).await?;
//! engine.connection().start().await?.finish().await?;
//! engine.close().await?;
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use goodfetch_collie_client::Error;
pub use goodfetch_collie_client::testing::TestEngineOptions;
use goodfetch_collie_client::testing::TestEngineParts;
use goodfetch_engine::browser_cache::CacheLocation;
use goodfetch_engine::settings::deployment_variable;
use goodfetch_protocol::settings::{
    BROWSERS_PATH_VARIABLE, COLLIE_ENGINE_PATH_VARIABLE, HOME_VARIABLE,
};
use tempfile::TempDir;

use crate::collie::Collie;
use crate::engine_choice::EngineChoice;
use crate::linked::{self, ConnectOptions};

/// A real Collie engine for a test, linked in its process: in a temporary
/// Goodfetch home, removed when it closes or drops, with a cleared
/// environment, `egress.offline` on, short durations and no project file
/// unless the test names one. Browsers come from the caller's Browser
/// cache, `GOODFETCH_BROWSERS_PATH`'s or the caller's Goodfetch home's,
/// which a missing browser leaves as it was: `egress.offline` fails its
/// launch with `artifact_unavailable_offline`, whose `fix` names
/// `goodfetch-collie browser install`.
#[derive(Debug)]
pub struct TestEngine {
    connection: Collie,
    home: TempDir,
}

impl TestEngine {
    /// Starts a test engine with `options`.
    ///
    /// # Errors
    ///
    /// `engine_exited` when the temporary Goodfetch home cannot be made,
    /// and what [`Collie::connect`] fails with, such as `setting_invalid`
    /// for a project file that is not there.
    pub async fn start(options: TestEngineOptions) -> Result<Self, Error> {
        let home = tempfile::Builder::new()
            .prefix("goodfetch-test-engine-")
            .tempdir()
            .map_err(Error::engine_failed)?;
        let connection = Collie::connect(connect_options(
            options.into_parts(),
            home.path(),
            COLLIE_ENGINE_PATH_VARIABLE,
        ))
        .await?;
        Ok(Self { connection, home })
    }

    /// The connection to the engine, on which the code under test runs
    /// unchanged.
    #[must_use]
    pub fn connection(&self) -> &Collie {
        &self.connection
    }

    /// Closes the connection, then removes the temporary Goodfetch home.
    ///
    /// # Errors
    ///
    /// What [`Collie::close`] fails with; the home is removed even then.
    pub async fn close(self) -> Result<(), Error> {
        let closed = self.connection.close().await;
        drop(self.home);
        closed
    }
}

impl goodfetch_collie_client::testing::TestEngine<Collie> for TestEngine {
    fn connection(&self) -> &Collie {
        Self::connection(self)
    }

    async fn close(self) -> Result<(), Error> {
        Self::close(self).await
    }
}

/// What a Product's test engine connects with: a linked engine in `home`,
/// its environment cleared but for that home, the caller's Browser cache
/// and `engine_path_variable`, read from the test's map or else the
/// process's, the settings `parts` hold and its project file, or none.
fn connect_options(
    parts: TestEngineParts,
    home: &Path,
    engine_path_variable: &str,
) -> ConnectOptions {
    let caller = parts.env.unwrap_or_else(process_environment);
    let options = ConnectOptions::default()
        .engine(EngineChoice::Linked)
        .environment(engine_environment(&caller, home, engine_path_variable))
        .settings(parts.settings);
    match parts.project {
        Some(project) => options.project(project),
        None => options.no_project(),
    }
}

/// The process's environment, read once, as `connect()` reads it when no
/// map is given.
#[expect(
    clippy::disallowed_methods,
    reason = "the testing kit, given no environment map, reads the process environment once, as the SDK's connect() does (ADR 0083)"
)]
fn process_environment() -> BTreeMap<OsString, OsString> {
    std::env::vars_os().collect()
}

/// The engine's environment: `GOODFETCH_HOME` naming `home`,
/// `GOODFETCH_BROWSERS_PATH` naming the Browser cache of `caller`, its own
/// variable's or its Goodfetch home's, and `engine_path_variable` as
/// `caller` sets it.
fn engine_environment(
    caller: &BTreeMap<OsString, OsString>,
    home: &Path,
    engine_path_variable: &str,
) -> BTreeMap<OsString, OsString> {
    let mut environment = BTreeMap::from([(HOME_VARIABLE.into(), home.into())]);
    if let Some(caller_home) = linked::home(caller) {
        let cache = CacheLocation::new(
            caller_home.root(),
            caller_home.browsers_path().map(Path::to_path_buf),
        );
        environment.insert(BROWSERS_PATH_VARIABLE.into(), cache.root().into());
    }
    if let Some(engine) = deployment_variable(caller, engine_path_variable) {
        environment.insert(engine_path_variable.into(), engine.to_owned());
    }
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    #[test]
    fn the_engine_reads_its_home_and_the_caller_s_browsers_and_engine_alone() {
        let caller = map(&[
            ("GOODFETCH_HOME", "/caller/home"),
            ("GOODFETCH_COLLIE_ENGINE_PATH", "/opt/goodfetch-collie"),
            ("GOODFETCH_URL", "ws://127.0.0.1:1"),
            ("PATH", "/usr/bin"),
        ]);

        let environment = engine_environment(
            &caller,
            Path::new("/tmp/test-home"),
            COLLIE_ENGINE_PATH_VARIABLE,
        );

        assert_eq!(
            environment,
            map(&[
                ("GOODFETCH_BROWSERS_PATH", "/caller/home/browsers"),
                ("GOODFETCH_COLLIE_ENGINE_PATH", "/opt/goodfetch-collie"),
                ("GOODFETCH_HOME", "/tmp/test-home"),
            ])
        );
    }

    #[test]
    fn the_caller_s_browsers_path_takes_the_place_of_its_home_s_cache() {
        let caller = map(&[
            ("GOODFETCH_HOME", "/caller/home"),
            ("GOODFETCH_BROWSERS_PATH", "/prepared/browsers"),
        ]);

        let environment = engine_environment(
            &caller,
            Path::new("/tmp/test-home"),
            COLLIE_ENGINE_PATH_VARIABLE,
        );

        assert_eq!(
            environment,
            map(&[
                ("GOODFETCH_BROWSERS_PATH", "/prepared/browsers"),
                ("GOODFETCH_HOME", "/tmp/test-home"),
            ])
        );
    }
}
