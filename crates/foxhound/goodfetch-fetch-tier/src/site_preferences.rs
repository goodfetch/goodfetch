//! What a site's preferences say of a request (ADRs 0016, 0045, 0086,
//! 0090 and 0093): the one judge every request of a crawl's Run asks, a
//! crawl request's own document, a sitemap's fetch, a handler page's
//! navigation and a redirect hop alike ([`Request`]). [`SitePreferences`]
//! holds the Run's settings for them, read once as its crawl starts (ADR
//! 0138).
//!
//! - **robots.txt** ([`SitePreferences::read_robots_txt`]) is read from
//!   the copy the Run reads (the one its user's runs share, on a crawl's
//!   Run), fetched on the transport the caller passes when the copy has
//!   none to use, and only while something uses it
//!   ([`SitePreferences::reads_robots_txt`]): its rules while
//!   `site_preferences.respect_robots_txt` is on, its `Content-Signal`
//!   lines while the AI-use signals are honoured, and, for a crawl
//!   request's document, whether it disallows it while
//!   `site_preferences.mark_robots_txt_ignored` marks the Records taken
//!   against it. The groups for `goodfetch-foxhound` and for
//!   `site_preferences.robots_txt_user_agent` apply, merged, else `*`'s
//!   (ADR 0090).
//! - **Before a request is sent** ([`SitePreferences::judged`]), it is
//!   refused with `robots_txt` when the rules disallow it or robots.txt
//!   could not be read with no earlier copy, with `crawl_delay` when the
//!   Crawl-delay is above `site_preferences.max_crawl_delay`, and with
//!   `use_signals` when the `Content-Signal` lines or `TDMRep`'s well-known
//!   file refuse the Run's `site_preferences.purpose`. A sitemap's fetch is
//!   judged by robots.txt's rules alone: it reads URLs, no content a
//!   Purpose would use.
//! - **Once its document arrived** ([`SitePreferences::document_judged`]),
//!   it is refused with `use_signals` when the AI-use signals its response
//!   and markup carry, or the RSL license they name, refuse the Purpose.
//!
//! Each AI-use signal file is fetched once for the Run ([`SignalFiles`]).
//! When a request may start after the robots.txt it read is its caller's
//! pacing; [`Reading::start_after`] is the part every caller shares.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::response_limits::{LimitExceeded, Limits};
use goodfetch_engine::robots_txt::{self, Answer, Fetch, Fetched, Judgment, Rule};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_page::robots_txt::{ProductToken, applicable_groups};
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::settings::{
    SITE_PREFERENCES_MARK_ROBOTS_TXT_IGNORED, SITE_PREFERENCES_MAX_CRAWL_DELAY,
    SITE_PREFERENCES_RESPECT_ROBOTS_TXT, SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
};
use goodfetch_protocol::page::SitePreference;
use goodfetch_protocol::site_preferences::{
    SITE_PREFERENCES_PURPOSE, SITE_PREFERENCES_RESPECT_USE_SIGNALS,
};
use tokio::sync::oneshot;
use url::Url;

use crate::use_signals::{
    Purpose, SignalFiles, TDMREP_FILE, UseSignals, content_signals_refusing, rsl_refusing,
    tdmrep_file_refusing,
};

/// What a request is, as a site's preferences judge it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// A crawl request's own document: its Records are marked when
    /// robots.txt disallows it and the crawl fetched it anyway.
    CrawlRequest,
    /// A sitemap's fetch, judged by robots.txt's rules alone.
    Sitemap,
    /// A handler page's navigation, `page.request` or service worker's
    /// navigation request, or a hop of a crawl request's redirect chain.
    Navigation,
}

/// What a site's preferences refuse a request for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusedBy {
    /// robots.txt's rules disallow it.
    RobotsTxt,
    /// robots.txt could not be read, with no earlier copy to apply: RFC
    /// 9309 §2.3.1.4 has a crawler assume complete disallow, and a crawl
    /// request retries it as a failed fetch of its own.
    UnreachableRobotsTxt,
    /// robots.txt asks for a Crawl-delay above
    /// `site_preferences.max_crawl_delay`.
    CrawlDelay,
    /// An AI-use signal refuses the Run's Purpose.
    UseSignals,
}

impl RefusedBy {
    /// The reason a crawl request it refuses ends `skipped` with.
    pub fn skip_reason(self) -> SkipReason {
        skip_reason(self.site_preference())
    }

