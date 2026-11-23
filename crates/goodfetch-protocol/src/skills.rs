//! The agent skills an executable carries (ADRs 0065, 0095 and 0097): what
//! `skill install` prints and its refusal.
//!
//! `skill install` writes the skills its executable embeds into a host's
//! skills folder, at project or user scope, each in a folder of its name,
//! and replaces only a skill it installed itself and nobody changed since.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `skill install` refused a skill's folder (story:11-237).
pub const SKILL_INSTALL_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "skill_install_refused",
    description: "`skill install` found a folder of a skill's name in the skills folder that it did not install, or one it installed that was changed since, and refused to replace it. The message names the folder; no skill is written.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The coding agent whose skills folder `skill install` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillHost {
    /// Claude Code, which reads `.claude/skills`.
    ClaudeCode,
    /// Codex, which reads `.agents/skills`.
    Codex,
    /// Cursor, which reads `.cursor/skills` among others.
    Cursor,
    /// Any host of the Agent Skills specification that reads
    /// `.agents/skills`.
    Agents,
}

/// Where the skills folder `skill install` writes is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    /// In the working folder, for the project the host opens there.
    Project,
    /// In the user's home folder, for every project.
    User,
}

/// What `skill install --json` prints (spec #608): every skill the
/// executable carries, as the skills folder now holds it, whether this run
/// wrote it or found it already there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillsInstalled {
    /// The host whose skills folder it wrote.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: SkillHost,
    /// Where that folder is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: SkillScope,
    /// Each skill, by name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub installed: Vec<InstalledSkill>,
}

/// One skill `skill install` made sure of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledSkill {
    /// Its name, such as `goodfetch-collie`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its folder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// The digest of its files, `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[SKILL_INSTALL_REFUSED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
