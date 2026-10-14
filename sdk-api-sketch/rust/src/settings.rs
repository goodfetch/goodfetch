//! Settings in code (ADR 0083, decided in #470).
//!
//! One [`Settings`] value per scope that opens: [`crate::ConnectOptions::settings`],
//! [`crate::Start::settings`] and a page's `Call::settings`; the nearest scope wins and is
//! inherited beneath it. A call takes only its task's inputs and its timeout. There are no setters
//! (G2). Each key is a generated [`Setting`] constant in a module path mirroring its dotted name,
//! so the value's type is checked: `Settings::new().set(timeout::NAVIGATION, Duration::from_secs(60))`.
//! A setting that can be cleared has `T = Option<…>`, and `None` clears it for the scope; a single
//! call keeps `.no_timeout()` (ADR 0080). The keys below are a sample of the generated ones. A key
//! set where no scope beneath can use it fails with `setting_not_applicable` when the scope opens.
//! Deployment variables are no keys here: `connect` reads them from its environment map.

use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::Duration;

use crate::results::{RunLayerOrigin, SettingValue, SettingsLayer};

/// A setting's typed key: its canonical dotted name and the type of its value.
pub struct Setting<T> {
    key: &'static str,
    _value: PhantomData<fn(T)>,
}

impl<T> Setting<T> {
    const fn new(key: &'static str) -> Self {
        Self {
            key,
            _value: PhantomData,
        }
    }

    /// The canonical key, such as `timeout.navigation`, the same in configuration files, errors
    /// and `config explain`.
    pub const fn key(&self) -> SettingKey {
        SettingKey(self.key)
    }
}

impl<T> Clone for Setting<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Setting<T> {}

impl<T> std::fmt::Debug for Setting<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key)
    }
}

/// A setting's canonical key without its value type, for [`explain_settings`](crate::Run::explain_settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SettingKey(&'static str);

impl SettingKey {
    /// The dotted name.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// Settings for one scope and everything opened beneath it. Left out: inherited.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct Settings {
    _private: (),
}

impl Settings {
    /// No settings: everything inherited.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets one setting; for `T = Option<U>`, `None` clears it for the scope.
    pub fn set<T>(self, key: Setting<T>, value: impl Into<T>) -> Self {
        let _ = (key, value.into());
        unimplemented!()
    }
}

/// `timeout.*`: ADR 0080's phases, one family.
pub mod timeout {
    use super::{Duration, Setting};

    /// `timeout.connect`.
    pub const CONNECT: Setting<Option<Duration>> = Setting::new("timeout.connect");
    /// `timeout.action`: the guard's window (ADR 0019).
    pub const ACTION: Setting<Option<Duration>> = Setting::new("timeout.action");
    /// `timeout.navigation`.
    pub const NAVIGATION: Setting<Option<Duration>> = Setting::new("timeout.navigation");
    /// `timeout.wait`: app-state waits.
    pub const WAIT: Setting<Option<Duration>> = Setting::new("timeout.wait");
    /// `timeout.handler`: a crawl handler.
    pub const HANDLER: Setting<Option<Duration>> = Setting::new("timeout.handler");
    /// `timeout.hook`.
    pub const HOOK: Setting<Option<Duration>> = Setting::new("timeout.hook");
    /// `timeout.sink_write`.
    pub const SINK_WRITE: Setting<Option<Duration>> = Setting::new("timeout.sink_write");
    /// `timeout.close`.
    pub const CLOSE: Setting<Option<Duration>> = Setting::new("timeout.close");
    /// `timeout.run`: the default for each Run's own timeout, given at the connection, with no
    /// built-in default. A Run's own is [`crate::Start::timeout`]; set on a Run, this key fails
    /// with `setting_not_applicable` (S2).
    pub const RUN: Setting<Option<Duration>> = Setting::new("timeout.run");
}

/// A browser window's size in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// A position the browser reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geolocation {
    /// Latitude in degrees.
    pub latitude: f64,
    /// Longitude in degrees.
    pub longitude: f64,
    /// Accuracy in metres, if any.
    pub accuracy: Option<f64>,
}

/// A preferred colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorScheme {
    /// `light`.
    Light,
    /// `dark`.
    Dark,
    /// `no-preference`.
    NoPreference,
}

/// `browser.*`: part of a Session's identity, fixed for its life (ADR 0021), so set where a
/// Session opens or above it, never on a page. Playwright's names where the meaning matches (S3).
/// A value that conflicts with an identity fixed elsewhere (a `stealth` Identity profile, an
/// imported login, a Profile directory used in place) fails with `setting_locked`; on a Borrowed
/// browser, any of them fails with `setting_not_applicable`.
pub mod browser {
    use super::{ColorScheme, Geolocation, Setting, Viewport};