    /// The reason a handler's navigation it refuses is refused with
    /// (`site_preferences_refused`).
    pub fn site_preference(self) -> SitePreference {
        match self {
            Self::RobotsTxt | Self::UnreachableRobotsTxt => SitePreference::RobotsTxt,
            Self::CrawlDelay => SitePreference::CrawlDelay,
            Self::UseSignals => SitePreference::UseSignals,
        }
    }
}

/// The reason a crawl request ends `skipped` with when a site preference
/// `preference` refused it: at its own document, at a redirect hop, or at
/// a navigation its handler let a `site_preferences_refused` escape from
/// (ADR 0093).
pub fn skip_reason(preference: SitePreference) -> SkipReason {
    match preference {
        SitePreference::RobotsTxt => SkipReason::RobotsTxt,
        SitePreference::CrawlDelay => SkipReason::CrawlDelay,
        SitePreference::UseSignals => SkipReason::UseSignals,
    }
}

/// A site preference's refusal of a request: what refused it, and what
/// it says of the request.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct Refusal {
    /// What refused it.
    pub by: RefusedBy,
    /// What it says of the request.
    pub message: String,
}

/// Why a request is not sent: a preference refused it, or what the site
/// declares its preferences in could not be read past a limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotAllowed {
    /// A preference refused it.
    Refused(Refusal),
    /// A file of its preferences passed a limit: `response_limit_exceeded`.
    PastALimit(LimitExceeded),
}

impl From<Refusal> for NotAllowed {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<LimitExceeded> for NotAllowed {
    fn from(limit: LimitExceeded) -> Self {
        Self::PastALimit(limit)
    }
}

/// What a site's preferences let a request do once they allow it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allowed {
    /// The Crawl-delay robots.txt asks for, while its rules are respected:
    /// waited out between starts, at most `site_preferences.max_crawl_delay`.
    pub crawl_delay: Option<Duration>,
    /// Whether robots.txt disallows the crawl request's document, fetched
    /// anyway as the crawl does not respect it, while
    /// `site_preferences.mark_robots_txt_ignored` marks the Records taken
    /// against it (ADR 0138); a robots.txt that could not be read
    /// disallows nothing here.
    pub ignored: bool,
}

/// The robots.txt a request read: where it is, the copy that applies, and
/// when the response of the fetch made for this request arrived, if one
/// was.
#[derive(Clone, Debug)]
pub struct Reading {
    location: Url,
    copy: Arc<Fetched>,
    fetched: Option<SystemTime>,
}

impl Reading {
    /// Where the robots.txt is.
    pub fn location(&self) -> &Url {
        &self.location
    }

    /// The copy that applies.
    pub fn copy(&self) -> &Fetched {
        &self.copy
    }

    /// The copy that applies, as the Run's copies share it: another read
    /// of the same copy answers the same allocation, a copy fetched again
    /// another ([`goodfetch_frontier::CrawlDelays`] records each once).
    pub fn shared_copy(&self) -> &Arc<Fetched> {
        &self.copy
    }

    /// When the response of the fetch this request made arrived; `None`
    /// when it read a copy another request had fetched. A delay counts
    /// from that arrival, the earliest moment the request is known to have
    /// reached the site, never from the fetch's send (ADR 0086).
    pub fn fetched(&self) -> Option<SystemTime> {
        self.fetched
    }

    /// When a request of `key` that read this robots.txt starts, `spacing`
    /// its delay between starts, taken for every run sharing `politeness`
    /// at once ([`goodfetch_frontier::start_after_robots_txt`]): while the
    /// fetch of it, this request's or the one whose copy it read, is less
    /// than `spacing` before, `spacing` after that fetch's response arrived
    /// and after the last start any sharing run took, so the site never
    /// receives it sooner than that after the robots.txt request. Without
    /// a Politeness, or when its file fails, `spacing` after this request's
    /// own fetch. [`AfterRobotsTxt::Kept`] when its admission kept the
    /// delay, or with no spacing.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a transaction already queued.
    pub async fn start_after(
        &self,
        politeness: Option<&Politeness>,
        run: &Run,
        key: &str,
        spacing: Duration,
    ) -> AfterRobotsTxt {
        if spacing.is_zero() {
            return AfterRobotsTxt::Kept;
        }
        let own = || self.fetched.map(|fetched| fetched + spacing);
        let start = match politeness {
            Some(politeness) => goodfetch_frontier::start_after_robots_txt(
                politeness,
                key,
                self.location.as_str(),
                self.fetched,
                spacing,
                run.clock().wall(),
            )
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, run_id = %run.id(), "a start after its robots.txt was not taken");
                own()
            }),
            None => own(),
        };
        start.map_or(AfterRobotsTxt::Kept, AfterRobotsTxt::At)
    }
}

