//! Foxhound's settings (ADR 0083, decided in #470): the keys only a crawl's Run honours
//! (`crawl.*`, `fetch.*`, `sinks.*`, the crawl phases of `timeout.*`, Politeness toward sites and
//! robots.txt), added to Collie's, which are the Collie client crate's (ADRs 0088, 0095). A Collie
//! engine refuses them with `not_in_product`. Each module here holds Collie's keys of its area too,
//! so a Foxhound program reaches every key from this crate: `settings::timeout::HANDLER` beside
//! `settings::timeout::NAVIGATION`. [`Settings`] is Collie's with [`Settings::sink`]; every call that
//! takes settings takes `impl Into<goodfetch_collie_client::Settings>`, which it is.

use std::time::Duration;

pub use goodfetch_collie_client::settings::*;

use goodfetch_collie_client::SecretBinding;

use crate::output::SinkConfig;

/// A Foxhound program's settings for one scope: Collie's [`goodfetch_collie_client::Settings`],
/// with the Sinks only Foxhound has. Left out: inherited.
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

    /// Sets one setting, Collie's or Foxhound's; for `T = Option<U>`, `None` clears it for the scope.
    pub fn set<T>(self, key: Setting<T>, value: impl Into<T>) -> Self {
        let _ = (key, value.into());
        unimplemented!()
    }

    /// Declares the Model provider `name` (`ai.providers.<name>`), as Collie's.
    pub fn model_provider(self, name: &str, provider: goodfetch_collie_client::ModelProvider) -> Self {
        let _ = (name, provider);
        unimplemented!()
    }

    /// Binds the Secret placeholder `name` (`secrets.<name>`), as Collie's.
    pub fn secret(self, name: &str, binding: SecretBinding) -> Self {
        let _ = (name, binding);
        unimplemented!()
    }

    /// Configures the Sink `name` (`sinks.<name>.*`) for this scope. A Collie Run on a Foxhound
    /// engine has no Datasets and refuses one set on it with `setting_not_applicable`, while one
    /// inherited from the connection or configuration is inactive for it.
    pub fn sink(self, name: &str, sink: SinkConfig) -> Self {
        let _ = (name, sink);
        unimplemented!()
    }
}

impl From<Settings> for goodfetch_collie_client::Settings {
    fn from(settings: Settings) -> Self {
        let _ = settings;
        unimplemented!()
    }
}

/// `timeout.*`: Collie's phases and a crawl's own.
pub mod timeout {
    pub use goodfetch_collie_client::settings::timeout::*;

    use super::{Duration, Setting};

    /// `timeout.handler`: a crawl handler.
    pub const HANDLER: Setting<Option<Duration>> = Setting::new("timeout.handler");
    /// `timeout.hook`.
    pub const HOOK: Setting<Option<Duration>> = Setting::new("timeout.hook");
    /// `timeout.sink_write`.
    pub const SINK_WRITE: Setting<Option<Duration>> = Setting::new("timeout.sink_write");
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
    /// `crawl.max_url_length`: characters of a URL as parsed; a longer one is not added with
    /// `MaxUrlLength`, and the finished Run is `unverified`. Built-in: 2,083, Scrapy's; `None` for
    /// none (ADR 0120).
    pub const MAX_URL_LENGTH: Setting<Option<u32>> = Setting::new("crawl.max_url_length");
    /// `crawl.max_path_segments`: a URL with a longer path is not added with `MaxPathSegments`, and
    /// the finished Run is `unverified`. Built-in: 20, Heritrix's; `None` for none (ADR 0120).
    pub const MAX_PATH_SEGMENTS: Setting<Option<u32>> = Setting::new("crawl.max_path_segments");
    /// `crawl.max_segment_repeats`: how many times a path segment, or a run of segments, may follow
    /// itself; past it a URL is not added with `MaxSegmentRepeats`, and the finished Run is
    /// `unverified`. Built-in: 2, Heritrix's; `None` for none (ADR 0120).
    pub const MAX_SEGMENT_REPEATS: Setting<Option<u32>> = Setting::new("crawl.max_segment_repeats");
    /// `crawl.keep_url_fragment`: the fragment stays in a request's key, for a Label whose requests
    /// are hash routes. Such a Label is served only on `light_browser` or `browser`, and a resume may
    /// not change it. Built-in: false (ADR 0120).
    pub const KEEP_URL_FRAGMENT: Setting<bool> = Setting::new("crawl.keep_url_fragment");
    /// `crawl.handle_statuses`: statuses outside 2xx whose responses go to the handler instead of
    /// failing with `response_not_ok`; a redirect status listed here is not followed (ADR 0118).
    /// Built-in: none (ADR 0093).
    pub const HANDLE_STATUSES: Setting<Vec<u16>> = Setting::new("crawl.handle_statuses");
    /// `crawl.content_types`: the media types the handler takes (`type/subtype`, `type/*`, `*/*`),
    /// the type computed by MIME sniffing as Chrome computes it. Any other response that would
    /// reach the handler ends `skipped` with `ContentType` before its body is read. For an
    /// accepted type that is no HTML document, the page holds no document and `ctx.response()`
    /// holds the body (ADR 0119). Built-in: `text/html` and `application/xhtml+xml`.
    pub const CONTENT_TYPES: Setting<Vec<String>> = Setting::new("crawl.content_types");
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
        use crate::crawl::LinkPattern;

