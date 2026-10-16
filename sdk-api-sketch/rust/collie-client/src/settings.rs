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
use crate::sessions::{HttpCredentials, PageCheck, Permission, SecretReference, SessionSource};

/// A setting's typed key: its canonical dotted name and the type of its value.
pub struct Setting<T> {
    key: &'static str,
    _value: PhantomData<fn(T)>,
}

impl<T> Setting<T> {
    /// A key of the generated settings: each Product's crate builds its own keys with it.
    #[doc(hidden)]
    pub const fn new(key: &'static str) -> Self {
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

    /// `timeout.connect`, set only in the project file, which `connect` applies: a connection's own is
    /// [`crate::ConnectOptions::timeout`], and this key in a `Settings` value fails with
    /// `setting_not_applicable` (S2).
    pub const CONNECT: Setting<Option<Duration>> = Setting::new("timeout.connect");
    /// `timeout.action`: the guard's window (ADR 0019).
    pub const ACTION: Setting<Option<Duration>> = Setting::new("timeout.action");
    /// `timeout.navigation`.
    pub const NAVIGATION: Setting<Option<Duration>> = Setting::new("timeout.navigation");
    /// `timeout.wait`: app-state waits.
    pub const WAIT: Setting<Option<Duration>> = Setting::new("timeout.wait");
    /// `timeout.close`.
    pub const CLOSE: Setting<Option<Duration>> = Setting::new("timeout.close");
    /// `timeout.run`: the default for each Run's own timeout, given at the connection, with no
    /// built-in default. A Run's own is [`crate::Start::timeout`]; set on a Run, this key fails
    /// with `setting_not_applicable` (S2).
    pub const RUN: Setting<Option<Duration>> = Setting::new("timeout.run");
    /// `timeout.human_pause`: how long a Human pause may wait for a person; built-in 24 hours
    /// (ADR 0092).
    pub const HUMAN_PAUSE: Setting<Option<Duration>> = Setting::new("timeout.human_pause");
    /// An AI verb from its call to its return, model calls and a Repair included: 120 s (ADR 0091).
    pub const AI: Setting<Option<Duration>> = Setting::new("timeout.ai");
    /// One attempt of one model request, the program's own provider included: 60 s. An attempt past
    /// it is retried, within the call that made it (ADR 0101).
    pub const MODEL: Setting<Option<Duration>> = Setting::new("timeout.model");
    /// An Agent's task: 10 minutes. Reaching it stops the task with the reason `timeout` (ADR 0100).
    pub const AGENT: Setting<Option<Duration>> = Setting::new("timeout.agent");
}

/// `agent.*` (ADR 0100). `timeout.agent` is in [`timeout`].
pub mod agent {
    use super::Setting;

    /// Steps a task may take: 20 by default. [`crate::agent::AgentCall::max_steps`] sets it for one
    /// task.
    pub const MAX_STEPS: Setting<u32> = Setting::new("agent.max_steps");
    /// Stalled steps in a row that stop a task: 3 by default.
    pub const MAX_STALLED_STEPS: Setting<u32> = Setting::new("agent.max_stalled_steps");
    /// Where a task may navigate: by default the registrable domain of the page it starts on.
    /// `["*"]` lifts it.
    pub const DOMAINS: Setting<Vec<String>> = Setting::new("agent.domains");
    /// Whether the model may ask a person through a Human pause: `false` by default (ADR 0092).
    pub const HUMAN_PAUSE: Setting<bool> = Setting::new("agent.human_pause");
}

/// `ai.*` (ADR 0101). Model providers are declared with [`Settings::model_provider`].
pub mod ai {
    use super::Setting;
    use crate::models::{ModelReference, StructuredOutput};

    /// `ai.model`: the model every AI call of the scope uses unless the call names its own. No
    /// default: no model is called unless one is configured (ADR 0043).
    pub const MODEL: Setting<Option<ModelReference>> = Setting::new("ai.model");
    /// `ai.structured_output`: [`StructuredOutput::Auto`] by default.
    pub const STRUCTURED_OUTPUT: Setting<StructuredOutput> = Setting::new("ai.structured_output");

