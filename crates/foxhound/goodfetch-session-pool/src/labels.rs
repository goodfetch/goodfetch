//! Which pool each Label's requests draw from (ADR 0093): a Label that sets
//! `session.source`, `session.http_credentials`, any `secrets.*` key or a
//! `login` step gets a pool of its own, so a logged-in section and a public
//! section of one site never share cookies; every other Label shares the
//! Run's pool, a Label's `session.page_check` and
//! `locator.test_id_attribute` among what needs none. A Label's own pool
//! opens its Sessions from its `session.source`, `null` opening each fresh
//! whatever the Run's; the Run's pool opens them from the Run's.

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{
    SESSION_SOURCE, SETTING_INVALID, SettingInvalidDetails, SettingsLayer,
};
use serde_json::{Map, Value};

/// Which of a crawl's Session pools a Session belongs to: the Run's, or
/// the own pool of the Label so named.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoolKey {
    /// The Run's pool.
    Run,
    /// The pool of the Label so named.
    Label(String),
}

/// The keys under which a Label's setting gives it a pool of its own, each
/// as its path in the Label's nested settings: a whole group (`secrets`,
/// `login`) when any key under it does.
const OWN_POOL_KEYS: [&[&str]; 4] = [
    &["session", "source"],
    &["session", "http_credentials"],
    &["secrets"],
    &["login"],
];

/// The Labels of a crawl that have a pool of their own, each with the
/// Session source its Sessions open from: `None` for a fresh Session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelPools(BTreeMap<String, Option<SessionSource>>);

impl LabelPools {
    /// The pools `labels` declares.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a Label's `session.source` that is neither
    /// `null` nor a Session source.
    pub fn of(labels: &BTreeMap<String, LabelDeclaration>) -> Result<Self, CallError> {
        let mut pools = BTreeMap::new();
        for (name, declaration) in labels {
            if !OWN_POOL_KEYS
                .iter()
                .any(|path| at(&declaration.settings, path).is_some())
            {
                continue;
            }
            let source = match at(&declaration.settings, &["session", "source"]) {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    serde_json::from_value::<SessionSource>(value.clone()).map_err(|error| {
                        CallError::new(
                            &SETTING_INVALID,
                            format!(
                                "The Label `{name}` sets `session.source` to {value}, which is no Session source: {error}."
                            ),
                            &SettingInvalidDetails {
                                key: SESSION_SOURCE.name.to_owned(),
                                layer: Some(SettingsLayer::Run),
                                reason: error.to_string(),
                            },
                        )
                    })?,
                ),
            };
            pools.insert(name.clone(), source);
        }
        Ok(Self(pools))
    }

    /// The pool the requests of `label` draw from.
    pub fn pool(&self, label: &str) -> PoolKey {
        if self.0.contains_key(label) {
            PoolKey::Label(label.to_owned())
        } else {
            PoolKey::Run
        }
    }

    /// The Session source the Sessions of `pool` open from: the Label's
    /// own, `Some(None)` opening each fresh, or `None` for the Run's.
    pub fn source(&self, pool: &PoolKey) -> Option<Option<SessionSource>> {
        match pool {
            PoolKey::Run => None,
            PoolKey::Label(label) => self.0.get(label).cloned(),
        }
    }
}

/// The value at `path` in the nested settings `settings`, if set.
fn at<'a>(settings: &'a Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut nested = settings;
    for parent in parents {
        nested = nested.get(*parent)?.as_object()?;
    }
    nested.get(*last)
}

#[cfg(test)]
mod tests;
