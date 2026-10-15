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
    /// Where a task may go: by default the registrable domain of the page it starts on.
    pub const DOMAINS: Setting<Option<Vec<String>>> = Setting::new("agent.domains");
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
    //! `extract.*` (ADR 0091).
    use super::Setting;
    use crate::ai::ExtractMode;

    /// Compiled (the default) or model; `extract()` takes it for one call as `.mode(..)`.
    pub const MODE: Setting<ExtractMode> = Setting::new("extract.mode");
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

/// `crawl.*`.
pub mod crawl {
    use super::Setting;

    /// `crawl.max_depth`: seeds are at 0. Reaching it refuses further enqueues with `MaxDepth` and
    /// ends the Run `incomplete`. Built-in: none (ADR 0093).
    pub const MAX_DEPTH: Setting<Option<u32>> = Setting::new("crawl.max_depth");
    /// `crawl.max_requests`: requests admitted to the Frontier. Reaching it refuses further enqueues
    /// with `MaxRequests` and ends the Run `incomplete`. Built-in: none (ADR 0093).
    pub const MAX_REQUESTS: Setting<Option<u64>> = Setting::new("crawl.max_requests");
    /// `crawl.handle_statuses`: statuses outside 2xx whose responses go to the handler instead of
    /// failing with `response_not_ok`. Built-in: none (ADR 0093).
    pub const HANDLE_STATUSES: Setting<Vec<u16>> = Setting::new("crawl.handle_statuses");
    /// `crawl.keep_alive`: the Run stays `running` with an empty Frontier, waiting for `add_seed`,
    /// until `finish()` lets it end once drained (ADR 0102, ADR 0037's RT-12). Built-in: false.
    pub const KEEP_ALIVE: Setting<bool> = Setting::new("crawl.keep_alive");

    /// `crawl.payload.*` (ADR 0082).
    pub mod payload {
        use super::super::Setting;

        /// `crawl.payload.max_size`, in bytes.
        pub const MAX_SIZE: Setting<u64> = Setting::new("crawl.payload.max_size");
    }

    /// `crawl.scope.*`: checked on every enqueue and every redirect (ADR 0093).
    pub mod scope {
        use super::super::Setting;
        use crate::crawl::UrlPattern;

        /// Which hosts a crawl may reach, measured against every seed after the Frontier's URL
        /// rules, so `www.` is no other host (ADR 0005).
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum Strategy {
            /// `same_hostname`, the built-in, Crawlee's default.
            #[default]
            SameHostname,
            /// `same_domain`: the registrable domain, subdomains included.
            SameDomain,
            /// `same_origin`: scheme, host and port.
            SameOrigin,
            /// `all`: any host, for `INCLUDE` to narrow.
            All,
        }

        /// `crawl.scope.strategy`.
        pub const STRATEGY: Setting<Strategy> = Setting::new("crawl.scope.strategy");
        /// `crawl.scope.include`: only URLs matching one of these, which must match as well as the strategy.
        pub const INCLUDE: Setting<Vec<UrlPattern>> = Setting::new("crawl.scope.include");
        /// `crawl.scope.exclude`: no URL matching any of these.
        pub const EXCLUDE: Setting<Vec<UrlPattern>> = Setting::new("crawl.scope.exclude");
    }

    /// `crawl.retry.*` (ADR 0093).
    pub mod retry {
        use super::super::Setting;

        /// `crawl.retry.max`: retries after the first attempt, for retryable errors and
        /// `ctx.retry()`. Built-in: 2, Scrapy's.
        pub const MAX: Setting<u32> = Setting::new("crawl.retry.max");
        /// `crawl.retry.handler_failed`: retry `handler_failed` too, as Crawlee does. Built-in:
        /// false: a bug fails the same way again (G5).
        pub const HANDLER_FAILED: Setting<bool> = Setting::new("crawl.retry.handler_failed");
    }