    /// `ai.allowance.*`: the Run's Model allowance (ADR 0107), none by default, `None` for none.
    /// Set where a Run opens or above, never on a Label, a Session or a page
    /// (`setting_not_applicable`). Not a Limit: each call reserves its worst case against it, and
    /// once a call does not fit, every later model call of the Run is refused with
    /// `model_allowance_reached` while the Run goes on. When several are set, the first reached
    /// holds.
    pub mod allowance {
        use crate::settings::Setting;
        use crate::time::Money;

        /// `ai.allowance.price`: priced through the price table; an unpriced model is refused with
        /// `model_price_unknown`.
        pub const PRICE: Setting<Option<Money>> = Setting::new("ai.allowance.price");
        /// `ai.allowance.tokens`: input plus output.
        pub const TOKENS: Setting<Option<u64>> = Setting::new("ai.allowance.tokens");
        /// `ai.allowance.calls`.
        pub const CALLS: Setting<Option<u64>> = Setting::new("ai.allowance.calls");
    }
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

pub mod plan {
    //! `plan.*` (ADR 0091).
    use super::Setting;

    /// Replay only: never call a model and never write the Plan; a missing step or a Drift fails.
    /// Default: `false`.
    pub const FROZEN: Setting<bool> = Setting::new("plan.frozen");
}

pub mod extract {
    //! `extract.*` (ADRs 0091, 0123).
    use super::Setting;
    use crate::ai::ExtractMode;

    /// Compiled (the default) or model; `extract()` takes it for one call as `.mode(..)`.
    pub const MODE: Setting<ExtractMode> = Setting::new("extract.mode");
    /// `extract.locale`: the BCP 47 locale every locale-dependent Value transform reads under, such as
    /// `de-DE` (ADR 0123). The compile must use it, and Grounding refuses a transform under any other.
    /// `None`, the default: the compile chooses from the page's evidence, sharing the element's `lang`
    /// language. Never the machine's.
    pub const LOCALE: Setting<Option<String>> = Setting::new("extract.locale");
    /// `extract.timezone_id`: the IANA time zone of a date-time whose text states none, and of a
    /// relative time. `None`, the default: the Session's `browser.timezone_id`. Never the machine's.
    pub const TIMEZONE_ID: Setting<Option<String>> = Setting::new("extract.timezone_id");
}

pub mod observation {
    //! `observation.*` (ADR 0091).
    use super::Setting;

    /// The most characters an AI aria snapshot holds before its deepest subtrees collapse.
    /// Default: 40,000.
    pub const MAX_SIZE: Setting<u64> = Setting::new("observation.max_size");
}

/// `browser.*`: part of a Session's identity, fixed for its life (ADR 0021), so set where a
/// Session opens or above it, never on a page. Playwright's names where the meaning matches (S3).
/// A value set where a Session opens that conflicts with an identity fixed elsewhere (a `stealth`
/// Identity profile, an imported login, a Profile directory used in place) fails with
/// `setting_locked`, and on a Borrowed browser's Session any of them fails with
/// `setting_not_applicable`; an inherited one yields, and `explain_settings` names what fixed it.
pub mod browser {
    use super::{ColorScheme, Geolocation, Permission, Setting, Viewport};

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
    /// `browser.headless`: built-in `true`, `false` under `stealth` (ADR 0035). A person at the
    /// keyboard sees the window (ADR 0092).
    pub const HEADLESS: Setting<bool> = Setting::new("browser.headless");
    /// `browser.permissions`: granted to every page of the Session; none by default, no setter
    /// later (ADR 0092).
    pub const PERMISSIONS: Setting<Vec<Permission>> = Setting::new("browser.permissions");
    /// `browser.block_resources`: resource types the engine's interception blocks on every page
    /// of the Session, before any navigation, so every browser fetch of a crawl is covered
    /// (ADRs 0047, 0117). Takes only the four [`BlockedResourceType`]s, inputs being strict
    /// (G3); none by default. Set on the Run or above it: a Label refuses it
    /// with `setting_not_applicable`, as every `browser.*` key (ADR 0093). In the `standard` and `stealth`
    /// Access modes it warns once, since a site can see the missing loads.
    pub const BLOCK_RESOURCES: Setting<Vec<BlockedResourceType>> =
        Setting::new("browser.block_resources");
    /// `browser.service_workers`: whether a site's service workers run, Playwright's
    /// `serviceWorkers` values (S3, ADR 0121). [`ServiceWorkers::Allow`] by default on Chromium,
    /// where a service worker's own requests pass the engine's interception and a response its
    /// fetch handler supplies is marked `from_service_worker()`; set [`ServiceWorkers::Block`]
    /// when a route or the Response cache must act on every request. `Block` on Firefox and
    /// WebKit, where an explicit `Allow` fails with `capability_unavailable`. On Chromium, `Block`
    /// never patches `register`: registration fails as a browser's own refusal does.
    pub const SERVICE_WORKERS: Setting<ServiceWorkers> = Setting::new("browser.service_workers");

