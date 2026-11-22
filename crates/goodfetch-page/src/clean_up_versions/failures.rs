//! Why a version of the Clean-up rules could not be had (ADR 0047): each
//! failure's stable name, which Provenance records as a fallback's reason,
//! and the refusal a start answers when `clean_up.version` names a version
//! the engine cannot have, in the Browser cache's download codes (ADR 0115).

use std::time::Duration;

use goodfetch_clean_up_rules::rules::EMBEDDED_DIGEST;
use goodfetch_engine::CallError;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::response_limits::LimitExceeded;
use goodfetch_engine::rule_sources::SourceFailure;
use goodfetch_engine::runs::{Fallback, RulesContext};
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_protocol::browser_cache::{
    ARTIFACT_DOWNLOAD_FAILED, ARTIFACT_UNAVAILABLE_OFFLINE, ArtifactDownloadFailedDetails,
    ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails, DownloadFailure, PinnedBy,
};
use goodfetch_protocol::clean_up_rules::RulesSource;
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::settings::{
    CLEAN_UP_CHANNEL, CLEAN_UP_VERSION, SETTING_INVALID, SettingInvalidDetails, SettingLimit,
    SettingsLayer,
};
use serde_json::json;

/// Why a version of the Clean-up rules could not be had. Each has a stable
/// name, which Provenance records as the reason of a fallback.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum Unobtained {
    /// `clean_up.version` holds no `sha256:<hex>` digest.
    #[error("is no digest `sha256:<hex>`")]
    NotADigest,
    /// Not on this machine, and no source of the Run fetches it.
    #[error("is not on this machine, and no source the Run names fetches it")]
    NotHere,
    /// `egress.offline`, set by `layer`, refused the fetch.
    #[error("cannot be fetched: `egress.offline` turns the fetching of the Clean-up rules off")]
    Offline {
        /// The layer that set `egress.offline`.
        layer: SettingsLayer,
    },
    /// The fetches took longer than `clean_up.fetch_timeout`.
    #[error("was not fetched within `clean_up.fetch_timeout`, {} ms", .0.as_millis())]
    TimedOut(Duration),
    /// A source did not answer, broke the connection or redirected too
    /// often.
    #[error("cannot be fetched: {host} did not answer")]
    Unreachable {
        /// The source's host.
        host: String,
    },
    /// A source answered another status than `200 OK`.
    #[error("cannot be fetched: {host} answered {status}")]
    Refused {
        /// The source's host.
        host: String,
        /// Its status.
        status: u16,
    },
    /// A file the daily channel served is not the bytes its digest names.
    #[error("cannot be fetched: its file `{file}` from {host} is not the bytes its digest names")]
    Tampered {
        /// The channel's host.
        host: String,
        /// The file.
        file: String,
    },
    /// The daily channel served the digest's tag as files of another
    /// digest.
    #[error("is served by {host} as files of another digest, {found}")]
    OtherDigest {
        /// The channel's host.
        host: String,
        /// The digest of the files it served.
        found: String,
    },
    /// What a source gave is no version this engine reads.
    #[error("{0}")]
    Unreadable(String),
    /// What a source gave passed a limit on what the engine reads.
    #[error("cannot be fetched: {0}")]
    PastALimit(LimitExceeded),
    /// The Browser cache did not keep it.
    #[error("cannot be kept in the Browser cache: {0}")]
    NotKept(String),
    /// Its files in the Browser cache did not read.
    #[error("cannot be read from the Browser cache: {0}")]
    NotLoaded(String),
}

