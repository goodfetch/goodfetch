//! The Browser cache's contracts (ADRs 0024, 0043, 0098 and 0115): the
//! Pinned artifacts a Product version names, the entry each one keeps in the
//! cache, the Error codes of a missing build, the download's progress event,
//! `browser_cache.max_unused_age`, and what `browser install` and `browsers
//! clean` print under `--json` (ADR 0069). Collie carries them, since Collie
//! launches browsers (ADR 0088).
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Product,
    ProtocolVersion, Setting, Stability, ValueKind, schema_of,
};
use crate::connection::ProductVersion;
use crate::settings::SettingLimit;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a Pinned artifact is (ADR 0098: the clean-up's rule covers every
/// kind). An entry of a kind the engine does not know is one it cannot read,
/// and keeps.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// A browser build, such as Chrome for Testing's.
    Browser,
    /// Playwright's driver (ADR 0019).
    Driver,
    /// The versioned copy of an executable a Daemon runs from (ADR 0024).
    ExecutableCopy,
    /// The engine executable a Product's Rust crate downloads (ADR 0051).
    EngineExecutable,
    /// A full Clean-up rules set (ADR 0047).
    CleanUpRules,
    /// Recorded mouse movements (ADR 0035).
    MouseMovements,
}

impl ArtifactKind {
    /// Its name on disk and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Driver => "driver",
            Self::ExecutableCopy => "executable_copy",
            Self::EngineExecutable => "engine_executable",
            Self::CleanUpRules => "clean_up_rules",
            Self::MouseMovements => "mouse_movements",
        }
    }
}

/// Which Pinned artifact: its kind, its name and its build.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactId {
    /// What it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: ArtifactKind,
    /// Its name, such as `chrome-for-testing`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its build ID, such as `141.0.7390.54-linux64`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub build: String,
}

/// The Product version that pins an artifact (ADR 0098).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PinnedBy {
    /// The Product.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
}

/// One Pinned artifact's entry in the Browser cache, `entry.json` beside its
/// files: what it is, the digest its files have, and when it was installed
/// and last used, which the clean-up reads (ADR 0098).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserCacheEntry {
    /// The version of this format the entry was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format_version: ProtocolVersion,
    /// Which artifact it holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub artifact: ArtifactId,
    /// The digest of its files, `sha256:<hex>`, which every launch checks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// When it was installed, in RFC 3339.
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub installed_at: jiff::Timestamp,
    /// When it was last launched or installed, in RFC 3339.
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub used_at: jiff::Timestamp,
}

/// A build the engine needs and cannot fetch (ADR 0115).
pub const BROWSER_NOT_INSTALLED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "browser_not_installed",
    description: "The browser build a launch needs is not in the Browser cache and the engine cannot write that cache, such as an image's read-only `GOODFETCH_BROWSERS_PATH`; or a browser named by channel or path is not there. Run the Product's `browser install` where the cache is writable, install the browser of that channel, or correct the path.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `browser_not_installed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserNotInstalledDetails {
    /// What was missing.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub missing: MissingBrowser,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A browser the engine looked for and did not find.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MissingBrowser {
    /// A pinned build absent from a cache the engine cannot write.
    Pinned {
        /// The build.
        #[schemars(extend("arrived_in" = "0.1"))]
        artifact: ArtifactId,
        /// The Product version that pins it.
        #[schemars(extend("arrived_in" = "0.1"))]
        pinned_by: PinnedBy,
        /// The Browser cache's path.
        #[schemars(extend("arrived_in" = "0.1"))]
        cache: String,
    },
    /// An installed Chrome named by its channel.
    Channel {
        /// The channel.
        #[schemars(extend("arrived_in" = "0.1"))]
        channel: ChromeChannel,
        /// Where the engine looked for it.
        #[schemars(extend("arrived_in" = "0.1"))]
        looked_in: Vec<String>,
    },
    /// A browser named by its path.
    Path {
        /// The path.
        #[schemars(extend("arrived_in" = "0.1"))]
        path: String,
    },
}

/// A channel of Google Chrome (ADR 0024: an installed Chrome can be named by
/// its channel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChromeChannel {
    /// Chrome.
    Stable,
    /// Chrome Beta.
    Beta,
    /// Chrome Dev.
    Dev,
    /// Chrome Canary.
    Canary,
}