/// When a request starts after the robots.txt it read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterRobotsTxt {
    /// At this moment.
    At(SystemTime),
    /// As its admission spaced it: the robots.txt fetch is far enough
    /// behind.
    Kept,
}

/// The Run's settings for a site's preferences, read once as its crawl
/// starts, and the AI-use signal files it fetched, each once for all its
/// requests.
pub struct SitePreferences {
    /// `site_preferences.respect_robots_txt`.
    respect_robots_txt: bool,
    /// `site_preferences.mark_robots_txt_ignored`.
    mark_robots_txt_ignored: bool,
    /// `site_preferences.max_crawl_delay`, the longest Crawl-delay waited
    /// out.
    max_crawl_delay: Duration,
    /// `site_preferences.robots_txt_user_agent`, when it is a product
    /// token: a value that is none names no group.
    operator: Option<ProductToken>,
    /// The Purpose `site_preferences.purpose` declares, while
    /// `site_preferences.respect_use_signals` is on and it is not
    /// `search`, which is no declaration (ADR 0086).
    purpose: Option<Purpose>,
    signal_files: SignalFiles,
    /// `network.max_json_depth`, for the signal files read as JSON.
    limits: Limits,
}

impl SitePreferences {
    /// The preferences a crawl under `settings` honours.
    pub fn of(settings: &Resolved) -> Self {
        let purpose = settings
            .get::<bool>(&SITE_PREFERENCES_RESPECT_USE_SIGNALS)
            .then(|| Purpose::named(settings.get(&SITE_PREFERENCES_PURPOSE)))
            .flatten()
            .filter(|purpose| *purpose != Purpose::Search);
        Self {
            respect_robots_txt: settings.get(&SITE_PREFERENCES_RESPECT_ROBOTS_TXT),
            mark_robots_txt_ignored: settings.get(&SITE_PREFERENCES_MARK_ROBOTS_TXT_IGNORED),
            max_crawl_delay: settings.get(&SITE_PREFERENCES_MAX_CRAWL_DELAY),
            operator: settings
                .get::<Option<&str>>(&SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT)
                .and_then(|token| ProductToken::new(token).ok()),
            purpose,
            signal_files: SignalFiles::default(),
            limits: Limits::of(settings),
        }
    }

    /// Whether robots.txt's rules are respected.
    pub fn respects_robots_txt(&self) -> bool {
        self.respect_robots_txt
    }

    /// Whether the AI-use signals are judged, against a Purpose: a
    /// document's body is read for its markup's only then.
    pub fn honours_use_signals(&self) -> bool {
        self.purpose.is_some()
    }

    /// Whether `request` reads its origin's robots.txt before it is sent:
    /// only while something uses what it says. Otherwise robots.txt is
    /// neither fetched, read nor stored for it (ADRs 0086 and 0138).
    pub fn reads_robots_txt(&self, request: Request) -> bool {
        match request {
            Request::CrawlRequest => {
                self.respect_robots_txt || self.mark_robots_txt_ignored || self.purpose.is_some()
            }
            Request::Sitemap => self.respect_robots_txt,
            Request::Navigation => self.respect_robots_txt || self.purpose.is_some(),
        }
    }

