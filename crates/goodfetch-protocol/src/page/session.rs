//! A Run's Sessions (ADRs 0021, 0078 and 0092): `run.new_session` opens a
//! Session of its own, its own browser context with its own cookies and
//! storage, sharing the Run's browser process; `session.new_page` opens a
//! page in it; and `session.close` closes its pages and its context and
//! gives its pages' admissions back to the Resource governor, never closing
//! a Borrowed browser. A call on a closed Session raises `closed`, and a
//! second `session.close` changes nothing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{ARRIVED_IN, PageOpened, SessionId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, Reach, SharedType,
    SharedTypeValue, Stability, schema_of,
};
use crate::run::RunId;
use crate::settings::session_source::SessionSource;

/// What `run.new_session` takes: the Run whose Session it opens, the
/// Session's own source and its own settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewSessionParams {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// `session.source` for this Session alone: left out, the Run's or the
    /// connection's applies; `null`, the Session opens fresh (ADRs 0083
    /// and 0092).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: Option<Option<SessionSource>>,
    /// The Session's own settings, nested by key: `filter_lists.ads`,
    /// `.trackers`, `.annoyances`, `.first_party_scripts`,
    /// `browser.block_resources` and `live_view.hiding` choose its pages'
    /// Clean-up rules over the Run's, and `browser.service_workers` whether
    /// a site's service workers run in them, and `session.page_check` the
    /// Page check its pages are judged against, and `content.extractor`
    /// the main-content step their formats read with; `browser.access_mode`
    /// and the `stealth.*` items its Access mode and Identity profile, and
    /// the `proxy.*` keys its proxy, which it keeps for its life.
    /// `session.source` is refused here with `setting_not_applicable`,
    /// as the call's own `source` sets it, and so is any other key, which
    /// only the Run sets.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
}

/// What `run.new_session` answers: the Session it opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionOpened {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
}

/// What `session.close` takes: the Session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionParams {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
}

/// What `session.close` answers once the Session is closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionClosed {}

/// `run.new_session`.
pub const RUN_NEW_SESSION: Operation = Operation {
    name: "run.new_session",
    description: "Opens a Session of the Run's own: its own browser context, with its own cookies, storage, identity and proxy, sharing the Run's browser process, so two Sessions of a Run never share a login. It opens from `source`, or else from the Run's `session.source`, or fresh; `session.source` in its own `settings` is refused with `setting_not_applicable`. The Run's first Session launches its browser, so a missing browser build raises its code from this call.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<NewSessionParams>,
    result: schema_of::<SessionOpened>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `session.new_page` takes: the Session, and the page's own
/// settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewSessionPageParams {
    /// The Session.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// The page's own settings, nested by key (ADR 0083), which it takes
    /// over its Session's and its Run's: `content.extractor`, the
    /// main-content step its formats read with. Any other key is refused
    /// with `setting_not_applicable`, as only its Session or its Run sets
    /// it; the Session's Access mode, Identity profile and proxy only its
    /// opening sets.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
}