/// A pinned download `offline` refused (ADRs 0043 and 0115).
pub const ARTIFACT_UNAVAILABLE_OFFLINE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "artifact_unavailable_offline",
    description: "The `offline` setting refused a pinned download the call needed. Run the Product's `browser install` beforehand where the cache is writable, point `GOODFETCH_BROWSERS_PATH` at a prepared cache, or lift `offline` where it was set.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `artifact_unavailable_offline` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactUnavailableOfflineDetails {
    /// The artifact.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub artifact: ArtifactId,
    /// The Product version that pins it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pinned_by: PinnedBy,
    /// The Browser cache's path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cache: String,
    /// The setting that refused it: `offline`, its value and its layer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A pinned download that failed (ADR 0115).
pub const ARTIFACT_DOWNLOAD_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "artifact_download_failed",
    description: "A pinned download the call needed failed: the network, an HTTP status, or bytes whose digest is not the pinned one. Retry, or install beforehand with the Product's `browser install`, or point `GOODFETCH_BROWSERS_PATH` at a prepared cache.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `artifact_download_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactDownloadFailedDetails {
    /// The artifact.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub artifact: ArtifactId,
    /// The Product version that pins it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pinned_by: PinnedBy,
    /// The host it came from, never a full URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// What failed.
    #[serde(flatten)]
    pub failure: DownloadFailure,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What failed in a pinned download.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "failure", rename_all = "snake_case")]
pub enum DownloadFailure {
    /// No answer, or a connection that broke.
    Network,
    /// An answer other than `200 OK`.
    HttpStatus {
        /// Its status.
        #[schemars(extend("arrived_in" = "0.1"))]
        status: u16,
    },
    /// Bytes whose digest is not the pinned one.
    DigestMismatch,
}

/// What `browser_cache.download_progress` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DownloadProgress {
    /// The artifact being downloaded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub artifact: ArtifactId,
    /// The host it comes from.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// The bytes received so far.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub received_bytes: u64,
    /// The bytes the answer announced, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub total_bytes: Option<u64>,
}

/// A first-use download's progress (ADR 0024).
pub const DOWNLOAD_PROGRESS: Event = Event {
    name: "browser_cache.download_progress",
    description: "A Pinned artifact the call needs is downloading into the Browser cache: the bytes received so far, out of those announced.",
    class: EventClass::Telemetry,
    payload: schema_of::<DownloadProgress>,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn thirty_days() -> serde_json::Value {
    serde_json::json!(30_u64 * 24 * 60 * 60 * 1000)
}

/// `browser_cache.max_unused_age` (ADR 0098).
pub const MAX_UNUSED_AGE: Setting = Setting {
    name: "browser_cache.max_unused_age",
    description: "How long a Pinned artifact no installed executable pins stays in the Browser cache unused before the clean-up deletes it, and how long an executable's entry counts as installed after it was last seen: 30 days. `null` turns the automatic clean-up off. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: thirty_days,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// What `browser install --json` prints (spec #608).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserInstalled {
    /// Each build the Product version pins for this platform, and what
    /// installing it did.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub builds: Vec<InstalledBuild>,
}

/// One build `browser install` made sure of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledBuild {
    /// Its name, such as `chrome-for-testing`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its build ID, such as `141.0.7390.54-linux64`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// Whether it was downloaded now or already there.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub outcome: InstallOutcome,
}

/// What installing an artifact did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallOutcome {
    /// It was downloaded and checked now.
    Installed,
    /// It was already there, with its pinned digest.
    AlreadyInstalled,
}

/// What `browser clean --json` prints (spec #608): what it deleted, or
/// would delete under `--dry-run`, which is what a `browser clean` right
/// after deletes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserCleaned {
    /// Whether it only listed what it would delete.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dry_run: bool,
    /// What it deleted, or would delete under `--dry-run`, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed: Vec<RemovedBuild>,
    /// The bytes those take on disk, freed unless `--dry-run`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bytes_freed: u64,
    /// Why it cleaned nothing, when it did not run; absent when it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub not_cleaned: Option<NotCleaned>,
}

/// One build `browser clean` deleted, or would delete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemovedBuild {
    /// Its name, such as `chrome-for-testing`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its build ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The bytes its files take on disk.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bytes: u64,
}

/// Why the clean-up did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotCleaned {
    /// The engine cannot write the cache, such as a read-only
    /// `GOODFETCH_BROWSERS_PATH` (ADR 0098).
    CacheNotWritable,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[DOWNLOAD_PROGRESS],
    handler_requests: &[],
    error_codes: &[
        BROWSER_NOT_INSTALLED,
        ARTIFACT_UNAVAILABLE_OFFLINE,
        ARTIFACT_DOWNLOAD_FAILED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[MAX_UNUSED_AGE],
    hook_stages: &[],
    shared_type_values: &[],
};