    /// `crawl.handler.*` (ADR 0093).
    pub mod handler {
        use super::super::Setting;

        /// `crawl.handler.concurrency`: handlers running at once in the program, the engine's
        /// leases to it (ADR 0081). On a Label, within the Run's. Built-in: 16.
        pub const CONCURRENCY: Setting<u32> = Setting::new("crawl.handler.concurrency");
    }
}

/// `locator.*` (ADR 0085), settable at any scope, a crawl's Label included (ADR 0093).
pub mod locator {
    use super::Setting;

    /// `locator.test_id_attribute`: the attribute `get_by_test_id` matches, `data-testid` by default.
    pub const TEST_ID_ATTRIBUTE: Setting<String> = Setting::new("locator.test_id_attribute");
}

/// `fetch.*`: the fetch tiers of ADR 0020 (ADR 0093).
pub mod fetch {
    use super::Setting;
    use crate::crawl::FetchTier;

    /// A tier choice: learned, or forced (ADR 0023).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum TierChoice {
        /// `auto`, the built-in: learned per Label on a domain by sample comparison.
        #[default]
        Auto,
        /// Forced, which Provenance records.
        Forced(FetchTier),
    }

    /// `fetch.tier`.
    pub const TIER: Setting<TierChoice> = Setting::new("fetch.tier");
    /// `fetch.tier_race`, ADR 0040's `tier_race`. Built-in: false.
    pub const TIER_RACE: Setting<bool> = Setting::new("fetch.tier_race");
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

/// `politeness.*` (ADR 0086): one family, so `goodfetch config explain politeness` shows every
/// limit. With nothing set, a crawl keeps one request in flight per domain, one second between
/// starts. They govern the requests a crawl's Frontier admits and every navigation a crawl handler
/// makes from its page. Politeness is Foxhound's (ADR 0084): a program's own navigation outside a
/// handler is Collie's, and nothing of Politeness gates or counts it; whether Collie carries the
/// per-proxy limits is #455's. They are set on a connection or a Run; set on a Collie Run, one is
/// refused when the Run opens with `setting_not_applicable`, since one `Settings` type takes every
/// key (ADR 0083). A value that presses a site harder than its built-in default is recorded in the
/// Audit log, listed in the run's result and logged once as a warning through `tracing`.
pub mod politeness {
    use super::{Duration, PolitenessSharing, Setting};

    /// `politeness.sharing`; built-in `Shared`.
    pub const SHARING: Setting<PolitenessSharing> = Setting::new("politeness.sharing");
    /// `politeness.degraded_max`: how long an engine cut off from the Politeness store keeps
    /// admitting within its share (ADR 0053); built-in 10 minutes. The engine's own, set in user
    /// configuration or an Operator policy only: in a program's `Settings` it is refused with
    /// `setting_not_applicable`, and its key serves `explain_settings`.
    pub const DEGRADED_MAX: Setting<Duration> = Setting::new("politeness.degraded_max");

    /// `politeness.domain.*`: per domain, keyed as in ADR 0004, shared by every run of the user.
    pub mod domain {
        use super::super::{Duration, Setting};

        /// `politeness.domain.concurrency`: requests in flight per domain, each from its start
        /// until its response (a browser-tier page until its load event). Built-in 1; `0` fails
        /// with `setting_invalid`. At 1, a crawl's Run that switches hedges or `fetch.tier_race` on is
        /// refused.
        pub const CONCURRENCY: Setting<u32> = Setting::new("politeness.domain.concurrency");
        /// `politeness.domain.delay`: the least time between two request starts to one domain;
        /// built-in one second. While robots.txt is respected, a host's Crawl-delay raises it for
        /// the host's whole domain key.
        pub const DELAY: Setting<Duration> = Setting::new("politeness.domain.delay");
    }

    /// `politeness.backoff.*`: on 429, 502, 503, 504, `Retry-After`, and a connection refused or
    /// timed out.
    pub mod backoff {
        use super::super::{Duration, Setting};