        /// Which hosts a crawl may reach, measured against every seed, hosts compared as the WHATWG
        /// parser leaves them and one leading `www.` label ignored (ADR 0120).
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
        pub const INCLUDE: Setting<Vec<LinkPattern>> = Setting::new("crawl.scope.include");
        /// `crawl.scope.exclude`: no URL matching any of these.
        pub const EXCLUDE: Setting<Vec<LinkPattern>> = Setting::new("crawl.scope.exclude");
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
    /// `fetch.encoding`: a WHATWG Encoding label forced on the crawl request's document on every
    /// tier, after its BOM; an unknown label is refused with `setting_invalid`. With none, every
    /// tier reads a BOM, the `Content-Type` charset, the `<meta>` prescan, then UTF-8 for a valid
    /// UTF-8 body, and a body it would have to guess escalates to the browser (ADR 0119).
    /// Built-in: none.
    pub const ENCODING: Setting<Option<String>> = Setting::new("fetch.encoding");

    /// `fetch.kept_responses.*` (ADR 0110): the responses a handler reads with
    /// `ctx.kept_response` and `ctx.kept_responses`, on a crawl's Run or its Labels. A Label that
    /// sets `URLS` while forcing `http` or `browser_fetch` fails the Run's start with
    /// `setting_not_applicable`: those tiers run no script.
    pub mod kept_responses {
        use crate::crawl::LinkPattern;
        use crate::settings::Setting;

        /// `fetch.kept_responses.urls`: the URLs of the responses kept on a request's page from
        /// the start of its navigation, matched in the engine. Built-in: none.
        pub const URLS: Setting<Vec<LinkPattern>> = Setting::new("fetch.kept_responses.urls");
        /// `fetch.kept_responses.max_size`: bytes of kept bodies per attempt; a body past it reads
        /// as `body_unavailable`. Built-in: 16 MiB.
        pub const MAX_SIZE: Setting<u64> = Setting::new("fetch.kept_responses.max_size");
    }

    /// `fetch.redirect.*` (ADR 0118): every tier follows Fetch's redirect statuses (301, 302, 303,
    /// 307, 308) with Fetch's method and header rules, each hop through the navigation gate. A
    /// redirect status in `crawl.handle_statuses` reaches the handler unfollowed: the one way not to
    /// follow.
    pub mod redirect {
        use crate::settings::Setting;

        /// `fetch.redirect.max`: from 1 to 20, the browser's most; past it, `too_many_redirects`.
        /// Built-in: 20.
        pub const MAX: Setting<u32> = Setting::new("fetch.redirect.max");
    }

    /// `fetch.refresh.*` (ADR 0118): a crawl request's document that refreshes to another URL (a
    /// `Refresh` header, or `<meta http-equiv="refresh">` outside `<noscript>`) is one more hop, made
    /// at once on every tier. A refresh to the document itself is a reload, never followed.
    pub mod refresh {
        use crate::settings::{Duration, Setting};

        /// `fetch.refresh.max_delay`: a longer refresh is left on the page; `None` follows any.
        /// Built-in: 100 s, Scrapy's.
        pub const MAX_DELAY: Setting<Option<Duration>> = Setting::new("fetch.refresh.max_delay");
    }
}

/// `export.*`: options of the Export formats (ADR 0103). Every text format is UTF-8 (ADR 0119).
pub mod export {
    /// `export.csv.*`.
    pub mod csv {
        use crate::settings::Setting;

        /// `export.csv.bom`: start a CSV Export with a UTF-8 BOM, for spreadsheets. Built-in: false.
        pub const BOM: Setting<bool> = Setting::new("export.csv.bom");
    }
}

