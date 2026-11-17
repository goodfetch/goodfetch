//! The version check (ADR 0035; spec #599): a Chromium- or Firefox-based
//! build is graded before a Session uses it against the release table every
//! release ships, `browser-releases.toml`, which `xtask browser-releases`
//! writes from Chromium Dash's milestone schedule and Mozilla's release
//! calendar, with the scheduled dates of upcoming majors, so the date alone
//! says which major is Stable today. It makes no network call.
//!
//! A grade is current (nothing shown), 1 version behind (info), 2 or 3
//! behind (a notice), or 4 or more behind or no longer patched by its
//! vendor (a warning that calls the build outdated), each with its gap in
//! versions and weeks. An installed Chrome is graded against its own
//! channel, and one ADR 0030 supports never above a notice; Playwright's
//! `WebKit` is not graded. Nothing here blocks a run.

use std::sync::LazyLock;

use goodfetch_engine::runs::Run;
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::BROWSER_BUILD_OUTDATED;
use jiff::civil::Date;
use serde::Deserialize;

/// The release table `xtask browser-releases` writes, embedded in the
/// executable.
const SHIPPED: &str = include_str!("version_check/browser-releases.toml");

static TABLE: LazyLock<ReleaseTable> = LazyLock::new(|| {
    ReleaseTable::parse(SHIPPED)
        .expect("invariant: the shipped release table reads, as its test holds")
});

/// How many days after its Stable date a major stays supported as an
/// installed Chrome (ADR 0030: a stable major of the last eight weeks).
const SUPPORTED_DAYS: i64 = 8 * 7;

/// Chrome's and Firefox's majors, each with the day it reached Stable (for
/// Firefox, its release), oldest first.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseTable {
    chrome: Vec<Release>,
    firefox: Vec<Release>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
    major: u32,
    stable: Date,
    /// The last day its vendor patches it, when the vendor announced one.
    #[serde(default)]
    patched_until: Option<Date>,
}

/// Why a release table was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TableError {
    #[error("the release table does not read: {0}")]
    Unreadable(#[from] toml::de::Error),
    #[error("the release table's {0} releases are not at least two, with majors and dates rising")]
    OutOfOrder(&'static str),
}

impl ReleaseTable {
    /// The table shipped in the executable.
    pub(crate) fn shipped() -> &'static Self {
        &TABLE
    }

    /// Reads a table, which holds at least two releases of each browser,
    /// each major and date after the one before.
    pub(crate) fn parse(text: &str) -> Result<Self, TableError> {
        let table: Self = toml::from_str(text)?;
        for (name, releases) in [("Chrome", &table.chrome), ("Firefox", &table.firefox)] {
            let rising = releases
                .windows(2)
                .all(|pair| pair[0].major < pair[1].major && pair[0].stable < pair[1].stable);
            if releases.len() < 2 || !rising {
                return Err(TableError::OutOfOrder(name));
            }
        }
        Ok(table)
    }
}

/// The release line a build is graded against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    /// A Chromium build the engine launches, such as the pinned Chrome for
    /// Testing or a distribution's Chromium: Chrome's Stable.
    Chrome,
    /// An installed Chrome, on its own channel (ADR 0030).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no launch picks an installed Chrome by its channel, or WebKit, yet; `grade` takes them as ADR 0035 says"
        )
    )]
    InstalledChrome(ChromeChannel),
    /// A Firefox-based build, Camoufox included: Firefox's current release.
    Firefox,
    /// Playwright's `WebKit`, pinned with each release: not graded.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no launch picks an installed Chrome by its channel, or WebKit, yet; `grade` takes them as ADR 0035 says"
        )
    )]
    WebKit,
}

/// An installed Chrome's channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no launch picks an installed Chrome by its channel, or WebKit, yet; `grade` takes them as ADR 0035 says"
    )
)]
pub(crate) enum ChromeChannel {
    Stable,
    /// Chrome's even majors, which it keeps patching for enterprises.
    ExtendedStable,
    Beta,
    Dev,
}

/// A build's grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grade {
    /// Not graded: `WebKit`, or a day before the table's first release.
    NotGraded,
    /// Its line's current major, or later: nothing shown.
    Current,
    Behind(Gap),
}

