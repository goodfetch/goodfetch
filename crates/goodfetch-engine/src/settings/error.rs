//! Why a value cannot take effect where it is set (ADRs 0083 and 0087), each
//! refusal with its Error code; `run::settings_error` turns one into a
//! call's failure.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::FORMAT_VERSION_UNSUPPORTED;
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, SETTING_INVALID, SETTING_LOCKED, SETTING_NOT_APPLICABLE,
    SETTING_UNKNOWN, SettingLimit, SettingsLayer,
};

/// A refused setting, a file that does not read, or a deployment variable
/// that is not one.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// A key no Product knows, or an unknown `GOODFETCH_` variable.
    #[error("{}", unknown_message(key, *layer, closest.as_deref()))]
    Unknown {
        /// The key or variable as written.
        key: String,
        /// Its layer; `None` for the environment.
        layer: Option<SettingsLayer>,
        /// The closest known one.
        closest: Option<String>,
    },
    /// A value of the wrong kind, a file that does not read, or an empty
    /// deployment variable.
    #[error("`{key}` is not valid{}: {reason}", in_layer(*layer))]
    Invalid {
        /// The key, variable or file.
        key: String,
        /// Its layer; `None` for the environment.
        layer: Option<SettingsLayer>,
        /// What is wrong.
        reason: String,
    },
    /// A key set where it cannot take effect.
    #[error("`{key}` cannot be set in {}: {fix}", layer.as_str())]
    NotApplicable {
        /// The key.
        key: String,
        /// The layer that held it.
        layer: SettingsLayer,
        /// Where it can be set instead.
        fix: String,
    },
    /// A value away from its default of a setting whose feature the
    /// engine does not build yet, which nothing would read (ADR 0083).
    #[error(
        "`{key}` is set in {}, and this engine does not build what it asks for yet (`{feature}`), so its value would be ignored: leave it at its default, {fix}",
        layer.as_str()
    )]
    Unavailable {
        /// The key.
        key: String,
        /// The layer that set it.
        layer: SettingsLayer,
        /// The feature it asks for.
        feature: String,
        /// Its default, the value that does without the feature.
        fix: String,
    },
    /// A value a lower layer locked, capped or reserved.
    #[error(
        "`{key}` in {} is held by {} at `{}`: {fix}",
        layer.as_str(),
        limit.layer.as_str(),
        limit.value
    )]
    Locked {
        /// The key.
        key: String,
        /// The layer whose value was refused.
        layer: SettingsLayer,
        /// What holds it.
        limit: SettingLimit,
        /// What to do instead.
        fix: String,
    },
    /// A settings file of a format version this engine cannot read (ADR
    /// 0066), or a shared one whose lowest minor is newer than this engine
    /// (ADR 0098).
    #[error("{}", format_message(format, found, *compatibility, install.as_deref()))]
    Format {
        /// The file's layer.
        layer: SettingsLayer,
        /// The format's name.
        format: &'static str,
        /// The file's version.
        found: String,
        /// The version this engine reads.
        supported: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
        /// The Product version to install, when its lowest minor refused
        /// this engine.
        install: Option<String>,
    },
    /// Goodfetch home holds an Operator policy only for an older major: a
    /// new major refuses to start until `upgrade-files` has given it its
    /// copy (ADR 0030).
    #[error(
        "Goodfetch home holds an Operator policy only for the older major {major}, at `{path}`; run `upgrade-files` to give the major {current} its copy, then try again"
    )]
    OlderMajorPolicy {
        /// The older major's Operator policy.
        path: String,
        /// The older major's name.
        major: String,
        /// This engine's major's name.
        current: String,
    },
    /// A key, or a Recipe, of the Product this engine lacks, given
    /// explicitly.
    #[error("`{key}` is part of {}", needs.name())]
    NotInProduct {
        /// The key, or the Recipe's name.
        key: String,
        /// The Product that carries it.
        needs: Product,
    },
}

fn format_message(
    format: &str,
    found: &str,
    compatibility: Compatibility,
    install: Option<&str>,
) -> String {
    match (compatibility, install) {
        (_, Some(install)) => format!(
            "this {format} file ({found}) can be used from Goodfetch {install} on; install Goodfetch {install} or newer"
        ),
        (Compatibility::OlderMajor, None) => format!(
            "this {format} file is {found}, of an older major; run `upgrade-files` to upgrade it"
        ),
        (_, None) => format!(
            "this {format} file is {found}, newer than this engine reads; upgrade Goodfetch to read it"
        ),
    }
}

fn in_layer(layer: Option<SettingsLayer>) -> String {
    layer.map_or_else(
        || " in the environment".to_owned(),
        |layer| format!(" in {}", layer.as_str()),
    )
}

fn unknown_message(key: &str, layer: Option<SettingsLayer>, closest: Option<&str>) -> String {
    let what = if layer.is_some() {
        "no setting"
    } else {
        "no deployment variable"
    };
    match closest {
        Some(closest) => format!(
            "`{key}`{} is {what}; did you mean `{closest}`?",
            in_layer(layer)
        ),
        None => format!("`{key}`{} is {what}.", in_layer(layer)),
    }
}

impl SettingsError {
    /// Its Error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown { .. } => SETTING_UNKNOWN.name,
            Self::Invalid { .. } => SETTING_INVALID.name,
            Self::NotApplicable { .. } => SETTING_NOT_APPLICABLE.name,
            Self::Unavailable { .. } => CAPABILITY_UNAVAILABLE.name,
            Self::Locked { .. } => SETTING_LOCKED.name,
            Self::NotInProduct { .. } => NOT_IN_PRODUCT.name,
            Self::Format { .. } | Self::OlderMajorPolicy { .. } => FORMAT_VERSION_UNSUPPORTED.name,
        }
    }
}
