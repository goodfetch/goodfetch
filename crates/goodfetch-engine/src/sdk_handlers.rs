//! A program's SDK handlers, which its Connection's `hello` declares by
//! Plugin kind and name (ADR 0104). The only handler of a kind that one
//! setting chooses becomes that setting's value at the connection scope, in
//! the run layer with the origin `registered`: every other run-layer value
//! replaces it, configuration does not, and a lock wins over it with no
//! refusal (ADR 0104's amendment of ADR 0083). Two handlers of such a kind
//! fail `hello` with `setting_invalid` when nothing else sets the key; so
//! does a handler named as a built-in or an installed Plugin of its kind;
//! and a setting naming an unknown handler (neither built-in, installed
//! Plugin, nor registered SDK handler) fails when its scope opens.

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_protocol::catalog::{PluginKind, Product, Setting};
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::notifications::{BUILT_IN_TYPES, NOTIFICATIONS_CHANNELS};
use goodfetch_protocol::page::{CONTENT_EXTRACTOR, CONTENT_EXTRACTORS};
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};
use serde_json::{Map, Value};

use crate::call_error::{CallError, named};
use crate::plugin_store::PluginStore;
use crate::settings::Resolved;

/// Built-in secret schemes that no Secret provider handler may shadow.
const BUILT_IN_SECRET_SCHEMES: &[&str] = &["env", "file", "keyring", "caller"];

/// The Plugin kinds an engine of `engine` carries: those of the catalog
/// modules its Product contains.
pub(crate) fn carried(engine: Product) -> Vec<&'static PluginKind> {
    MODULES
        .into_iter()
        .filter(|entries| engine.contains(Product::of_module(entries.defined_in)))
        .flat_map(|entries| entries.plugin_kinds)
        .collect()
}

/// The single-value choosing setting of `kind`, when one setting chooses a
/// single implementation by name (ADR 0104). Map-valued settings such as
/// `notifications.channels` choose per entry rather than one global name.
fn single_key_setting(kind: &PluginKind) -> Option<&'static Setting> {
    kind.chosen_by
        .filter(|setting| setting.name != NOTIFICATIONS_CHANNELS.name)
}

/// The built-in names reserved for `kind`, which no SDK handler of `kind`
/// may shadow (ADR 0104).
fn built_in_names(kind: &str) -> &'static [&'static str] {
    match kind {
        "extract" => CONTENT_EXTRACTORS,
        "notification" => &BUILT_IN_TYPES,
        "secret" => BUILT_IN_SECRET_SCHEMES,
        _ => &[],
    }
}

/// The run layer's `registered` values that `handlers`, a `hello`'s, give
/// on an engine carrying `kinds`, by dotted key: for each kind one setting
/// chooses and that has one handler, that setting set to its name.
///
/// # Errors
///
/// Invalid params for a kind not among `kinds`, which nothing would
/// otherwise answer (ADR 0083).
pub(crate) fn registered(
    handlers: &BTreeMap<String, BTreeSet<String>>,
    kinds: &[&'static PluginKind],
) -> Result<Map<String, Value>, CallError> {
    let mut values = Map::new();
    for (name, registered) in handlers {
        let Some(kind) = kinds.iter().find(|kind| kind.name == name) else {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "`hello` registers handlers of the Plugin kind `{name}`, which this engine has none of"
                ),
            ));
        };
        let mut names = registered.iter();
        if let (Some(setting), Some(handler), None) =
            (single_key_setting(kind), names.next(), names.next())
        {
            values.insert(setting.name.to_owned(), Value::String(handler.clone()));
        }
    }
    Ok(values)
}

