//! Loader for published benchmark governor default ceilings (`GovernorCeilings`
//! emitted by `bench/src/load.ts`, ADRs 0025 and 0040, issue 178).

use std::path::Path;

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::governor::WorkKind;
use serde::Deserialize;

/// Per-work-kind ceilings section of a published `governor-ceilings.json` artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedWorkKindCeilings {
    pub http: Option<u32>,
    pub browser_page: Option<u32>,
    pub model_call: Option<u32>,
}

/// Published default ceilings produced by `bench` concurrency search (ADR 0040).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PublishedGovernorCeilings {
    pub schema_version: u32,
    pub product: String,
    pub family: String,
    pub work_kind: WorkKind,
    pub setting: String,
    pub default_ceiling: u32,
    pub ceilings: PublishedWorkKindCeilings,
}

/// Errors returned when loading a published `GovernorCeilings` JSON document.
#[derive(Debug, thiserror::Error)]
pub enum PublishedCeilingsError {
    #[error("reading published governor ceilings: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing published governor ceilings JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported governor ceilings schema_version {0} (expected 1)")]
    UnsupportedVersion(u32),
    #[error("governor ceiling for {0} must be >= 1")]
    ZeroCeiling(&'static str),
    #[error("setting `{setting}` does not match work_kind `{expected}`")]
    SettingMismatch {
        setting: String,
        expected: &'static str,
    },
}

impl PublishedGovernorCeilings {
    /// Parses and validates a published `governor-ceilings.json` payload.
    pub fn parse(json: &str) -> Result<Self, PublishedCeilingsError> {
        let parsed: Self = serde_json::from_str(json)?;
        if parsed.schema_version != 1 {
            return Err(PublishedCeilingsError::UnsupportedVersion(
                parsed.schema_version,
            ));
        }
        if parsed.default_ceiling == 0 {
            return Err(PublishedCeilingsError::ZeroCeiling(
                parsed.work_kind.as_str(),
            ));
        }
        let expected_setting = parsed.work_kind.ceiling().name;
        if parsed.setting != expected_setting {
            return Err(PublishedCeilingsError::SettingMismatch {
                setting: parsed.setting,
                expected: expected_setting,
            });
        }
        for (kind, value) in [
            (WorkKind::Http, parsed.ceilings.http),
            (WorkKind::BrowserPage, parsed.ceilings.browser_page),
            (WorkKind::ModelCall, parsed.ceilings.model_call),
        ] {
            if value == Some(0) {
                return Err(PublishedCeilingsError::ZeroCeiling(kind.as_str()));
            }
        }
        Ok(parsed)
    }

    /// Reads and validates a published `governor-ceilings.json` file from disk.
    #[expect(
        clippy::disallowed_methods,
        reason = "loads a small startup configuration artifact before or outside async request paths"
    )]
    pub fn read_file(path: &Path) -> Result<Self, PublishedCeilingsError> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text)
    }

    /// Returns the published default ceiling for `kind`, if present in this document.
    pub fn ceiling_for(&self, kind: WorkKind) -> Option<u32> {
        match kind {
            WorkKind::Http => self.ceilings.http,
            WorkKind::BrowserPage => self.ceilings.browser_page,
            WorkKind::ModelCall => self.ceilings.model_call,
        }
        .or_else(|| (self.work_kind == kind).then_some(self.default_ceiling))
    }

    /// Returns the Engine runtime setting overrides (`governor.<kind>.ceiling`) carried by this artifact.
    pub fn setting_overrides(&self) -> Vec<(&'static Setting, u64)> {
        WorkKind::ALL
            .into_iter()
            .filter_map(|kind| {
                self.ceiling_for(kind)
                    .map(|ceiling| (kind.ceiling(), u64::from(ceiling)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schema_version": 1,
        "product": "foxhound",
        "family": "http-crawl",
        "work_kind": "http",
        "setting": "governor.http.ceiling",
        "default_ceiling": 64,
        "ceilings": {
            "http": 64,
            "browser_page": null,
            "model_call": null
        },
        "peak_verified_tasks_per_second": 420.5,
        "stop_reason": "throughput_plateau",
        "stopped_at_concurrency": 256,
        "host_class": "local-controlled",
        "measured_at": "2026-11-28T00:00:00.000Z",
        "provenance_commit": "abc1234",
        "levels": []
    }"#;

    #[test]
    fn parses_published_governor_ceilings_and_maps_to_work_kind_settings() {
        let doc = PublishedGovernorCeilings::parse(SAMPLE).expect("valid governor ceilings");
        assert_eq!(doc.ceiling_for(WorkKind::Http), Some(64));
        assert_eq!(doc.ceiling_for(WorkKind::BrowserPage), None);
        assert_eq!(doc.ceiling_for(WorkKind::ModelCall), None);
        let overrides = doc.setting_overrides();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].0.name, "governor.http.ceiling");
        assert_eq!(overrides[0].1, 64);
    }

    #[test]
    fn rejects_mismatched_setting_and_zero_ceiling() {
        let bad_setting = SAMPLE.replace("governor.http.ceiling", "governor.browser_page.ceiling");
        assert!(matches!(
            PublishedGovernorCeilings::parse(&bad_setting),
            Err(PublishedCeilingsError::SettingMismatch { .. })
        ));

        let zero = SAMPLE.replace("\"default_ceiling\": 64", "\"default_ceiling\": 0");
        assert!(matches!(
            PublishedGovernorCeilings::parse(&zero),
            Err(PublishedCeilingsError::ZeroCeiling("http"))
        ));
    }
}
