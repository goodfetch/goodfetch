//! Global Privacy Control (`consent.gpc`, ADR 0048; issue 758): sends
//! `Sec-GPC: 1` and exposes `navigator.globalPrivacyControl` only when the
//! browser a Session presents would send them under `auto`, or when `on`
//! forces them.
//!
//! Under `auto`, Brave, DuckDuckGo and private Firefox send GPC by default;
//! stock Chrome, Edge, Android Chrome, Safari and regular Firefox do not.
//! Under `on`, a launched Chromium Session gets a browser process of its own
//! started with `--enable-features=GlobalPrivacyControlTest` and configured
//! via `Browser.setGlobalPrivacyControl`, warning (`gpc_forced`) when the
//! presented browser does not send GPC by default; on a Borrowed browser,
//! where Goodfetch did not launch the process, `Sec-GPC: 1` is sent through
//! interception step 7 and the warning also says that
//! `navigator.globalPrivacyControl` is absent. Under `off`, neither is sent.

use goodfetch_engine::settings::{Resolved, Values, value_as};
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::GPC_FORCED;
use goodfetch_protocol::settings::CONSENT_GPC;
use goodfetch_protocol::settings::session_source::{BrowserName, SessionSource};

/// The value of `consent.gpc` for a Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpcMode {
    /// Send GPC only when the presented browser sends it by default.
    Auto,
    /// Force GPC on (own browser on launched Chromium; header-only on a
    /// Borrowed browser), warning when it contradicts the presented browser
    /// or lacks `navigator.globalPrivacyControl`.
    On,
    /// Send neither `Sec-GPC` nor `navigator.globalPrivacyControl`.
    Off,
}

impl GpcMode {
    /// Reads `consent.gpc` from a Session's `own` settings over its Run's
    /// `run` settings, defaulting to [`Self::Auto`].
    pub(crate) fn of(own: &Values, run: Option<&Resolved>) -> Self {
        let written: Option<&str> = own
            .get(CONSENT_GPC.name)
            .map(|value| value_as(&CONSENT_GPC, value))
            .or_else(|| {
                run.and_then(|resolved| resolved.effective(&CONSENT_GPC))
                    .map(|effective| value_as(&CONSENT_GPC, &effective.value))
            });
        match written {
            Some("on") => Self::On,
            Some("off") => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// The browser identity a Session presents, for deciding whether it sends
/// Global Privacy Control by default (ADR 0048: Brave, DuckDuckGo and
/// private Firefox send GPC by default; stock Chrome, Edge, Safari and
/// regular Firefox do not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentedBrowser {
    Chrome,
    Edge,
    Safari,
    Firefox,
    Brave,
}

impl PresentedBrowser {
    /// Whether this browser sends `Sec-GPC: 1` and exposes
    /// `navigator.globalPrivacyControl` by default (ADR 0048).
    pub(crate) const fn sends_gpc_by_default(self) -> bool {
        matches!(self, Self::Brave)
    }

    /// The browser identity implied by a Session's `source`: its browser's
    /// for a source that names one, stock Chrome otherwise.
    pub(crate) const fn of_source(source: Option<&SessionSource>) -> Self {
        let Some(
            SessionSource::ProfileDirectory { browser, .. }
            | SessionSource::InstalledBrowser { browser, .. }
            | SessionSource::RunningBrowser { browser, .. },
        ) = source
        else {
            return Self::Chrome;
        };
        match browser {
            BrowserName::Brave => Self::Brave,
            BrowserName::Edge => Self::Edge,
            BrowserName::Safari => Self::Safari,
            BrowserName::Firefox => Self::Firefox,
            BrowserName::Chrome | BrowserName::Chromium => Self::Chrome,
        }
    }

    /// Human-readable name for warning messages.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Safari => "Safari",
            Self::Firefox => "Firefox",
            Self::Brave => "Brave",
        }
    }
}

