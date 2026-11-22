//! The Clean-up rules' manifest (ADR 0047): the versioned format that names
//! one version of the Clean-up rules, its source, its date, its expiry and
//! the Filter list files it holds. A version is identified by the digest of
//! its files, the manifest among them, by the Browser cache's tree rule
//! (ADR 0024), which a Run records in its state and its Provenance names. It
//! is read under ADR 0030's newest-readable rule: an engine skips a version
//! whose manifest it cannot read and uses the newest one it can.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::ProtocolVersion;

/// The manifest's file name in a version's files.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Where a version of the Clean-up rules comes from (ADR 0047).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RulesSource {
    /// The part every executable embeds.
    Embedded,
    /// The full version a release pins by digest.
    Release,
    /// The opt-in daily channel.
    Daily,
    /// The Filter lists fetched from their authors' URLs.
    Upstream,
}

impl RulesSource {
    /// Every value, in declaration order.
    pub const ALL: [Self; 4] = [Self::Embedded, Self::Release, Self::Daily, Self::Upstream];

    /// Its name on the wire and on disk.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// One version of the Clean-up rules, as its `manifest.json` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CleanUpRulesManifest {
    /// The version of this format the manifest was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format_version: ProtocolVersion,
    /// Where the version comes from.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: RulesSource,
    /// When its lists were taken from their authors, in RFC 3339: the date
    /// Provenance names, from which a Run report counts its 14 days.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub published_at: SystemTime,
    /// When a newer version should replace it, in RFC 3339: a daily
    /// version's 14 days, an upstream one's shortest declared expiry; none
    /// for the embedded part and a release's.
    #[serde(with = "crate::point_in_time::nullable", default)]
    #[schemars(with = "Option<String>", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub expires_at: Option<SystemTime>,
    /// The file of each Filter list, beside the manifest.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lists: RulesLists,
}

/// The file of each Filter list a version holds, by the setting that turns
/// it on, with Brave's unbreak list, which applies whichever are on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RulesLists {
    /// `filter_lists.ads`: EasyList.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ads: String,
    /// `filter_lists.trackers`: EasyPrivacy.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub trackers: String,
    /// `filter_lists.annoyances`: Fanboy Annoyance, without its cookie
    /// sections.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub annoyances: String,
    /// Brave's unbreak list.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unbreak: String,
}

impl RulesLists {
    /// Each file, with the list it is.
    pub fn files(&self) -> [(&'static str, &str); 4] {
        [
            ("ads", &self.ads),
            ("trackers", &self.trackers),
            ("annoyances", &self.annoyances),
            ("unbreak", &self.unbreak),
        ]
    }
}