    /// The robots.txt of `url`'s origin as `run` reads it, from the copy
    /// it reads, `fetch`ed only when the copy has none to use; `None` for
    /// a URL with no robots.txt.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops a fetch it runs and keeps nothing (the
    /// copy's own).
    pub async fn read_robots_txt<F, Fut>(&self, run: &Run, url: &Url, fetch: F) -> Option<Reading>
    where
        F: FnOnce(Url) -> Fut + Send,
        Fut: Future<Output = Answer> + Send,
    {
        let location = robots_txt::location(url)?;
        let (arriving, arrived) = oneshot::channel::<SystemTime>();
        let clock = run.clock();
        let at = location.clone();
        let fetching: Fetch<'_> = Box::pin(async move {
            let answer = fetch(at).await;
            let _ = arriving.send(clock.wall());
            answer
        });
        let copy = run.robots_txt().copy(&location, fetching).await;
        Some(Reading {
            location,
            copy,
            fetched: arrived.await.ok(),
        })
    }

    /// The Crawl-delay the robots.txt `reading` asks of the crawl while
    /// its rules are respected, whether or not it waits it out: the
    /// largest of the groups that apply.
    pub fn crawl_delay_asked(&self, reading: &Reading) -> Option<Duration> {
        if !self.respect_robots_txt {
            return None;
        }
        let Fetched::File(file) = reading.copy() else {
            return None;
        };
        applying_groups(file, self.operator.as_ref())
            .into_iter()
            .filter_map(robots_txt::Group::crawl_delay)
            .max()
    }

    /// What the site's preferences say of `request` to `url` before it is
    /// sent, by the robots.txt it read, `reading`, and, but for a sitemap,
    /// the AI-use signals its host declared: the `Content-Signal` lines of
    /// the groups that apply, then `TDMRep`'s well-known file, fetched once
    /// for the Run with `fetch`.
    ///
    /// # Errors
    ///
    /// [`NotAllowed`] of a request that is not to be sent.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops a fetch it runs, keeping nothing.
    pub async fn judged<F, Fut>(
        &self,
        request: Request,
        url: &Url,
        reading: Option<&Reading>,
        fetch: F,
    ) -> Result<Allowed, NotAllowed>
    where
        F: FnOnce(Url) -> Fut + Send,
        Fut: Future<Output = Answer> + Send,
    {
        let applying = reading.map_or_else(Applying::nothing, |reading| {
            applying(reading.copy(), self.operator.as_ref(), url)
        });
        let allowed = self.by_robots_txt(request, url, &applying)?;
        if request == Request::Sitemap {
            return Ok(allowed);
        }
        let Some(purpose) = self.purpose else {
            return Ok(allowed);
        };
        let refused = |signal: &str| Refusal {
            by: RefusedBy::UseSignals,
            message: format!(
                "{signal} refuses the run's Purpose for `{url}`, so the request never left."
            ),
        };
        if let Some(signal) = content_signals_refusing(&applying.content_signals, purpose) {
            return Err(refused(signal).into());
        }
        let Ok(location) = url.join(TDMREP_FILE) else {
            return Ok(allowed);
        };
        let Some(file) = self.signal_file(&location, fetch).await else {
            return Ok(allowed);
        };
        match tdmrep_file_refusing(&file, (&location, url), purpose, &self.limits)? {
            Some(signal) => Err(refused(signal).into()),
            None => Ok(allowed),
        }
    }

    /// What robots.txt's rules, as `applying` holds them, say of `request`
    /// to `url`.
    fn by_robots_txt(
        &self,
        request: Request,
        url: &Url,
        applying: &Applying,
    ) -> Result<Allowed, Refusal> {
        if !self.respect_robots_txt {
            let ignored = request == Request::CrawlRequest
                && self.mark_robots_txt_ignored
                && matches!(applying.disallowed, Some(Disallowed::Rule(_)));
            return Ok(Allowed {
                crawl_delay: None,
                ignored,
            });
        }
        match &applying.disallowed {
            Some(Disallowed::Unreachable(reason)) => {
                return Err(Refusal {
                    by: RefusedBy::UnreachableRobotsTxt,
                    message: format!(
                        "The site's robots.txt could not be read ({reason}), and no earlier copy applies, so the request to `{url}` was not sent."
                    ),
                });
            }
            Some(Disallowed::Rule(rule)) => {
                return Err(Refusal {
                    by: RefusedBy::RobotsTxt,
                    message: format!(
                        "The site's robots.txt disallows `{url}` to the crawl ({rule}), so the request never left."
                    ),
                });
            }
            None => {}
        }
        let crawl_delay = applying.crawl_delay;
        if let Some(asked) = crawl_delay.filter(|asked| *asked > self.max_crawl_delay) {
            return Err(Refusal {
                by: RefusedBy::CrawlDelay,
                message: format!(
                    "The site's robots.txt asks for a Crawl-delay of {} s, more than `site_preferences.max_crawl_delay` waits out ({} s), so the request to `{url}` never left.",
                    asked.as_secs_f64(),
                    self.max_crawl_delay.as_secs_f64()
                ),
            });
        }
        Ok(Allowed {
            crawl_delay,
            ignored: false,
        })
    }

    /// What the AI-use signals of a document a request to `url` arrived
    /// with, `signals`, its response's headers and its markup, say of it
    /// while they are honoured: refused when one refuses the Run's
    /// Purpose, or the RSL license they name, fetched once for the Run with
    /// `fetch`, does.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] of a document whose handler must not read it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops a fetch it runs, keeping nothing.
    pub async fn document_judged<F, Fut>(
        &self,
        url: &str,
        signals: &UseSignals,
        fetch: F,
    ) -> Result<(), Refusal>
    where
        F: FnOnce(Url) -> Fut + Send,
        Fut: Future<Output = Answer> + Send,
    {
        let Some(purpose) = self.purpose else {
            return Ok(());
        };
        let refused = |signal: &str| Refusal {
            by: RefusedBy::UseSignals,
            message: format!("{signal} refuses the run's Purpose for {url}"),
        };
        if let Some(signal) = signals.refusing(purpose) {
            return Err(refused(signal));
        }
        let Some(license) = signals.license() else {
            return Ok(());
        };
        match self
            .signal_file(&license, fetch)
            .await
            .and_then(|file| rsl_refusing(&file, purpose))
        {
            Some(signal) => Err(refused(signal)),
            None => Ok(()),
        }
    }

    /// The body at `location`, an AI-use signal file, fetched with `fetch`
    /// once for the Run; none for a fetch that answers no 2xx.
    async fn signal_file<F, Fut>(&self, location: &Url, fetch: F) -> Option<Arc<[u8]>>
    where
        F: FnOnce(Url) -> Fut + Send,
        Fut: Future<Output = Answer> + Send,
    {
        self.signal_files
            .read(location, async {
                match fetch(location.clone()).await {
                    Answer::Response { status, body } if (200..300).contains(&status) => Some(body),
                    _ => None,
                }
            })
            .await
    }
}