    /// Whether a site's service workers run (ADR 0121).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ServiceWorkers {
        /// `allow`.
        Allow,
        /// `block`.
        Block,
    }

    /// A resource type `browser.block_resources` takes (ADRs 0047, 0117).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum BlockedResourceType {
        /// Images.
        Image,
        /// Audio and video.
        Media,
        /// Web fonts.
        Font,
        /// Style sheets.
        Stylesheet,
    }
}

/// `session.*` (ADRs 0056, 0006, 0092). Opening a Session spells its own source as
/// `Call::<Session>::source`; [`session::SOURCE`] is the default for the Sessions opened beneath a
/// connection or a Run.
pub mod session {
    use super::{HttpCredentials, PageCheck, SessionSource, Setting};

    /// `session.source`: where a Session's login comes from (ADR 0056). Only the program's own
    /// code, configuration, an Operator policy or a person sets it; a Recipe never.
    pub const SOURCE: Setting<SessionSource> = Setting::new("session.source");
    /// `session.page_check` (ADR 0006). `None`: the built-in detector, pages weakly checked.
    pub const PAGE_CHECK: Setting<Option<PageCheck>> = Setting::new("session.page_check");
    /// `session.http_credentials`: sent only to a listed domain that challenges.
    pub const HTTP_CREDENTIALS: Setting<Vec<HttpCredentials>> = Setting::new("session.http_credentials");

    /// `session.recovery.*`.
    pub mod recovery {
        use super::super::Setting;

        /// `session.recovery.human_pause`: whether ADR 0006's ladder may end in a Human pause;
        /// built-in `true`.
        pub const HUMAN_PAUSE: Setting<bool> = Setting::new("session.recovery.human_pause");
        /// `session.recovery.agent`: whether the ladder's AI agent login step runs, the built-in
        /// Agent with the Session's Secret placeholders; built-in `true`, skipped with no model and
        /// no login Plan (ADR 0100).
        pub const AGENT: Setting<bool> = Setting::new("session.recovery.agent");
    }
}

/// `proxy.*`: part of a Session's identity, one proxy per Session (ADR 0021), with Playwright's
/// fields. Pools by kind and provider adapters are this family's too; area 8's spec names their
/// keys. Secret placeholders are bound with [`Settings::secret`].
pub mod proxy {
    use super::{SecretReference, Setting};