/// Checks every declared SDK handler name against built-ins and installed
/// Plugins of its kind in `store` (ADR 0104): any clash fails `hello` with
/// `setting_invalid`.
///
/// # Errors
///
/// `setting_invalid` when a handler is named as a built-in or an installed
/// Plugin of its kind.
pub(crate) fn check_clashes(
    handlers: &BTreeMap<String, BTreeSet<String>>,
    kinds: &[&'static PluginKind],
    store: &PluginStore,
) -> Result<(), CallError> {
    for (kind_name, names) in handlers {
        let key = kinds
            .iter()
            .find(|kind| kind.name == kind_name)
            .and_then(|kind| single_key_setting(kind))
            .map_or_else(|| format!("handlers.{kind_name}"), |s| s.name.to_owned());
        let built_ins = built_in_names(kind_name);
        let plugin_kind = PluginKindName::new(kind_name);
        for handler in names {
            if built_ins.contains(&handler.as_str()) {
                return Err(setting_invalid(
                    &key,
                    format!(
                        "SDK handler `{handler}` of kind `{kind_name}` has the same name as a built-in implementation"
                    ),
                    format!(
                        "Register the `{kind_name}` handler under a name other than built-in `{handler}`."
                    ),
                ));
            }
            if let Some(plugin_kind) = &plugin_kind
                && store.resolve(handler, plugin_kind).is_ok()
            {
                return Err(setting_invalid(
                    &key,
                    format!(
                        "SDK handler `{handler}` of kind `{kind_name}` has the same name as an installed Plugin"
                    ),
                    format!(
                        "Choose a different handler name, or remove the installed Plugin `{handler}` with `goodfetch plugin remove {handler}`."
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Checks that when a Connection registers two or more handlers of a kind
/// that one setting chooses, some layer (its own `settings`, configuration
/// or an Operator policy lock) sets that key (ADR 0104).
///
/// # Errors
///
/// `setting_invalid` naming the choosing key with a `fix` telling the caller
/// to set it.
pub(crate) fn check_ambiguous(
    handlers: &BTreeMap<String, BTreeSet<String>>,
    kinds: &[&'static PluginKind],
    resolved: &Resolved,
) -> Result<(), CallError> {
    for (kind_name, names) in handlers {
        if names.len() < 2 {
            continue;
        }
        let Some(setting) = kinds
            .iter()
            .find(|kind| kind.name == kind_name)
            .and_then(|kind| single_key_setting(kind))
        else {
            continue;
        };
        if resolved
            .effective(setting)
            .is_none_or(|effective| effective.layer == SettingsLayer::Default)
        {
            let listed = names.iter().cloned().collect::<Vec<_>>().join(", ");
            return Err(setting_invalid(
                setting.name,
                format!(
                    "registered {} handlers for `{kind_name}` ({listed}) without setting {}",
                    names.len(),
                    named::setting(setting.name),
                ),
                format!(
                    "Set `{}` to one of ({listed}) or to another implementation.",
                    setting.name
                ),
            ));
        }
    }
    Ok(())
}

/// Checks that `content.extractor`, when resolved to a non-built-in name,
/// names either an installed Extractor Plugin or an SDK handler registered
/// on this Connection (ADR 0104).
///
/// # Errors
///
/// `setting_invalid` naming `content.extractor` with a `fix` naming both
/// installing a Plugin and registering an SDK handler.
pub(crate) fn check_extractor_known(
    resolved: &Resolved,
    handlers: &BTreeMap<String, BTreeSet<String>>,
    store: &PluginStore,
) -> Result<(), CallError> {
    let Some(effective) = resolved.effective(&CONTENT_EXTRACTOR) else {
        return Ok(());
    };
    let Some(name) = effective.value.as_str() else {
        return Ok(());
    };
    if CONTENT_EXTRACTORS.contains(&name) {
        return Ok(());
    }
    if handlers
        .get("extract")
        .is_some_and(|registered| registered.contains(name))
    {
        return Ok(());
    }
    if store.resolve(name, &PluginKindName::EXTRACT).is_ok() {
        return Ok(());
    }
    Err(CallError::new(
        &SETTING_INVALID,
        format!(
            "{} names `{name}`, which is neither a built-in Extractor ({}), an installed Extractor Plugin, nor a registered SDK handler.",
            named::setting(CONTENT_EXTRACTOR.name),
            CONTENT_EXTRACTORS.join(", "),
        ),
        &SettingInvalidDetails {
            key: CONTENT_EXTRACTOR.name.to_owned(),
            layer: Some(effective.layer),
            reason: format!("unknown Extractor `{name}`"),
            limit: None,
            fix: Some(format!(
                "Install an Extractor Plugin named `{name}`, register an SDK handler under `{name}` on `connect()`, or set `content.extractor` to a built-in ({}).",
                CONTENT_EXTRACTORS.join(", ")
            )),
        },
    ))
}

/// Looks up a dotted key (such as `"content.extractor"`) in a settings map
/// that may hold either nested objects (`{"content": {"extractor": ...}}`) or
/// flat dotted keys (`{"content.extractor": ...}`).
fn lookup_dotted<'a>(map: &'a Map<String, Value>, dotted: &str) -> Option<&'a Value> {
    if let Some(value) = map.get(dotted) {
        return Some(value);
    }
    let mut current: Option<&Value> = None;
    for part in dotted.split('.') {
        current = match current {
            None => map.get(part),
            Some(Value::Object(obj)) => obj.get(part),
            Some(_) => return None,
        };
    }
    current
}

/// Returns the setting key that chooses an SDK handler registered on this
/// Connection (whether via `run_settings`, `connection_settings`, or the
/// `registered` origin), if any (ADR 0104). Used to refuse `detached` starts
/// with `detach_unsupported` naming that choosing key.
pub(crate) fn chosen_sdk_handler_key(
    handlers: &BTreeMap<String, BTreeSet<String>>,
    registered: &Map<String, Value>,
    run_settings: &Map<String, Value>,
    connection_settings: &Map<String, Value>,
) -> Option<String> {
    if handlers.is_empty() {
        return None;
    }
    if let Some(extractors) = handlers.get("extract") {
        let effective = lookup_dotted(run_settings, CONTENT_EXTRACTOR.name)
            .or_else(|| lookup_dotted(connection_settings, CONTENT_EXTRACTOR.name))
            .or_else(|| registered.get(CONTENT_EXTRACTOR.name));
        if let Some(name) = effective.and_then(Value::as_str)
            && extractors.contains(name)
        {
            return Some(CONTENT_EXTRACTOR.name.to_owned());
        }
    }
    if let Some(channels) = handlers.get("notification") {
        for map in [run_settings, connection_settings] {
            if let Some(Value::Object(entries)) = lookup_dotted(map, NOTIFICATIONS_CHANNELS.name) {
                for (channel_name, entry) in entries {
                    let chosen_type = entry
                        .as_object()
                        .and_then(|obj| obj.get("type"))
                        .and_then(Value::as_str);
                    if channels.contains(channel_name)
                        || chosen_type.is_some_and(|t| channels.contains(t))
                    {
                        return Some(format!("{}.{channel_name}", NOTIFICATIONS_CHANNELS.name));
                    }
                }
            }
        }
    }
    if let Some(secrets) = handlers.get("secret") {
        for map in [run_settings, connection_settings] {
            if let Some(Value::Object(entries)) = lookup_dotted(map, "secrets") {
                for (secret_name, entry) in entries {
                    if let Some(reference) = entry.as_str()
                        && let Some(rest) = reference.strip_prefix("plugin:")
                        && let Some((scheme, _)) = rest.split_once(':')
                        && secrets.contains(scheme)
                    {
                        return Some(format!("secrets.{secret_name}"));
                    }
                }
            }
        }
    }
    None
}

fn setting_invalid(key: &str, reason: String, fix: String) -> CallError {
    CallError::new(
        &SETTING_INVALID,
        format!("{} is invalid: {reason}. {fix}", named::setting(key)),
        &SettingInvalidDetails {
            key: key.to_owned(),
            layer: Some(SettingsLayer::Run),
            reason,
            limit: None,
            fix: Some(fix),
        },
    )
}

#[cfg(test)]
mod tests;