/// `politeness.*` (ADR 0086): one family, so `goodfetch-foxhound config explain politeness` shows
/// every limit, Collie's per-proxy limits included. With nothing set, a crawl keeps one request in
/// flight per domain, one second between starts. They govern the requests a crawl's Frontier admits
/// and every navigation a crawl handler makes from its page; a program's own navigation outside a
/// handler is Collie's, and nothing of Politeness toward sites gates or counts it. A value that
/// presses a site harder than its built-in default is recorded in the Audit log, listed in the run's
/// result and logged once as a warning through `tracing`.
pub mod politeness {
    pub use goodfetch_collie_client::settings::politeness::*;

    use super::{Duration, Setting};

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

    /// `politeness.backoff.*`: on 429, 502, 503, 504, an HTTP/2 `ENHANCE_YOUR_CALM`, and a
    /// connection refused or timed out.
    pub mod backoff {
        use super::super::{Duration, Setting};

        /// `politeness.backoff.enabled`; built-in `true`. Off, the run records that it ignored
        /// them and `Retry-After` (ADR 0023).
        pub const ENABLED: Setting<bool> = Setting::new("politeness.backoff.enabled");
        /// `politeness.backoff.initial`: the least wait. Each wait is drawn from the `Entropy`
        /// handle between it and three times the previous one, at most `MAX`: decorrelated jitter,
        /// never below it (ADR 0118); built-in 2 s.
        pub const INITIAL: Setting<Duration> = Setting::new("politeness.backoff.initial");
        /// `politeness.backoff.max`: the longest computed wait, which a `Retry-After` may exceed;
        /// built-in 60 s.
        pub const MAX: Setting<Duration> = Setting::new("politeness.backoff.max");
    }

    /// `politeness.retry_after.*` (ADR 0118): a `Retry-After` in seconds or as an HTTP-date, on a
    /// 429, 502, 503, 504 or a 3xx, holds its domain for the longer of it and the backoff. Nothing is
    /// sent to the domain before the site's time, whatever this is set to, so it presses no site
    /// harder.
    pub mod retry_after {
        use super::super::{Duration, Setting};

        /// `politeness.retry_after.max`: the longest `Retry-After` a domain is waited for. Past it,
        /// the requests the wait holds end failed with `response_unavailable`, its `retry_after`
        /// set, for `retry_failed()` later. Built-in: 15 minutes.
        pub const MAX: Setting<Duration> = Setting::new("politeness.retry_after.max");
    }

    /// `politeness.ip.*`: keyed by the IP the engine resolves (ADR 0021).
    pub mod ip {
        use super::super::Setting;

        /// `politeness.ip.concurrency`. `None`, the built-in: no limit beyond the domain's.
        pub const CONCURRENCY: Setting<Option<u32>> = Setting::new("politeness.ip.concurrency");
    }
}

/// `site_preferences.*` (ADR 0086): what the engine does with the Site preferences it always reads
/// and records in Provenance (ADR 0016), with Collie's `RESPECT_USE_SIGNALS`. Set on a Collie Run,
/// these keys are refused when the Run opens with `setting_not_applicable`: robots.txt is a
/// crawl's (ADR 0084).
pub mod site_preferences {
    pub use goodfetch_collie_client::settings::site_preferences::*;

    use super::{Duration, Setting};

    /// `site_preferences.respect_robots_txt`: obey robots.txt (RFC 9309) for crawl requests, its
    /// Crawl-delay included, start URLs too. Built-in `true`; `false` is recorded.
    pub const RESPECT_ROBOTS_TXT: Setting<bool> = Setting::new("site_preferences.respect_robots_txt");
    /// `site_preferences.max_crawl_delay`: the longest Crawl-delay a crawl waits out; a host asking
    /// for more has its requests skipped with [`SkipReason::CrawlDelay`](crate::SkipReason::CrawlDelay).
    /// `None`: wait out any. Built-in 30 s.
    pub const MAX_CRAWL_DELAY: Setting<Option<Duration>> = Setting::new("site_preferences.max_crawl_delay");
    /// `site_preferences.robots_txt_user_agent`: the operator's own product token, as the crawl's
    /// user agent names it; its robots.txt groups apply, merged with `goodfetch-foxhound`'s (ADR 0090), before
    /// `*`. `None`, the built-in: `goodfetch-foxhound` alone. Setting it is recorded.
    pub const ROBOTS_TXT_USER_AGENT: Setting<Option<String>> =
        Setting::new("site_preferences.robots_txt_user_agent");
}
