//! The Plugin manifest (ADR 0033), `goodfetch-plugin.json` at the root of a
//! Plugin's bundle and its OCI config blob of media type [`MEDIA_TYPE`]:
//! what a Plugin is and may do, which the operator approves and the
//! engine holds its `hello` to. A field the manifest leaves out takes ADR
//! 0033's default. Its reader first takes its version with
//! [`PluginManifest::version_of`] and holds it to the `plugin-manifest`
//! format's rule (ADR 0066), as every reader of a format does, then
//! [`PluginManifest::read`] refuses a network level its kinds do not allow
//! or a loader variable in its environment.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::PluginProtocolRange;
use super::names::{HookStageName, PluginKindName};
use crate::catalog::ProtocolVersion;

/// The manifest's file name at the root of a Plugin's bundle.
pub const FILE_NAME: &str = "goodfetch-plugin.json";

/// The manifest's media type as an OCI config blob.
pub const MEDIA_TYPE: &str = "application/vnd.goodfetch.plugin.manifest.v1+json";

/// The manifest format's version, `major.minor` (ADRs 0030 and 0066).
pub const MANIFEST_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Plugin's manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// The version of the manifest format it is written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub manifest_version: ProtocolVersion,
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The Plugin's version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The Plugin kinds it implements.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: BTreeSet<PluginKindName>,
    /// The Hook stages it implements, for a Hook.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hook_stages: BTreeSet<HookStageName>,
    /// The plugin protocol versions it speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol: PluginProtocolRange,
    /// The JSON Schema of its settings, which the engine validates them
    /// against and the CLI and MCP take the fields of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings_schema: Option<serde_json::Value>,
    /// The language runtime it runs on, which the engine provides from its
    /// runtime cache.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runtime: Runtime,
    /// What the engine starts: a script the runtime runs, or an executable
    /// without one, by its path in the bundle.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entrypoint: String,
    /// What it may reach on the network, which its kinds bound.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network_level: NetworkLevel,
    /// What it may do beyond its kinds' calls.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub permissions: Permissions,
    /// The variables of its environment, each by the setting of the Plugin
    /// that values it; nothing is inherited, and no loader variable is
    /// allowed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub env: BTreeMap<String, String>,
    /// The paths outside its bundle it may read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub files: Vec<String>,
    /// The executables it may start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub processes: Vec<String>,
    /// Its resource limits, which Operator policy caps.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Limits,
    /// The browsers it adds, each by download and digest per platform.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browsers: Vec<ManifestBrowser>,
    /// Its spend outside the engine, by unit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub spend: BTreeMap<String, Spend>,
    /// For a Model provider, the provider name a Model reference names it
    /// by (ADR 0101).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provider_name: Option<String>,
    /// For a Model provider, its models, each with its capabilities and
    /// price (ADR 0101).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub models: Vec<ManifestModel>,
    /// For a Secret provider, the reference scheme it resolves (ADR 0092).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scheme: Option<String>,
}

/// The language runtime a Plugin runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Runtime {
    /// None: its entrypoint is an executable.
    #[default]
    None,
    /// Node.js.
    Node {
        /// The version, such as `22.11.0`.
        #[schemars(extend("arrived_in" = "0.1"))]
        version: String,
    },
    /// Python.
    Python {
        /// The version, such as `3.13.1`.
        #[schemars(extend("arrived_in" = "0.1"))]
        version: String,
    },
}

/// What a process Plugin may reach on the network (ADR 0033), derived from
/// its kinds rather than chosen: see [`NetworkLevel::allowed_for`].
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NetworkLevel {
    /// Nothing: it creates no socket, and reaches the outside only through
    /// the engine's calls back.
    #[default]
    None,
    /// TCP streams through the engine's proxy, to the hosts it may reach.
    ProxyTcp,
    /// The network directly.
    Direct,
}

/// Foxhound's Sink kind, which ADR 0033's rule lets go `direct`; Foxhound's
/// own part of the plugin protocol gives it its public constant.
const SINK: PluginKindName = PluginKindName::from_static("sink");

impl NetworkLevel {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::None, Self::ProxyTcp, Self::Direct];

    /// The levels a Plugin of `kinds` may declare (ADR 0033): only `none`
    /// for one with a Hook or an Extractor; `direct` too only when every
    /// kind is a Sink, Notification channel, Secret provider, Model
    /// provider, Decision provider or Code source; otherwise `none` or
    /// `proxy_tcp`.
    #[must_use]
    pub fn allowed_for<'kind>(
        kinds: impl IntoIterator<Item = &'kind PluginKindName>,
    ) -> &'static [Self] {
        let mut direct = true;
        for kind in kinds {
            if [PluginKindName::HOOK, PluginKindName::EXTRACT].contains(kind) {
                return &[Self::None];
            }
            direct &= [
                SINK,
                PluginKindName::NOTIFICATION,
                PluginKindName::SECRET,
                PluginKindName::MODEL,
                PluginKindName::DECISION,
                PluginKindName::CODE,
            ]
            .contains(kind);
        }
        if direct {
            &Self::ALL
        } else {
            &[Self::None, Self::ProxyTcp]
        }
    }

    /// Its name in the manifest.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// What a Plugin may do beyond its kinds' calls (ADR 0033), each held to
/// what Operator policy approved for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Permissions {
    /// The domains it may reach through the engine.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<String>,
    /// The Secret references it uses.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub secrets: Vec<String>,
    /// What else it asks for, each granted or refused on its own.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub grants: BTreeSet<Grant>,
}

