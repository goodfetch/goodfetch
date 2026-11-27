//! User-configured extra Filter lists (`filter_lists.extra`, ADR 0047):
//! fetched from URLs (checked as Operator-configured endpoints when set in
//! Operator policy, user or project configuration, and as page-reachable
//! addresses when set in a Recipe or Run) or read from local files, cached in
//! the Browser cache under their content digest, and compiled once per digest
//! beside the Run's base Clean-up rules. Their scriptlets run only when set in
//! Operator policy, user or project configuration (ADRs 0028, 0047).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::HeldArtifact;
use goodfetch_engine::egress::{ConfiguredKind, EgressClass, Policy};
use goodfetch_engine::rule_sources::{self, SourceFailure};
use goodfetch_engine::runs::RulesContext;
use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_protocol::browser_cache::{
    ARTIFACT_DOWNLOAD_FAILED, ARTIFACT_UNAVAILABLE_OFFLINE, ArtifactDownloadFailedDetails,
    ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails, DownloadFailure, PinnedBy,
};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::settings::{
    CLEAN_UP_FETCH_TIMEOUT, FILTER_LISTS_EXTRA, SETTING_INVALID, SettingInvalidDetails,
    SettingLimit, SettingsLayer,
};
use serde_json::json;

use super::{FILE_LIMIT, Sources, within};
use crate::clean_up_versions::failures::Unobtained;

/// What a Run's `filter_lists.extra` asks for when non-empty.
#[derive(Clone, Debug)]
pub(super) struct ExtraAsked {
    pub(super) entries: Vec<String>,
    pub(super) layer: SettingsLayer,
    pub(super) timeout: Duration,
}

/// The extra Filter lists fetched or read for one Run, kept in the Browser
/// cache while the Run lives.
#[derive(Clone)]
pub(super) struct FetchedExtra {
    pub(super) digest: String,
    pub(super) entries: Vec<String>,
    pub(super) texts: Arc<Vec<String>>,
    pub(super) layer: SettingsLayer,
    pub(super) _artifact: Option<Arc<HeldArtifact>>,
}

/// Whether `filter_lists.extra` set at `layer` may contribute `+js(...)`
/// scriptlets (ADR 0047: Operator policy, user or project configuration only;
/// never Recipe, Run, Session or agent surfaces).
pub(crate) const fn scriptlets_allowed(layer: SettingsLayer) -> bool {
    matches!(
        layer,
        SettingsLayer::OperatorPolicy
            | SettingsLayer::UserConfiguration
            | SettingsLayer::ProjectConfiguration
    )
}

/// Reads `filter_lists.extra` from resolved `settings`; returns `None` when
/// unset or empty.
pub(super) fn asked(settings: Option<&Resolved>) -> Option<ExtraAsked> {
    let effective = settings?.effective(&FILTER_LISTS_EXTRA)?;
    let entries: Vec<String> = effective
        .value
        .as_array()?
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(ExtraAsked {
        entries,
        layer: effective.layer,
        timeout: get_or_default(settings, &CLEAN_UP_FETCH_TIMEOUT),
    })
}

/// Fetches or reads every entry of `asked` and keeps them in the Browser cache.
///
/// # Errors
///
/// `egress_refused` when a Recipe/Run URL is refused by egress,
/// `artifact_unavailable_offline` when `egress.offline` blocks a URL,
/// `artifact_download_failed` when a URL fetch fails or times out, or
/// `setting_invalid` when a local path or scheme is invalid or unreadable.
pub(super) async fn fetch(
    asked: &ExtraAsked,
    settings: Option<&Resolved>,
    sources: &Sources,
    context: &RulesContext<'_>,
    now: jiff::Timestamp,
) -> Result<FetchedExtra, CallError> {
    let operator = ConfiguredKind::CleanUpRulesSource
        .classed()
        .class(asked.layer)
        == EgressClass::OperatorConfigured;
    let page_policy = if operator {
        None
    } else {
        Some(page_policy_of(settings, asked.layer)?)
    };
    let mut texts = Vec::with_capacity(asked.entries.len());
    let mut files = Vec::with_capacity(asked.entries.len());
    for (index, entry) in asked.entries.iter().enumerate() {
        let text = if entry.starts_with("https://") || entry.starts_with("http://") {
            fetch_url(entry, asked, sources, page_policy.as_ref(), context).await?
        } else if entry.contains("://") {
            return Err(invalid_entry(
                asked.layer,
                entry,
                "uses an unsupported URL scheme (expected `https://`, `http://`, or a file path)",
            ));
        } else {
            read_file(entry, asked.layer, sources).await?
        };
        files.push((format!("extra-{index}.txt"), text.as_bytes().to_vec()));
        texts.push(text);
    }
    let (digest, artifact) = sources
        .kept
        .keep_extra(files, now)
        .await
        .map_err(|failure| {
            download_failed(
                asked,
                "local",
                DownloadFailure::Network,
                &failure.to_string(),
                context,
            )
        })?;
    Ok(FetchedExtra {
        digest,
        entries: asked.entries.clone(),
        texts: Arc::new(texts),
        layer: asked.layer,
        _artifact: Some(Arc::new(artifact)),
    })
}