/// `session.new_page`.
pub const SESSION_NEW_PAGE: Operation = Operation {
    name: "session.new_page",
    description: "Opens a page in the Session, with its cookies and storage, and its own settings. Once the Session is closed, it raises `closed`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<NewSessionPageParams>,
    result: schema_of::<PageOpened>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `session.close`.
pub const SESSION_CLOSE: Operation = Operation {
    name: "session.close",
    description: "Closes the Session: its pages and its browser context close, and its pages' admissions go back to the Resource governor; a Borrowed browser is never closed. A later call on the Session or one of its pages raises `closed`, and closing it again changes nothing.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SessionParams>,
    result: schema_of::<SessionClosed>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `session.save_identity` takes: the Session and the name under which
/// to save its cookies and storage (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveIdentityParams {
    /// The Session whose login is saved.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: SessionId,
    /// The Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What `session.save_identity` answers once the Saved identity is written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdentitySaved {
    /// The Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// `session.save_identity`.
pub const SESSION_SAVE_IDENTITY: Operation = Operation {
    name: "session.save_identity",
    description: "Saves the Session's cookies and storage as a named Saved identity (ADR 0092). Refused with `session_not_savable` on a Profile directory used in place, naming the import that would work.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SaveIdentityParams>,
    result: schema_of::<IdentitySaved>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `profile_directory_in_use` details (ADR 0056; ticket #737).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileDirectoryInUseDetails {
    /// The Profile directory path that could not be opened.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Why it is refused (active lock file or Chrome's default user data directory).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// Suggested alternative or action (e.g. `running_browser` or closing the running browser).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `profile_directory_browser_mismatch` details (ADR 0056; ticket #737).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileDirectoryBrowserMismatchDetails {
    /// The Profile directory path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// The browser family requested in `session.source`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expected: String,
    /// The browser family detected in the directory.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub found: String,
}

/// What `profile_directory_newer_than_browser` details (ADR 0056; ticket #737).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileDirectoryNewerThanBrowserDetails {
    /// The Profile directory path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// The version recorded in the Profile directory.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub profile_version: String,
    /// The browser version the engine would launch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_version: String,
}

/// What `session_not_savable` details (ADR 0092; ticket #737).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionNotSavableDetails {
    /// The Session source kind that cannot be saved directly.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: String,
    /// The import kind that works instead (e.g. `state_file` or `installed_browser`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub alternative: String,
}

/// What `remote_debugging_disabled_by_policy` details (ADR 0056; ticket #742).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoteDebuggingDisabledByPolicyDetails {
    /// The browser family whose administrator policy disabled remote debugging.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser: String,
    /// The managed policy file that disabled remote debugging, when found on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy_path: Option<String>,
    /// The policy key that disabled remote debugging (for example `RemoteDebuggingAllowed`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy_key: String,
    /// Suggested alternative or administrator action.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `running_browser_unsupported` details (ADR 0056; ticket #742).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunningBrowserUnsupportedDetails {
    /// The browser family requested in `session.source`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser: String,
    /// Why this running browser cannot be borrowed via consented remote debugging.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// Suggested alternative (for example `installed_browser`, `profile_directory`, or `state_file`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// `profile_directory_in_use`.
pub const PROFILE_DIRECTORY_IN_USE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "profile_directory_in_use",
    description: "The Profile directory holds an active browser lock (`SingletonLock`, `lockfile`, `parent.lock`, `.parentlock`) or is a browser's default user data directory, which must be reached via `running_browser` instead (ADR 0056).",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `profile_directory_browser_mismatch`.
pub const PROFILE_DIRECTORY_BROWSER_MISMATCH: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "profile_directory_browser_mismatch",
    description: "The Profile directory was written by a different browser family than `session.source.browser` names, so launching on it would corrupt it (ADR 0056).",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `profile_directory_newer_than_browser`.
pub const PROFILE_DIRECTORY_NEWER_THAN_BROWSER: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "profile_directory_newer_than_browser",
    description: "The Profile directory was last written by a newer browser version than the engine would launch, so opening it in place would downgrade and corrupt its databases (ADR 0056).",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `session_not_savable`.
pub const SESSION_NOT_SAVABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "session_not_savable",
    description: "This Session's source cannot be saved as a Saved identity directly (for example, a Profile directory used in place or a Borrowed browser); import via the named alternative instead (ADRs 0056 and 0092).",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `remote_debugging_disabled_by_policy`.
pub const REMOTE_DEBUGGING_DISABLED_BY_POLICY: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "remote_debugging_disabled_by_policy",
    description: "An administrator policy (such as `RemoteDebuggingAllowed = false` or `DeveloperToolsAvailability = 2`) has disabled consented remote debugging on the named running browser (ADR 0056).",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `running_browser_unsupported`.
pub const RUNNING_BROWSER_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "running_browser_unsupported",
    description: "The named running browser does not implement consented remote debugging (for example Firefox, Safari, or Brave); use Chrome or Edge, or import cookies via `installed_browser` or `state_file` (ADR 0056).",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `domain_not_allowed` details (ADRs 0056 and 0092; tickets #745 and #746).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomainNotAllowedDetails {
    /// The registrable domain that was outside the Session source's allowed `domains`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// The registrable domains allowed by the Session's source.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub allowed: Vec<String>,
    /// The `session.source` kind that bound the Session (`running_browser`,
    /// `cdp_url`, `profile_directory`, etc.).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: String,
    /// The target URL when a navigation was refused; absent for cookie operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// Suggested fix.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// `domain_not_allowed`.
pub const DOMAIN_NOT_ALLOWED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "domain_not_allowed",
    description: "A top-level navigation, new tab, or cookie operation targeted a registrable domain outside the Session source's bound `domains` (ADRs 0056 and 0092).",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A login was imported from a Session source (`state_file`, `cookie_header`,
/// `installed_browser`, or `profile_directory`, ADR 0056).
pub const LOGIN_IMPORTED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "login_imported",
    description: "A login was imported from a Session source (`state_file`, `cookie_header`, `installed_browser`, or `profile_directory`) into a Session (ADR 0056).",
    arrived_in: ARRIVED_IN,
};

/// A running or remote browser login was borrowed (`running_browser`, `cdp_url`,
/// or `client_browser`, ADR 0056).
pub const LOGIN_BORROWED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "login_borrowed",
    description: "A browser login was borrowed (`running_browser`, `cdp_url`, or `client_browser`) for a Session (ADR 0056).",
    arrived_in: ARRIVED_IN,
};
