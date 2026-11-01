//! What a Run's settings ask of the Clean-up rules that this engine
//! cannot do yet, refused when the Run starts, never ignored (ADR 0083):
//! `clean_up.update = daily`, whose versions are used only once the
//! channel's signature is checked (ADR 0033's verifier),
//! `filter_lists.page_scriptlets` and `filter_lists.extra`.

use goodfetch_engine::CallError;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CLEAN_UP_UPDATE, CapabilityUnavailableDetails, FILTER_LISTS_EXTRA,
    FILTER_LISTS_PAGE_SCRIPTLETS, SettingsLayer,
};
use serde_json::json;

/// Refuses a start whose `settings` ask for what this engine cannot do yet:
/// `clean_up.update = daily` until the channel's signature is checked,
/// `filter_lists.page_scriptlets` and `filter_lists.extra` until they run.
///
/// # Errors
///
/// `capability_unavailable` naming the setting and the layer that set it.
pub(super) fn refuse_unbuilt(settings: Option<&Resolved>) -> Result<(), CallError> {
    let Some(settings) = settings else {
        return Ok(());
    };
    let unbuilt = [
        (
            CLEAN_UP_UPDATE.name,
            json!("daily"),
            "daily",
            "the daily channel's versions are used only once the engine checks their signature, which it cannot yet",
            "`clean_up.update: release` or `upstream`, or a `clean_up.version` pinned by its digest",
        ),
        (
            FILTER_LISTS_PAGE_SCRIPTLETS.name,
            json!(true),
            "page_scriptlets",
            "this engine runs no Filter-list scriptlet yet",
            "`filter_lists.page_scriptlets: false`",
        ),
    ];
    for (key, refused, kind, why, fix) in unbuilt {
        if let Some(effective) = settings
            .get(key)
            .filter(|effective| effective.value == refused)
        {
            return Err(unavailable(key, effective.layer, kind, why, fix));
        }
    }
    if let Some(effective) = settings.get(FILTER_LISTS_EXTRA.name).filter(|effective| {
        effective
            .value
            .as_array()
            .is_some_and(|lists| !lists.is_empty())
    }) {
        return Err(unavailable(
            FILTER_LISTS_EXTRA.name,
            effective.layer,
            "extra",
            "this engine fetches no extra Filter list yet",
            "an empty `filter_lists.extra`",
        ));
    }
    Ok(())
}

/// `capability_unavailable` for `key`, set by `layer` to `kind`, which the
/// engine cannot do for `why`; `fix` says what to set instead.
fn unavailable(key: &str, layer: SettingsLayer, kind: &str, why: &str, fix: &str) -> CallError {
    CallError::new(
        &CAPABILITY_UNAVAILABLE,
        format!(
            "`{key}` is `{kind}` in {}, but {why}: set {fix}.",
            layer.as_str()
        ),
        &CapabilityUnavailableDetails {
            key: key.to_owned(),
            layer,
            kind: kind.to_owned(),
            fix: fix.to_owned(),
        },
    )
}