async fn fetch_url(
    url: &str,
    asked: &ExtraAsked,
    sources: &Sources,
    page_policy: Option<&Policy>,
    context: &RulesContext<'_>,
) -> Result<String, CallError> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_default();
    let fetched = within(context.clock, asked.timeout, async {
        match rule_sources::get_extra(url, &sources.fetching, page_policy, FILE_LIMIT).await {
            Ok(answer) => Ok(Ok(answer)),
            Err(SourceFailure::Refused(refused)) => Ok(Err(refused.error())),
            Err(failure) => Err(Unobtained::of(failure, &sources.fetching)),
        }
    })
    .await;
    let answer = match fetched {
        Ok(Ok(answer)) => answer,
        Ok(Err(refused)) => return Err(refused),
        Err(Unobtained::Offline { layer }) => {
            return Err(offline_refusal(url, layer, context));
        }
        Err(failure) => {
            return Err(download_failed(
                asked,
                &host,
                DownloadFailure::Network,
                &format!(
                    "`{}` could not fetch `{url}`: {failure}",
                    FILTER_LISTS_EXTRA.name
                ),
                context,
            ));
        }
    };
    if answer.status != 200 {
        return Err(download_failed(
            asked,
            &host,
            DownloadFailure::HttpStatus {
                status: answer.status,
            },
            &format!(
                "`{}` URL `{url}` answered HTTP {}",
                FILTER_LISTS_EXTRA.name, answer.status
            ),
            context,
        ));
    }
    String::from_utf8(answer.body)
        .map_err(|_| invalid_entry(asked.layer, url, "is not valid UTF-8 Filter-list text"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "reads a configured extra Filter-list file on the tasks module's blocking policy"
)]
async fn read_file(
    path: &str,
    layer: SettingsLayer,
    sources: &Sources,
) -> Result<String, CallError> {
    let owned = PathBuf::from(path);
    let read = sources
        .kept
        .owner
        .blocking(move || std::fs::read(&owned))
        .await;
    let bytes =
        read.map_err(|error| invalid_entry(layer, path, &format!("could not be read: {error}")))?;
    if bytes.len() > FILE_LIMIT {
        return Err(invalid_entry(
            layer,
            path,
            &format!("exceeds the {FILE_LIMIT}-byte Filter-list limit"),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| invalid_entry(layer, path, "is not valid UTF-8 Filter-list text"))
}

fn page_policy_of(settings: Option<&Resolved>, layer: SettingsLayer) -> Result<Policy, CallError> {
    match settings {
        Some(settings) => Policy::of(settings),
        None => Policy::new(&["*"], &[] as &[&str]),
    }
    .map_err(|invalid| {
        CallError::new(
            &SETTING_INVALID,
            format!(
                "`{}` holds `{}`, which is no host",
                invalid.setting, invalid.entry
            ),
            &SettingInvalidDetails {
                key: invalid.setting.to_owned(),
                layer: Some(layer),
                reason: format!("`{}` is no host", invalid.entry),
                limit: None,
                fix: None,
            },
        )
    })
}

fn invalid_entry(layer: SettingsLayer, entry: &str, reason: &str) -> CallError {
    CallError::new(
        &SETTING_INVALID,
        format!("`{}` entry `{entry}` {reason}", FILTER_LISTS_EXTRA.name),
        &SettingInvalidDetails {
            key: FILTER_LISTS_EXTRA.name.to_owned(),
            layer: Some(layer),
            reason: reason.to_owned(),
            limit: None,
            fix: None,
        },
    )
}

fn offline_refusal(url: &str, layer: SettingsLayer, context: &RulesContext<'_>) -> CallError {
    CallError::new(
        &ARTIFACT_UNAVAILABLE_OFFLINE,
        format!(
            "`{}` URL `{url}` cannot be fetched while `{}` is on",
            FILTER_LISTS_EXTRA.name, EGRESS_OFFLINE.name
        ),
        &ArtifactUnavailableOfflineDetails {
            artifact: ArtifactId {
                kind: ArtifactKind::CleanUpRules,
                name: "extra".to_owned(),
                build: url.to_owned(),
            },
            pinned_by: PinnedBy {
                product: context.product,
                product_version: context.product_version.clone(),
            }
            .pinners(),
            cache: context.home.root().display().to_string(),
            limit: SettingLimit {
                key: EGRESS_OFFLINE.name.to_owned(),
                value: json!(true),
                layer,
                origin: None,
            },
            fix: "Lift `egress.offline`, or pass a local file path in `filter_lists.extra`."
                .to_owned(),
        },
    )
}

fn download_failed(
    asked: &ExtraAsked,
    host: &str,
    failure: DownloadFailure,
    message: &str,
    context: &RulesContext<'_>,
) -> CallError {
    CallError::new(
        &ARTIFACT_DOWNLOAD_FAILED,
        message.to_owned(),
        &ArtifactDownloadFailedDetails {
            artifact: ArtifactId {
                kind: ArtifactKind::CleanUpRules,
                name: "extra".to_owned(),
                build: asked.entries.join(","),
            },
            pinned_by: PinnedBy {
                product: context.product,
                product_version: context.product_version.clone(),
            }
            .pinners(),
            host: host.to_owned(),
            failure,
            fix: "Point `filter_lists.extra` at a reachable HTTP(S) URL or a local file path, or leave it empty.".to_owned(),
        },
    )
}