    /// `proxy.server`, such as `http://proxy.example:3128`; `None`: no proxy.
    pub const SERVER: Setting<Option<String>> = Setting::new("proxy.server");
    /// `proxy.bypass`: comma-separated domains that skip the proxy.
    pub const BYPASS: Setting<String> = Setting::new("proxy.bypass");
    /// `proxy.username`.
    pub const USERNAME: Setting<String> = Setting::new("proxy.username");
    /// `proxy.password`, a Secret reference, never a value.
    pub const PASSWORD: Setting<SecretReference> = Setting::new("proxy.password");
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

/// `effect.*` (ADR 0106): set on a connection or a Run.
pub mod effect {
    /// `effect.output.*`.
    pub mod output {
        use super::super::Setting;

        /// `effect.output.max_size`, in bytes: the largest output of a declared Effect the journal
        /// keeps. Built-in: 64 KiB.
        pub const MAX_SIZE: Setting<u64> = Setting::new("effect.output.max_size");
    }
}

/// `checkpoint.*` (ADRs 0002, 0102).
pub mod checkpoint {
    use super::Setting;
    use std::time::Duration;

    /// `checkpoint.interval`: how often the Run writes a Checkpoint. Built-in: none locally.
    pub const INTERVAL: Setting<Option<Duration>> = Setting::new("checkpoint.interval");
}

/// `locator.*` (ADR 0085), settable at any scope, a crawl's Label included (ADR 0093).
pub mod locator {
    use super::Setting;

    /// `locator.test_id_attribute`: the attribute `get_by_test_id` matches, `data-testid` by default.
    pub const TEST_ID_ATTRIBUTE: Setting<String> = Setting::new("locator.test_id_attribute");
}

/// How far a run shares Politeness (ADR 0053). Anything but `Shared` is marked in the run's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolitenessSharing {
    /// `shared`, the built-in: through the Politeness store when one is configured, else on the machine.
    Shared,
    /// `per_daemon`: on the machine only.
    PerDaemon,
    /// `run`: this run alone.
    Run,
}

/// `politeness.*` as Collie carries it (ADR 0088): the per-proxy limits, which protect the user's
/// own proxy accounts and hold a program's own navigation through them, and how far they are
/// shared. Politeness toward sites (per domain, backoff, IP) is a crawl's, in Foxhound's crate.
pub mod politeness {
    use super::{PolitenessSharing, Setting};

    /// `politeness.sharing`; built-in `Shared`.
    pub const SHARING: Setting<PolitenessSharing> = Setting::new("politeness.sharing");

    /// `politeness.proxy.*`: per proxy endpoint and credential (ADR 0053), for a crawl and a
    /// program's own navigation through a proxy alike (ADR 0088).
    pub mod proxy {
        use super::super::Setting;

        /// `politeness.proxy.concurrency`. `None`, the built-in: only the ceiling learned from the
        /// proxy's refusals.
        pub const CONCURRENCY: Setting<Option<u32>> = Setting::new("politeness.proxy.concurrency");
        /// `politeness.proxy.rate`, in requests per second. `None`, the built-in: no rate.
        pub const RATE: Setting<Option<f64>> = Setting::new("politeness.proxy.rate");
    }
}

/// `site_preferences.*` as a Collie Run sets it: robots.txt and Crawl-delay are a crawl's (ADR
/// 0084), in Foxhound's crate.
pub mod site_preferences {
    use super::Setting;

    /// `site_preferences.respect_use_signals`: apply the AI-use signals that concern the run's
    /// Purpose (ADR 0016). Built-in `false`. A Collie program's own page is never refused for one
    /// (ADR 0085).
    pub const RESPECT_USE_SIGNALS: Setting<bool> = Setting::new("site_preferences.respect_use_signals");
}

/// One effective value, as `goodfetch-collie config explain` shows it for the scope.
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
    /// The setting that fixed the Session's identity, such as `session.source`, when an inherited
    /// `browser::*` value yielded to it; `value` is then the identity's, and `layer` that setting's.
    pub fixed_by: Option<String>,
}

/// The scopes a setting can be set at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingScope {
    /// The engine connection.
    Connection,
    /// A Run.
    Run,
    /// A crawl's Label (ADR 0093).
    Label,
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
