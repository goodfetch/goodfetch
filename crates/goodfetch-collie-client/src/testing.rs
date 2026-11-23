//! The testing kit's shared part (ADRs 0069, 0095 and 0105), which each
//! Product's kit re-exports: [`TestEngineOptions`], what a test engine
//! starts with, and [`TestEngine`], what every Product's test engine is.
//! A Product's kit starts the engine, linked in the test's process, in a
//! temporary Goodfetch home with a cleared environment and no project file
//! unless the test names one.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;

use crate::error::Error;
use crate::settings::Settings;

/// What a test engine starts with: the settings of its connection, the
/// environment map it reads the caller's browsers and engine from, and the
/// project file it reads.
#[derive(Clone, Debug, Default)]
pub struct TestEngineOptions {
    settings: Settings,
    env: Option<BTreeMap<OsString, OsString>>,
    project: Option<PathBuf>,
}

impl TestEngineOptions {
    /// No settings of the test's, the process's environment, and no
    /// project file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The connection's settings, in place of any given before.
    #[must_use]
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// The environment map the kit reads `GOODFETCH_HOME`,
    /// `GOODFETCH_BROWSERS_PATH` and its Product's engine-path variable
    /// from, and no other variable, in place of the process's own; the
    /// engine's own environment stays cleared.
    #[must_use]
    pub fn env(mut self, env: BTreeMap<OsString, OsString>) -> Self {
        self.env = Some(env);
        self
    }

    /// Reads the project file `path` (ADR 0083), a relative one from the
    /// working directory, in place of none.
    #[must_use]
    pub fn project(mut self, path: impl Into<PathBuf>) -> Self {
        self.project = Some(path.into());
        self
    }

    /// What a Product's kit starts its engine from: the environment map
    /// given, if any, the project file named, if any, and the connection's
    /// settings.
    #[doc(hidden)]
    #[must_use]
    pub fn into_parts(self) -> TestEngineParts {
        TestEngineParts {
            env: self.env,
            project: self.project,
            settings: self.settings,
        }
    }
}

/// What [`TestEngineOptions::into_parts`] answers a Product's kit.
#[doc(hidden)]
#[derive(Debug)]
pub struct TestEngineParts {
    /// The environment map the test gave, if any.
    pub env: Option<BTreeMap<OsString, OsString>>,
    /// The project file the test named, if any.
    pub project: Option<PathBuf>,
    /// The connection's settings.
    pub settings: Settings,
}

/// A test engine of a Product, whose connection `C` is the Product's own
/// connection type: closing it closes the connection, then removes its
/// temporary Goodfetch home, which dropping it removes too.
pub trait TestEngine<C> {
    /// The Product's connection to the engine, on which the code under
    /// test runs unchanged.
    fn connection(&self) -> &C;

    /// Closes the connection, then removes the temporary Goodfetch home.
    ///
    /// # Errors
    ///
    /// What closing the connection fails with; the home is removed even
    /// then.
    fn close(self) -> impl Future<Output = Result<(), Error>> + Send
    where
        Self: Sized;
}
