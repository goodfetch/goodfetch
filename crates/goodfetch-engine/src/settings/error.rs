//! Why a value cannot take effect where it is set (ADRs 0083 and 0087), each
//! refusal with its Error code; `run::settings_error` turns one into a
//! call's failure.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::settings::{
    SETTING_INVALID, SETTING_LOCKED, SETTING_NOT_APPLICABLE, SETTING_UNKNOWN, SettingLimit,
    SettingsLayer,
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
            Self::Locked { .. } => SETTING_LOCKED.name,
            Self::NotInProduct { .. } => NOT_IN_PRODUCT.name,
        }
    }
}
