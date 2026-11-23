//! The testing kit's shared part (ADRs 0069, 0095 and 0105), which each
//! Product's kit re-exports: [`TestEngineOptions`], what a test engine
//! starts with, and [`TestEngine`], what every Product's test engine is.
//! A Product's kit starts the engine, linked in the test's process, in a
//! temporary Goodfetch home with a cleared environment, `egress.offline`
//! on, short durations and no project file unless the test names one;
//! every host its `network.resolve` maps to loopback alone is the only one
//! a Run's requests reach, live.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::error::Error;
use crate::settings::{Settings, egress, network, timeout};

/// How long the test engine's calls wait where the catalog's defaults wait
/// 30 s: a page's actions, navigations, waits and first bytes, and a Run's
/// close, so a test that went wrong fails in seconds. A test's own
/// settings take the place of each.
const SHORT: Duration = Duration::from_secs(5);

/// What a test engine starts with: the settings of its connection, over
/// the kit's own, the environment map it reads the caller's browsers and
/// engine from, and the project file it reads.
#[derive(Clone, Debug, Default)]
pub struct TestEngineOptions {
    settings: Settings,
    env: Option<BTreeMap<OsString, OsString>>,
    project: Option<PathBuf>,
}

impl TestEngineOptions {
    /// The kit's isolations alone: no settings of the test's, the
    /// process's environment, and no project file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The connection's settings, in place of any given before, over the
    /// kit's: `network.resolve`'s hosts mapped to loopback alone are the
    /// hosts its Runs reach, live, and a key it gives, such as
    /// `timeout.navigation`, takes the place of the kit's.
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
    /// settings, the test's over the kit's.
    #[doc(hidden)]
    #[must_use]
    pub fn into_parts(self) -> TestEngineParts {
        TestEngineParts {
            env: self.env,
            project: self.project,
            settings: isolated(self.settings),
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
    /// The connection's settings, the test's over the kit's.
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

/// `settings` over the kit's isolations: `egress.offline` on, the hosts
/// `network.resolve` maps to loopback alone as the only hosts allowed and
/// the private hosts a page may reach, and the short durations.
fn isolated(settings: Settings) -> Settings {
    let loopback = loopback_hosts(settings.given(network::RESOLVE.name()));
    settings
        .or_set(egress::OFFLINE, true)
        .or_set(egress::ALLOWED_DOMAINS, loopback.clone())
        .or_set(egress::PRIVATE_RANGES, loopback)
        .or_set(timeout::ACTION, SHORT)
        .or_set(timeout::NAVIGATION, SHORT)
        .or_set(timeout::WAIT, SHORT)
        .or_set(timeout::FIRST_BYTE, SHORT)
        .or_set(timeout::CLOSE, SHORT)
}

/// The hosts `resolve`, `network.resolve`'s value, maps to addresses that
/// are all loopback.
fn loopback_hosts(resolve: Option<&Value>) -> Vec<String> {
    let Some(Value::Object(hosts)) = resolve else {
        return Vec::new();
    };
    hosts
        .iter()
        .filter(|(_, addresses)| {
            addresses.as_array().is_some_and(|addresses| {
                !addresses.is_empty()
                    && addresses.iter().all(|address| {
                        address
                            .as_str()
                            .and_then(|address| address.parse::<IpAddr>().ok())
                            .is_some_and(|address| address.to_canonical().is_loopback())
                    })
            })
        })
        .map(|(host, _)| host.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::settings::Settings;

    /// The connection's settings `options` gives, as `hello` carries them.
    fn wire(options: TestEngineOptions) -> Value {
        Value::Object(
            options
                .into_parts()
                .settings
                .within(goodfetch_protocol::catalog::SettingScope::Connection)
                .unwrap(),
        )
    }

    #[test]
    fn only_the_hosts_resolve_maps_to_loopback_are_allowed_and_reached() {
        let resolve = BTreeMap::from([
            ("app.test".to_owned(), vec!["127.0.0.1".to_owned()]),
            ("api.test".to_owned(), vec!["::1".to_owned()]),
            (
                "mixed.test".to_owned(),
                vec!["127.0.0.1".to_owned(), "192.0.2.1".to_owned()],
            ),
            ("public.test".to_owned(), vec!["192.0.2.7".to_owned()]),
        ]);
        let settings = Settings::new().set(network::RESOLVE, resolve);

        let wire = wire(TestEngineOptions::new().settings(settings));

        assert_eq!(wire["egress"]["offline"], json!(true));
        assert_eq!(
            wire["egress"]["allowed_domains"],
            json!(["api.test", "app.test"])
        );
        assert_eq!(
            wire["egress"]["private_ranges"],
            json!(["api.test", "app.test"])
        );
    }

    #[test]
    fn with_no_resolve_no_host_is_allowed_and_the_durations_are_short() {
        let wire = wire(TestEngineOptions::new());

        assert_eq!(wire["egress"]["allowed_domains"], json!([]));
        assert_eq!(wire["egress"]["private_ranges"], json!([]));
        for key in ["action", "navigation", "wait", "first_byte", "close"] {
            assert_eq!(wire["timeout"][key], json!(5_000), "{key}");
        }
    }

    #[test]
    fn a_test_s_own_setting_takes_the_place_of_the_kit_s() {
        let settings = Settings::new()
            .set(egress::OFFLINE, false)
            .set(egress::ALLOWED_DOMAINS, vec!["*".to_owned()])
            .set(timeout::NAVIGATION, Duration::from_secs(60));

        let wire = wire(TestEngineOptions::new().settings(settings));

        assert_eq!(wire["egress"]["offline"], json!(false));
        assert_eq!(wire["egress"]["allowed_domains"], json!(["*"]));
        assert_eq!(wire["timeout"]["navigation"], json!(60_000));
        assert_eq!(wire["timeout"]["action"], json!(5_000));
    }
}