/// Why forcing Global Privacy Control warns in the Run report (ADR 0048).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpcWarning {
    /// `consent.gpc: on` on a launched browser whose presented identity does
    /// not send GPC by default, breaking `standard` identity consistency.
    Consistency { browser: PresentedBrowser },
    /// `consent.gpc: on` on a Borrowed browser, where only the `Sec-GPC`
    /// header goes through interception and `navigator.globalPrivacyControl`
    /// is absent.
    BorrowedHeaderOnly,
}

impl GpcWarning {
    /// The Run report warning recording this degradation for `session`.
    pub(crate) fn to_run_warning(self, session: &SessionId) -> RunWarning {
        let message = match self {
            Self::Consistency { browser } => format!(
                "`consent.gpc: on` forces Global Privacy Control (`Sec-GPC: 1` and `navigator.globalPrivacyControl`) on a {} identity, which does not send it by default; this breaks the consistency `standard` keeps",
                browser.label()
            ),
            Self::BorrowedHeaderOnly => "`consent.gpc: on` on a Borrowed browser sends the `Sec-GPC: 1` header through interception alone; `navigator.globalPrivacyControl` is absent because Goodfetch did not launch this browser".to_owned(),
        };
        RunWarning {
            code: GPC_FORCED.to_owned(),
            message,
            sessions: vec![session.0.clone()],
            folder: None,
        }
    }
}

/// How a Session carries out its `consent.gpc` setting (ADR 0048).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpcPlan {
    /// Send neither `Sec-GPC` nor `navigator.globalPrivacyControl`.
    Off,
    /// Launch a browser process of this Session's own with
    /// `--enable-features=GlobalPrivacyControlTest` and enable GPC through
    /// CDP's `Browser.setGlobalPrivacyControl`.
    OwnBrowser { warning: Option<GpcWarning> },
    /// On a Borrowed browser, inject `Sec-GPC: 1` in interception step 7 and
    /// warn that `navigator.globalPrivacyControl` is absent.
    HeaderOnly { warning: GpcWarning },
}

impl GpcPlan {
    /// Resolves `mode` for a Session presenting `presented`, on a Borrowed
    /// browser when `borrowed`.
    pub(crate) const fn resolve(
        mode: GpcMode,
        presented: PresentedBrowser,
        borrowed: bool,
    ) -> Self {
        match (mode, borrowed) {
            (GpcMode::Auto, false) if presented.sends_gpc_by_default() => {
                Self::OwnBrowser { warning: None }
            }
            (GpcMode::Off | GpcMode::Auto, _) => Self::Off,
            (GpcMode::On, true) => Self::HeaderOnly {
                warning: GpcWarning::BorrowedHeaderOnly,
            },
            (GpcMode::On, false) => Self::OwnBrowser {
                warning: if presented.sends_gpc_by_default() {
                    None
                } else {
                    Some(GpcWarning::Consistency { browser: presented })
                },
            },
        }
    }

    /// Resolves `consent.gpc` from a Session's `own` settings over its Run's
    /// `run` settings, its `source`, and whether it runs on a Borrowed
    /// browser.
    pub(crate) fn of(
        own: &Values,
        run: Option<&Resolved>,
        source: Option<&SessionSource>,
        borrowed: bool,
    ) -> Self {
        Self::resolve(
            GpcMode::of(own, run),
            PresentedBrowser::of_source(source),
            borrowed,
        )
    }

    /// Whether this Session needs its own launched Chromium process with
    /// `GlobalPrivacyControlTest` and `Browser.setGlobalPrivacyControl`.
    pub(crate) const fn own_browser_gpc(self) -> bool {
        matches!(self, Self::OwnBrowser { .. })
    }

    /// Whether interception step 7 injects `Sec-GPC: 1` (Borrowed browser
    /// with `consent.gpc: on`).
    pub(crate) const fn header_only(self) -> bool {
        matches!(self, Self::HeaderOnly { .. })
    }

    /// The Run report warning to record when the Session opens, if any.
    pub(crate) const fn warning(self) -> Option<GpcWarning> {
        match self {
            Self::Off => None,
            Self::OwnBrowser { warning } => warning,
            Self::HeaderOnly { warning } => Some(warning),
        }
    }
}

#[cfg(test)]
mod tests;