/// How far a build is behind its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Gap {
    pub(crate) level: Level,
    /// Majors between the build's and its line's current one.
    pub(crate) versions: u32,
    /// Whole weeks since its line moved past the build.
    pub(crate) weeks: u32,
    /// Its line's current major.
    pub(crate) current: u32,
    /// The last day its vendor patched it, once that day is past.
    pub(crate) unpatched_since: Option<Date>,
}

/// How loudly a gap is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    /// 1 version behind.
    Info,
    /// 2 or 3 behind.
    Notice,
    /// 4 or more behind, or no longer patched: `browser_build_outdated`.
    Outdated,
}

/// Grades the build whose `Browser.getVersion` product is `product` on
/// `run`'s day, against the shipped table: an outdated build warns in the
/// Run report, once, and a build less behind is logged. The run goes on
/// either way.
pub(crate) async fn check(run: &Run, product: &str) {
    let Some((major, channel)) = of_product(product) else {
        return;
    };
    let today = run
        .clock()
        .timestamp()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let Grade::Behind(gap) = grade(major, channel, ReleaseTable::shipped(), today) else {
        return;
    };
    let message = describe(major, channel, &gap);
    match gap.level {
        Level::Info => tracing::info!(%message, "the Run's browser build is a version behind"),
        Level::Notice => tracing::warn!(%message, "the Run's browser build is behind"),
        Level::Outdated => {
            let warning = RunWarning {
                code: BROWSER_BUILD_OUTDATED.to_owned(),
                message,
                sessions: Vec::new(),
                folder: None,
            };
            if let Err(error) = run.warn_once(warning).await {
                tracing::warn!(%error, "the warning of the outdated browser build was not kept");
            }
        }
    }
}

/// The major and line of the build a browser's `Browser.getVersion`
/// product names, such as `HeadlessChrome/156.0.8078.4`; none for a
/// browser the check does not know.
pub(crate) fn of_product(product: &str) -> Option<(u32, Channel)> {
    let (name, version) = product.split_once('/')?;
    let channel = match name {
        "Chrome" | "HeadlessChrome" | "Chromium" => Channel::Chrome,
        "Firefox" => Channel::Firefox,
        _ => return None,
    };
    let major = version.split('.').next()?.parse().ok()?;
    Some((major, channel))
}

/// Grades the build of `major` on `channel` against `table` on `today`.
pub(crate) fn grade(major: u32, channel: Channel, table: &ReleaseTable, today: Date) -> Grade {
    let (releases, channel_ahead) = match channel {
        Channel::WebKit => return Grade::NotGraded,
        Channel::Chrome => (Line(&table.chrome), Ahead::By(0)),
        Channel::Firefox => (Line(&table.firefox), Ahead::By(0)),
        Channel::InstalledChrome(chrome) => (
            Line(&table.chrome),
            match chrome {
                ChromeChannel::Stable => Ahead::By(0),
                ChromeChannel::Beta => Ahead::By(1),
                ChromeChannel::Dev => Ahead::By(2),
                ChromeChannel::ExtendedStable => Ahead::Extended,
            },
        ),
    };
    let Some(stable) = releases.current(today) else {
        return Grade::NotGraded;
    };
    let (current, moved_past) = match channel_ahead {
        // The channel left `major` when Stable reached `major + 1 - ahead`.
        Ahead::By(ahead) => (
            stable.saturating_add(ahead),
            releases.stable_from(major.saturating_add(1).saturating_sub(ahead)),
        ),
        // Extended Stable leaves an even major for the next even one.
        Ahead::Extended => (
            stable - stable % 2,
            releases.stable_from(major.saturating_add(2) - major % 2),
        ),
    };
    let unpatched_since = releases
        .patched_until(major)
        .filter(|&patched_until| patched_until < today);
    if major >= current && unpatched_since.is_none() {
        return Grade::Current;
    }
    let versions = current.saturating_sub(major);
    let mut level = match versions {
        0 | 1 => Level::Info,
        2 | 3 => Level::Notice,
        _ => Level::Outdated,
    };
    if unpatched_since.is_some() {
        level = Level::Outdated;
    }
    let supported = matches!(channel, Channel::InstalledChrome(_))
        && releases
            .stable_of(major)
            .is_some_and(|released| released <= today && days(released, today) <= SUPPORTED_DAYS);
    if supported {
        level = level.min(Level::Notice);
    }
    Grade::Behind(Gap {
        level,
        versions,
        weeks: u32::try_from(days(moved_past, today).max(0) / 7).unwrap_or(u32::MAX),
        current,
        unpatched_since,
    })
}