/// Why robots.txt disallows a URL.
#[derive(Debug)]
enum Disallowed {
    /// This rule of its file.
    Rule(String),
    /// It could not be read, for this reason.
    Unreachable(String),
}

/// What applies of a host's robots.txt to a URL.
struct Applying {
    /// Whether it disallows the URL, and why.
    disallowed: Option<Disallowed>,
    /// The largest Crawl-delay of the groups that apply.
    crawl_delay: Option<Duration>,
    /// The `Content-Signal` lines of the groups that apply, in file order.
    content_signals: Vec<(String, bool)>,
}

impl Applying {
    /// Nothing: no rule disallows the URL, no Crawl-delay is asked and no
    /// Content Signal declared.
    fn nothing() -> Self {
        Self {
            disallowed: None,
            crawl_delay: None,
            content_signals: Vec::new(),
        }
    }
}

/// What applies of `fetched` to `url` for a crawl whose operator token is
/// `operator` (ADRs 0086 and 0090).
fn applying(fetched: &Fetched, operator: Option<&ProductToken>, url: &Url) -> Applying {
    match fetched {
        Fetched::NotFound(_) => Applying::nothing(),
        Fetched::Unreachable(reason) => Applying {
            disallowed: Some(Disallowed::Unreachable(reason.clone())),
            crawl_delay: None,
            content_signals: Vec::new(),
        },
        Fetched::File(file) => {
            let applying = applying_groups(file, operator);
            let rules: Vec<&Rule> = applying.iter().flat_map(|group| group.rules()).collect();
            let crawl_delay = applying
                .iter()
                .filter_map(|group| group.crawl_delay())
                .max();
            let disallowed = match robots_txt::judge(rules, &robots_txt::target(url)) {
                Judgment::Disallowed(rule) => Some(Disallowed::Rule(rule.to_string())),
                Judgment::Allowed(_) => None,
            };
            Applying {
                disallowed,
                crawl_delay,
                content_signals: signals_of(applying),
            }
        }
    }
}

/// The `Content-Signal` lines of `groups`, in file order.
fn signals_of(mut groups: Vec<&robots_txt::Group>) -> Vec<(String, bool)> {
    groups.sort_by_key(|group| group.line());
    groups
        .into_iter()
        .flat_map(|group| group.content_signals().iter().cloned())
        .collect()
}

/// The groups of `file` that apply to a crawl whose operator token is
/// `operator` (ADRs 0086 and 0090).
fn applying_groups<'a>(
    file: &'a robots_txt::RobotsTxt,
    operator: Option<&ProductToken>,
) -> Vec<&'a robots_txt::Group> {
    applicable_groups(file.groups(), robots_txt::Group::user_agents, operator)
}