impl Unobtained {
    /// Its stable name, which Provenance records.
    pub(crate) fn reason(&self) -> &'static str {
        match self {
            Self::NotADigest => "not_a_digest",
            Self::NotHere => "not_here",
            Self::Offline { .. } => "offline",
            Self::TimedOut(_) => "timed_out",
            Self::Unreachable { .. } => "unreachable",
            Self::Refused { .. } => "refused",
            Self::Tampered { .. } => "tampered",
            Self::OtherDigest { .. } => "other_digest",
            Self::Unreadable(_) => "unreadable",
            Self::PastALimit(_) => "past_a_limit",
            Self::NotKept(_) => "not_kept",
            Self::NotLoaded(_) => "not_loaded",
        }
    }

    /// `failure`, a source's, under `fetching`, which says who set
    /// `egress.offline`.
    pub(crate) fn of(failure: SourceFailure, fetching: &Fetching) -> Self {
        match failure {
            SourceFailure::Offline => Self::Offline {
                layer: fetching.offline.unwrap_or(SettingsLayer::Default),
            },
            SourceFailure::Network { host } | SourceFailure::Redirects { host } => {
                Self::Unreachable { host }
            }
            other => Self::Unreadable(other.to_string()),
        }
    }

    /// The source's host it names, if any.
    fn host(&self) -> Option<&str> {
        match self {
            Self::Unreachable { host }
            | Self::Refused { host, .. }
            | Self::Tampered { host, .. }
            | Self::OtherDigest { host, .. } => Some(host),
            _ => None,
        }
    }
}

/// The fallback a Run holds to the embedded part for, as `failure` keeps
/// what it asked for from it; `what` names that.
pub(super) fn fallback(what: &str, failure: &Unobtained) -> Fallback {
    Fallback {
        reason: failure.reason(),
        message: format!(
            "{what} {failure}: the Run holds to the embedded part, {EMBEDDED_DIGEST}, instead"
        ),
    }
}

/// The refusal of a start whose `clean_up.version`, `digest`, set by
/// `layer`, the engine cannot have for `failure`, under `context`.
pub(super) fn refusal(
    (digest, layer): (&str, SettingsLayer),
    failure: &Unobtained,
    context: &RulesContext<'_>,
) -> CallError {
    let message = format!(
        "`{}` names {digest}, which {failure}",
        CLEAN_UP_VERSION.name
    );
    let artifact = ArtifactId {
        kind: ArtifactKind::CleanUpRules,
        name: RulesSource::Daily.as_str().to_owned(),
        build: digest
            .parse::<Digest>()
            .map_or_else(|_| digest.to_owned(), |digest| digest.hex()),
    };
    let pinned_by = PinnedBy {
        product: context.product,
        product_version: context.product_version.clone(),
    };
    let host = failure
        .host()
        .map_or_else(|| registry(context), str::to_owned);
    let download = |failure: DownloadFailure| {
        CallError::new(
            &ARTIFACT_DOWNLOAD_FAILED,
            message.clone(),
            &ArtifactDownloadFailedDetails {
                artifact: artifact.clone(),
                pinned_by: pinned_by.clone(),
                host: host.clone(),
                failure,
                fix: "Pin a version the daily channel serves, or one this machine holds, or leave `clean_up.version` unset.".to_owned(),
            },
        )
    };
    match failure {
        Unobtained::PastALimit(limit) => limit.error(),
        Unobtained::NotADigest => CallError::new(
            &SETTING_INVALID,
            message.clone(),
            &SettingInvalidDetails {
                key: CLEAN_UP_VERSION.name.to_owned(),
                layer: Some(layer),
                reason: "is no digest `sha256:<hex>`".to_owned(),
                limit: None,
                fix: None,
            },
        ),
        Unobtained::Offline { layer } => CallError::new(
            &ARTIFACT_UNAVAILABLE_OFFLINE,
            message.clone(),
            &ArtifactUnavailableOfflineDetails {
                artifact,
                pinned_by,
                cache: context.home.root().display().to_string(),
                limit: SettingLimit {
                    key: EGRESS_OFFLINE.name.to_owned(),
                    value: json!(true),
                    layer: *layer,
                    origin: None,
                },
                fix: "Lift `egress.offline` where it was set, or pin a version this machine holds."
                    .to_owned(),
            },
        ),
        Unobtained::Refused { status, .. } => {
            download(DownloadFailure::HttpStatus { status: *status })
        }
        Unobtained::Tampered { .. }
        | Unobtained::OtherDigest { .. }
        | Unobtained::Unreadable(_) => download(DownloadFailure::DigestMismatch),
        _ => download(DownloadFailure::Network),
    }
}

/// The daily channel's registry under `context`'s settings.
fn registry(context: &RulesContext<'_>) -> String {
    let channel = get_or_default::<&str>(
        context.settings.map(RunSettings::resolved),
        &CLEAN_UP_CHANNEL,
    );
    channel
        .split_once('/')
        .map_or(channel, |(registry, _)| registry)
        .to_owned()
}