/// The warning's message for a build of `major` on `channel` with `gap`.
pub(crate) fn describe(major: u32, channel: Channel, gap: &Gap) -> String {
    let (browser, line) = match channel {
        Channel::Chrome => ("Chrome", "Chrome's Stable"),
        Channel::Firefox => ("Firefox", "Firefox's release"),
        Channel::InstalledChrome(ChromeChannel::Stable) => ("Chrome", "Chrome's Stable channel"),
        Channel::InstalledChrome(ChromeChannel::ExtendedStable) => {
            ("Chrome", "Chrome's Extended Stable channel")
        }
        Channel::InstalledChrome(ChromeChannel::Beta) => ("Chrome", "Chrome's Beta channel"),
        Channel::InstalledChrome(ChromeChannel::Dev) => ("Chrome", "Chrome's Dev channel"),
        Channel::WebKit => ("WebKit", "WebKit"),
    };
    let state = match gap.level {
        Level::Info | Level::Notice => "behind",
        Level::Outdated => "outdated",
    };
    let unpatched = gap
        .unpatched_since
        .map(|day| format!(", and its vendor stopped patching it on {day}"))
        .unwrap_or_default();
    format!(
        "The browser build, {browser} {major}, is {state}: {} and {} behind {line}, {}{unpatched}. Update it, or pin a newer build (ADR 0035).",
        counted(gap.versions, "version"),
        counted(gap.weeks, "week"),
        gap.current,
    )
}

fn counted(count: u32, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

/// How far ahead of Stable a channel's major is.
#[derive(Clone, Copy)]
enum Ahead {
    By(u32),
    /// Extended Stable: Stable's major, or the even one before it.
    Extended,
}

/// One browser's releases, with the majors past the table's last coming at
/// the cadence of its last two.
struct Line<'a>(&'a [Release]);

impl Line<'_> {
    fn first(&self) -> Release {
        self.0[0]
    }

    fn last(&self) -> Release {
        self.0[self.0.len() - 1]
    }

    /// Days between the table's last two releases.
    fn cadence(&self) -> i64 {
        days(self.0[self.0.len() - 2].stable, self.last().stable)
    }

    /// The major that is Stable on `today`; none before the table's first.
    fn current(&self, today: Date) -> Option<u32> {
        let last = self.last();
        if today >= last.stable {
            let later = days(last.stable, today) / self.cadence();
            return Some(
                last.major
                    .saturating_add(u32::try_from(later).unwrap_or(u32::MAX)),
            );
        }
        self.0
            .iter()
            .rev()
            .find(|release| release.stable <= today)
            .map(|release| release.major)
    }

    /// When `major` reached Stable, listed or past the table's last; none
    /// for a major the table skips or predates.
    fn stable_of(&self, major: u32) -> Option<Date> {
        let last = self.last();
        if major > last.major {
            return self.after_last(major);
        }
        self.0
            .iter()
            .find(|release| release.major == major)
            .map(|release| release.stable)
    }

    /// When the first major from `major` on reached Stable, the table's
    /// first for an older one.
    fn stable_from(&self, major: u32) -> Date {
        if major <= self.first().major {
            return self.first().stable;
        }
        self.0
            .iter()
            .find(|release| release.major >= major)
            .map(|release| release.stable)
            .or_else(|| self.after_last(major))
            .unwrap_or(Date::MAX)
    }

    fn after_last(&self, major: u32) -> Option<Date> {
        let last = self.last();
        let steps = i64::from(major.checked_sub(last.major)?);
        last.stable
            .checked_add(
                jiff::Span::new()
                    .try_days(steps.checked_mul(self.cadence())?)
                    .ok()?,
            )
            .ok()
    }

    fn patched_until(&self, major: u32) -> Option<Date> {
        self.0
            .iter()
            .find(|release| release.major == major)
            .and_then(|release| release.patched_until)
    }
}

/// Whole days from `from` to `to`.
fn days(from: Date, to: Date) -> i64 {
    i64::from((to - from).get_days())
}

#[cfg(test)]
mod tests;
