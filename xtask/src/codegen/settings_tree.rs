//! The settings tree each SDK's settings types nest by (ADR 0083): the
//! catalog's settings a scope sets, by their wire segments, and the scope a
//! member's `settings` option opens. Each language's generator spells the
//! tree its own way (ADR 0099).

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail, ensure};
use goodfetch_protocol::catalog::SettingScope;

use super::handles::{Form, Member};
use super::layer::Setting;

/// The optional parameter that takes a scope's own settings (ADR 0083).
pub(super) const SETTINGS: &str = "settings";

/// A segment of the settings tree: a setting, or an area of more segments.
pub(super) enum Node<'a> {
    Setting(&'a Setting),
    Area(BTreeMap<String, Node<'a>>),
}

/// The settings tree of those `settings` that `scope` sets, by wire
/// segment; with no scope, of every setting some scope sets.
pub(super) fn tree(
    settings: &[Setting],
    scope: Option<SettingScope>,
) -> Result<BTreeMap<String, Node<'_>>> {
    grown(settings, |setting| match scope {
        Some(scope) => setting.scopes.iter().any(|named| named == scope.as_str()),
        None => !setting.scopes.is_empty(),
    })
}

/// The settings tree of every one of `settings`, those no scope sets
/// included: what spells a key a program passes in code although only a
/// file sets it, so that the engine names that file (ADR 0083).
pub(super) fn every(settings: &[Setting]) -> Result<BTreeMap<String, Node<'_>>> {
    grown(settings, |_| true)
}

/// The settings tree of those `settings` that `applies` keeps.
fn grown(
    settings: &[Setting],
    applies: impl Fn(&Setting) -> bool,
) -> Result<BTreeMap<String, Node<'_>>> {
    let mut root = BTreeMap::new();
    for setting in settings {
        if !applies(setting) {
            continue;
        }
        let segments: Vec<&str> = setting.name.split('.').collect();
        let (last, areas) = segments
            .split_last()
            .with_context(|| format!("the setting `{}` has no segment", setting.name))?;
        let mut level = &mut root;
        for area in areas {
            let node = level
                .entry((*area).to_owned())
                .or_insert_with(|| Node::Area(BTreeMap::new()));
            level = match node {
                Node::Area(children) => children,
                Node::Setting(other) => bail!(
                    "the setting `{}` is inside the setting `{}`",
                    setting.name,
                    other.name
                ),
            };
        }
        ensure!(
            level
                .insert((*last).to_owned(), Node::Setting(setting))
                .is_none(),
            "the setting `{}` holds other settings",
            setting.name
        );
    }
    Ok(root)
}

/// The scope `member` opens, when it takes `settings`.
pub(super) fn opened_scope<'m>(member: &'m Member<'_>) -> Option<&'m str> {
    match &member.form {
        Form::Opens(opened)
            if member
                .parameters
                .optional
                .iter()
                .any(|name| name == SETTINGS) =>
        {
            Some(opened.as_str())
        }
        _ => None,
    }
}
