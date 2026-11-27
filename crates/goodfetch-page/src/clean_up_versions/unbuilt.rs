//! What a Run's settings ask of the Clean-up rules that this engine
//! cannot do yet, refused when the Run starts, never ignored (ADR 0083):
//! `clean_up.update = daily`, whose versions are used only once the
//! channel's signature is checked (ADR 0033's verifier).

use goodfetch_engine::CallError;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CLEAN_UP_UPDATE, CapabilityUnavailableDetails, SettingsLayer,
};
use serde_json::json;

/// Refuses a start whose `settings` ask for what this engine cannot do yet:
/// `clean_up.update = daily` until the channel's signature is checked.
///
/// # Errors
///
/// `capability_unavailable` naming the setting and the layer that set it.
pub(super) fn refuse_unbuilt(settings: Option<&Resolved>) -> Result<(), CallError> {
    let Some(settings) = settings else {
        return Ok(());
    };
    if let Some(effective) = settings
        .effective(&CLEAN_UP_UPDATE)
        .filter(|effective| effective.value == json!("daily"))
    {
        return Err(unavailable(
            CLEAN_UP_UPDATE.name,
            effective.layer,
            "daily",
            "the daily channel's versions are used only once the engine checks their signature, which it cannot yet",
            "`clean_up.update: release` or `upstream`, or a `clean_up.version` pinned by its digest",
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