/// A power a Plugin asks for in its manifest's `permissions`, beyond
/// domains and Secret references.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Grant {
    /// A Secret's value itself, rather than the engine inserting it on the
    /// way out.
    SecretValue,
    /// Causing Effects.
    Effects,
    /// Running script in a page.
    PageScript,
    /// A logged-in Session's credentials.
    SessionCredentials,
}

/// A process Plugin's resource limits per instance, ADR 0033's defaults
/// where the manifest is silent; Operator policy caps each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// The memory it may hold, in bytes: 256 MiB by default.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub memory: u64,
    /// The CPU time it may take, in seconds: an hour by default.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cpu_time: u64,
    /// The processes it may run at once, itself included: 1 by default.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub processes: u32,
    /// The files it may hold open at once: 128 by default.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub open_files: u32,
    /// The calls it may hold at once: 4 by default.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub concurrency: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory: 256 * 1024 * 1024,
            cpu_time: 60 * 60,
            processes: 1,
            open_files: 128,
            concurrency: 4,
        }
    }
}

/// A browser a Plugin adds (ADR 0033), which the engine downloads, checks
/// and launches itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestBrowser {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its download for each platform, such as `linux-x64`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub platforms: BTreeMap<String, Download>,
}

/// A download the engine checks against its digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Download {
    /// Its HTTPS URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its digest, `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
}

/// A unit of a Plugin's spend outside the engine (ADR 0033).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spend {
    /// The most of the unit one call spends, which the engine reserves
    /// before the call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_per_call: u64,
    /// Its price, such as `0.002 USD`; without one, a currency Limit treats
    /// the unit as unbounded and refuses the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub price: Option<String>,
}

/// A Model provider's model (ADR 0101), with its declared capabilities and
/// price.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestModel {
    /// The model's ID, the part of a Model reference after the provider
    /// name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// The tokens its context holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub context_window: Option<u64>,
    /// The most tokens one answer holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_output_tokens: Option<u64>,
    /// Whether it reads images.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub image_input: bool,
    /// Whether it calls tools.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tool_calls: bool,
    /// The structured output strategies it accepts, best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub strategies: Vec<StructuredOutputStrategy>,
    /// The price of a million input tokens, such as `3 USD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub input_price: Option<String>,
    /// The price of a million output tokens, such as `15 USD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub output_price: Option<String>,
}

/// A way a model gives structured output (ADR 0101).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputStrategy {
    /// The provider's own schema support.
    Native,
    /// A forced tool call.
    Tool,
    /// The schema in the prompt, with JSON repair.
    Prompt,
}

/// The variables of a process's environment that load code into it, which
/// a manifest never declares (ADR 0033): matched by name, ignoring case, as
/// Windows reads them, and by prefix for those ending in `*`.
pub const LOADER_VARIABLES: [&str; 7] = [
    "LD_*",
    "DYLD_*",
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
];

/// Whether `variable` is one of [`LOADER_VARIABLES`].
fn is_loader_variable(variable: &str) -> bool {
    let variable = variable.to_ascii_uppercase();
    LOADER_VARIABLES
        .iter()
        .any(|loader| match loader.strip_suffix('*') {
            Some(prefix) => variable.starts_with(prefix),
            None => variable == *loader,
        })
}

/// Why a manifest is refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// It is not a manifest.
    #[error("the Plugin manifest does not read: {0}")]
    Malformed(#[from] serde_json::Error),
    /// It declares a network level its kinds do not allow.
    #[error(
        "the Plugin manifest declares the network level `{}`, which its kinds allow only as one of {}",
        level.as_str(),
        allowed.iter().map(|level| format!("`{}`", level.as_str())).collect::<Vec<_>>().join(", ")
    )]
    NetworkLevel {
        /// The level it declares.
        level: NetworkLevel,
        /// The levels its kinds allow.
        allowed: &'static [NetworkLevel],
    },
    /// It declares a loader variable in its environment.
    #[error("the Plugin manifest declares `{0}` in its environment, a variable that loads code")]
    LoaderVariable(String),
}

/// The part of a manifest read before the rest: its version.
#[derive(Deserialize)]
struct Header {
    manifest_version: ProtocolVersion,
}

impl PluginManifest {
    /// The version of the manifest whose JSON is `json`, read before the
    /// rest, which a newer minor may have added to (ADR 0066).
    ///
    /// # Errors
    ///
    /// When `json` is no object with a `manifest_version`.
    pub fn version_of(json: &[u8]) -> Result<ProtocolVersion, ManifestError> {
        let header: Header = serde_json::from_slice(json)?;
        Ok(header.manifest_version)
    }

    /// Reads a manifest of a version its reader reads from its JSON, then
    /// holds its network level and environment to what its kinds allow.
    ///
    /// # Errors
    ///
    /// When it does not read, or declares a network level its kinds do not
    /// allow or a loader variable in its environment.
    pub fn read(json: &[u8]) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_slice(json)?;
        manifest.check()?;
        Ok(manifest)
    }

    /// Whether its kinds allow its network level and its environment holds
    /// no loader variable.
    fn check(&self) -> Result<(), ManifestError> {
        let allowed = NetworkLevel::allowed_for(&self.kinds);
        if !allowed.contains(&self.network_level) {
            return Err(ManifestError::NetworkLevel {
                level: self.network_level,
                allowed,
            });
        }
        if let Some(variable) = self
            .env
            .keys()
            .find(|variable| is_loader_variable(variable))
        {
            return Err(ManifestError::LoaderVariable(variable.clone()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