    /// `browser.viewport`.
    pub const VIEWPORT: Setting<Option<Viewport>> = Setting::new("browser.viewport");
    /// `browser.device_scale_factor`.
    pub const DEVICE_SCALE_FACTOR: Setting<f64> = Setting::new("browser.device_scale_factor");
    /// `browser.locale`. `None`: matched to the proxy exit, the `standard` Access mode's default.
    pub const LOCALE: Setting<Option<String>> = Setting::new("browser.locale");
    /// `browser.timezone_id`. `None`: matched to the proxy exit, as [`LOCALE`].
    pub const TIMEZONE_ID: Setting<Option<String>> = Setting::new("browser.timezone_id");
    /// `browser.color_scheme`.
    pub const COLOR_SCHEME: Setting<ColorScheme> = Setting::new("browser.color_scheme");
    /// `browser.java_script_enabled`.
    pub const JAVA_SCRIPT_ENABLED: Setting<bool> = Setting::new("browser.java_script_enabled");
    /// `browser.geolocation`.
    pub const GEOLOCATION: Setting<Option<Geolocation>> = Setting::new("browser.geolocation");
}

/// Emulated network conditions, which Playwright closed as not planned (#8622).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkThrottle {
    /// Download throughput.
    pub download_bytes_per_second: u64,
    /// Upload throughput.
    pub upload_bytes_per_second: u64,
    /// Added latency.
    pub latency: Duration,
}

/// `network.*`.
pub mod network {
    use super::{NetworkThrottle, Setting};

    /// `network.throttle`.
    pub const THROTTLE: Setting<Option<NetworkThrottle>> = Setting::new("network.throttle");
}

/// `crawl.*`.
pub mod crawl {
    /// `crawl.payload.*` (ADR 0082).
    pub mod payload {
        use super::super::Setting;

        /// `crawl.payload.max_size`, in bytes.
        pub const MAX_SIZE: Setting<u64> = Setting::new("crawl.payload.max_size");
    }
}

/// One effective value, as `goodfetch config explain` shows it for the scope.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExplainedSetting {
    /// The canonical key; one this crate does not know is kept as received (G3).
    pub key: String,
    /// A Secret reference shows as the reference.
    pub value: SettingValue,
    /// The layer that set it.
    pub layer: SettingsLayer,
    /// Set when `layer` is [`SettingsLayer::Run`].
    pub origin: Option<RunLayerOrigin>,
    /// The configuration file, when `layer` is [`SettingsLayer::Configuration`].
    pub file: Option<PathBuf>,
    /// The layer that locked or capped the value, if any.
    pub locked_by: Option<SettingsLayer>,
}

/// The scopes a setting can be set at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingScope {
    /// The engine connection.
    Connection,
    /// A Run.
    Run,
    /// A Session.
    Session,
    /// A page.
    Page,
}

/// Details of `setting_unknown`: an unknown key, or an unknown `GOODFETCH_` variable at `connect`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SettingUnknown {
    /// The key as written, or the variable.
    pub key: String,
    /// The closest valid key or listed variable, when one is close.
    pub closest: Option<String>,
}

/// Details of `setting_invalid`: a wrong type or an out-of-range value, or an empty or
/// unparsable `GOODFETCH_` variable.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SettingInvalid {
    /// The key.
    pub key: String,
    /// The type or range the setting accepts.
    pub expected: String,
}

/// Details of `setting_not_applicable`: a setting no scope beneath can use.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SettingNotApplicable {
    /// The key.
    pub key: String,
    /// Where the program set it.
    pub scope: SettingScope,
    /// Where the setting takes effect.
    pub applies_to: Vec<SettingScope>,
}

/// Details of `setting_locked`: a value a lower layer locked or capped, or outside a Recipe's
/// exposed bounds, or a `browser.*` value that conflicts with an identity fixed elsewhere; the
/// error's `limit` names that layer, or the setting that fixed the identity, and its value.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SettingLocked {
    /// The key.
    pub key: String,
    /// The layer that locked or capped it, or set the setting that fixed the identity.
    pub locked_by: SettingsLayer,
    /// The setting that fixed a Session's identity, such as `session.source` or the Access
    /// mode's; `None` for a lock or a cap.
    pub fixed_by: Option<String>,
}

/// Details of `capability_unavailable`: a capability the platform cannot provide; the error's
/// `fix` names the setting that turns it off (ADRs 0038, 0057).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CapabilityUnavailable {
    /// The capability.
    pub capability: String,
    /// The platform, such as `linux-arm64-musl`.
    pub platform: String,
}