        /// `politeness.backoff.enabled`; built-in `true`. Off, the run records that it ignored
        /// them (ADR 0023).
        pub const ENABLED: Setting<bool> = Setting::new("politeness.backoff.enabled");
        /// `politeness.backoff.initial`: the first wait, doubled at each signal; built-in 2 s.
        pub const INITIAL: Setting<Duration> = Setting::new("politeness.backoff.initial");
        /// `politeness.backoff.max`: the longest wait; built-in 60 s.
        pub const MAX: Setting<Duration> = Setting::new("politeness.backoff.max");
    }

    /// `politeness.ip.*`: keyed by the IP the engine resolves (ADR 0021).
    pub mod ip {
        use super::super::Setting;

        /// `politeness.ip.concurrency`. `None`, the built-in: no limit beyond the domain's.
        pub const CONCURRENCY: Setting<Option<u32>> = Setting::new("politeness.ip.concurrency");
    }

    /// `politeness.proxy.*`: per proxy endpoint and credential (ADR 0053), for a crawl; whether
    /// Collie carries them is #455's.
    pub mod proxy {
        use super::super::Setting;

        /// `politeness.proxy.concurrency`. `None`, the built-in: only the ceiling learned from the
        /// proxy's refusals.
        pub const CONCURRENCY: Setting<Option<u32>> = Setting::new("politeness.proxy.concurrency");
        /// `politeness.proxy.rate`, in requests per second. `None`, the built-in: no rate.
        pub const RATE: Setting<Option<f64>> = Setting::new("politeness.proxy.rate");
    }
}

/// `site_preferences.*` (ADR 0086): what the engine does with the Site preferences it always reads
/// and records in Provenance (ADR 0016). Set on a Collie Run, every key but
/// [`site_preferences::RESPECT_USE_SIGNALS`] is refused when the Run opens with
/// `setting_not_applicable`: robots.txt is a crawl's (ADR 0084).
pub mod site_preferences {
    use super::{Duration, Setting};

    /// `site_preferences.respect_robots_txt`: obey robots.txt (RFC 9309) for crawl requests, its
    /// Crawl-delay included, start URLs too. Built-in `true`; `false` is recorded.
    pub const RESPECT_ROBOTS_TXT: Setting<bool> = Setting::new("site_preferences.respect_robots_txt");
    /// `site_preferences.max_crawl_delay`: the longest Crawl-delay a crawl waits out; a host asking
    /// for more has its requests skipped with [`SkipReason::CrawlDelay`](crate::SkipReason::CrawlDelay).
    /// `None`: wait out any. Built-in 30 s.
    pub const MAX_CRAWL_DELAY: Setting<Option<Duration>> = Setting::new("site_preferences.max_crawl_delay");
    /// `site_preferences.robots_txt_user_agent`: the operator's own product token, as the crawl's
    /// user agent names it; its robots.txt groups apply, merged with `Foxhound`'s, before `*`.
    /// `None`, the built-in: `Foxhound` alone. Setting it is recorded.
    pub const ROBOTS_TXT_USER_AGENT: Setting<Option<String>> =
        Setting::new("site_preferences.robots_txt_user_agent");
    /// `site_preferences.respect_use_signals`: apply the AI-use signals that concern the run's
    /// Purpose (ADR 0016). Built-in `false`.
    pub const RESPECT_USE_SIGNALS: Setting<bool> = Setting::new("site_preferences.respect_use_signals");
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
    /// The setting that fixed the Session's identity, such as `session.source`, when an inherited
    /// `browser::*` value yielded to it; `value` is then the identity's, and `layer` that setting's.
    pub fixed_by: Option<String>,
    /// When `layer` is the default: the SDK handler, by name, that stands in for the built-in
    /// default (ADR 0104).
    pub registered: Option<String>,
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
