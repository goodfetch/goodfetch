//! The Fingerprint method driver (ADRs 0035 and 0129; ticket #913): what
//! every method shares lives here once, the warning, the Provenance mark
//! and the Audit record of a `stealth` Session's choice, and each method
//! fills only its primitives: where its Sessions are placed, its CDP
//! Emulation values and its init script.
//!
//! `none`, `js_injection` and `basic` share a browser process as contexts,
//! and none needs a launch argument, so the launch-arguments source of
//! #395 has nothing of theirs yet. `basic` has each page show its values
//! through the browser's own overrides, and `js_injection` through a script
//! run in the main world of each document of the page's own process, which
//! Provenance marks as a main-world injection (ADRs 0019 and 0048); its
//! out-of-process frames and its workers are not masked yet (#913). `engine` is not built: a
//! `stealth` Session that leaves the method unset falls back to `basic`
//! with its warning, and one that asks for `engine` is refused (ADR 0083).
//! The values come from the Session's Identity profile once it is built
//! (#993); until then each method shows the machine's own.

#[cfg(test)]
mod tests;

use goodfetch_engine::audit::Origin;
use goodfetch_engine::runs::{Run, warnings};
use goodfetch_protocol::audit::{Actor, FINGERPRINT_METHOD_CHOSEN};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::{
    FINGERPRINT_BASIC, FINGERPRINT_BASIC_FALLBACK, FINGERPRINT_JS_INJECTION, FINGERPRINT_NONE,
};
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::json;

use crate::access_mode::{FingerprintMethod, MethodSource, SessionIdentityPlan};
use crate::cdp::{CdpPage, Emulated};
use crate::page_functions::{Injection, Injections};
use crate::pool::Placement;
use crate::trace;

/// The Provenance action of a Session's Fingerprint method.
const FINGERPRINT_METHOD: &str = "fingerprint_method";

/// A Fingerprint method this engine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    None,
    JsInjection,
    Basic,
}

impl Method {
    /// The method `method` names, when this engine builds it.
    const fn built(method: FingerprintMethod) -> Option<Self> {
        match method {
            FingerprintMethod::None => Some(Self::None),
            FingerprintMethod::JsInjection => Some(Self::JsInjection),
            FingerprintMethod::Basic => Some(Self::Basic),
            FingerprintMethod::Engine => None,
        }
    }

    /// Its name, as `stealth.fingerprint_method` writes it.
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => FingerprintMethod::None.as_str(),
            Self::JsInjection => FingerprintMethod::JsInjection.as_str(),
            Self::Basic => FingerprintMethod::Basic.as_str(),
        }
    }
}

/// How a `stealth` Session masks its browser's fingerprint: its method,
/// the layer that set it or its fallback to `basic` for want of a
/// Goodfetch Chromium, the layer that chose `stealth`, and the values its
/// pages show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FingerprintPlan {
    method: Method,
    source: MethodSource,
    stealth: SettingsLayer,
    values: Emulated,
}

impl FingerprintPlan {
    /// The plan of a Session that opens as `plan` says: none for a
    /// Session that is not `stealth`, nor for a method this engine does not
    /// build, which [`SessionIdentityPlan::built`] refuses first.
    pub(crate) fn of(plan: &SessionIdentityPlan) -> Option<Self> {
        let profile = plan.profile.as_ref()?;
        Some(Self {
            method: Method::built(profile.fingerprint_method)?,
            source: profile.method_source,
            stealth: plan.layer,
            values: Emulated::default(),
        })
    }

    /// Whether its method fell back to `basic`.
    fn fell_back(&self) -> bool {
        self.source == MethodSource::FellBack
    }

    /// The layer that chose its method: the one that set it, or the one
    /// that chose `stealth` for the fallback.
    const fn layer(&self) -> SettingsLayer {
        match self.source {
            MethodSource::Set(layer) => layer,
            MethodSource::FellBack => self.stealth,
        }
    }

    /// The refusal of a `page.on` that needs the browser's `Runtime`
    /// domain on one of its pages, which a detector sees (#599's Q2).
    pub(crate) fn runtime_refusal(&self) -> goodfetch_engine::CallError {
        crate::access_mode::runtime_under_stealth(self.stealth)
    }

    /// Where its Sessions are placed: every built method shares a browser
    /// process as contexts, and needs no launch argument of its own.
    pub(crate) const fn placement(&self) -> Placement {
        match self.method {
            Method::None | Method::JsInjection | Method::Basic => Placement::Shared,
        }
    }

    /// The values its pages show through the browser's own overrides:
    /// `basic`'s alone.
    fn emulated(&self) -> Option<&Emulated> {
        (self.method == Method::Basic && self.values != Emulated::default()).then_some(&self.values)
    }

    /// The script its pages run before each document's own: `js_injection`'s
    /// alone, when it has a value to show.
    fn init_script(&self) -> Option<String> {
        (self.method == Method::JsInjection)
            .then(|| init_script(&self.values))
            .flatten()
    }

    /// The warning the Session `session`'s Run report carries, and whether
    /// one stands for every Session of the Run.
    fn warning(&self, session: &SessionId) -> (RunWarning, bool) {
        let (code, message) = match (self.method, self.fell_back()) {
            (Method::None, _) => (
                FINGERPRINT_NONE,
                "A `stealth` Session ran with the Fingerprint method `none`: the machine's real fingerprint shows, and the Run's Sessions are linkable.",
            ),
            (Method::JsInjection, _) => (
                FINGERPRINT_JS_INJECTION,
                "A `stealth` Session ran with the Fingerprint method `js_injection`: a detector can see the values injected into its pages.",
            ),
            (Method::Basic, false) => (
                FINGERPRINT_BASIC,
                "A `stealth` Session ran with the Fingerprint method `basic`: its graphics, canvas, audio and fonts stay the machine's own, so Sessions on one machine stay linkable.",
            ),
            (Method::Basic, true) => (
                FINGERPRINT_BASIC_FALLBACK,
                "A `stealth` Session fell back to the Fingerprint method `basic`, as no Goodfetch Chromium exists for this machine: its graphics, canvas, audio and fonts stay the machine's own, so Sessions on one machine stay linkable.",
            ),
        };
        let once = self.method == Method::None;
        // Until an Identity profile gives it values (#993), a masking
        // method shows the machine's own as well.
        let unmasked = self.method != Method::None && self.values == Emulated::default();
        let warning = RunWarning {
            code: code.to_owned(),
            message: if unmasked {
                format!(
                    "{message} Its time zone, locale, geolocation and core count are the machine's own too, as no Identity profile gives them values yet."
                )
            } else {
                message.to_owned()
            },
            sessions: if once {
                Vec::new()
            } else {
                vec![session.0.clone()]
            },
            folder: None,
        };
        (warning, once)
    }

    /// What its Provenance mark and its Audit record say of the Session
    /// `session`'s choice.
    fn details(&self, session: &SessionId) -> serde_json::Value {
        json!({
            "session_id": session,
            "method": self.method.as_str(),
            "fell_back": self.fell_back(),
        })
    }

    /// Records that the Session `session` of `run` opened with this
    /// method: its Provenance mark, its warning, `none`'s once per report,
    /// and its Audit record. A warning or a record that cannot be kept is
    /// logged.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the entries not yet written.
    pub(crate) async fn chosen(&self, run: &Run, session: &SessionId) {
        let details = self.details(session);
        let mut mark = details.clone();
        mark["action"] = json!(FINGERPRINT_METHOD);
        trace::provenance(run, mark).await;
        let (warning, once) = self.warning(session);
        let kept = if once {
            warnings(run).warn_once(warning).await
        } else {
            warnings(run).warn(warning).await
        };
        if let Err(error) = kept {
            tracing::warn!(%error, "the warning of a Session's Fingerprint method was not kept");
        }
        let origin = Origin {
            actor: Actor::Engine,
            surface: None,
            layer: Some(self.layer()),
            run_id: Some(run.id().clone()),
        };
        if let Err(error) = run
            .audit_log()
            .record(
                &origin,
                &FINGERPRINT_METHOD_CHOSEN,
                details,
                run.clock().timestamp(),
            )
            .await
        {
            tracing::error!(%error, "the Audit log did not record a Session's Fingerprint method");
        }
    }

    /// Has the page `page_id` of the Session `session`, driven as `page`,
    /// show this method's values before it runs anything of its own;
    /// `js_injection`'s script is marked once in Provenance as the
    /// Session's main-world injection, in `injections`.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave some of the values set.
    pub(crate) async fn mask(
        &self,
        page: &CdpPage,
        (run, session, page_id): (&Run, &SessionId, &PageId),
        injections: &Injections,
    ) -> Result<(), goodfetch_engine::CallError> {
        if let Some(values) = self.emulated() {
            page.emulate_opening(values).await?;
        }
        if let Some(script) = self.init_script() {
            page.add_init_script_opening(script).await?;
            injections
                .mark(run, session, page_id, Injection::FingerprintMethod)
                .await;
        }
        Ok(())
    }
}

/// `js_injection`'s script, which has a document read `values` in place of
/// its machine's: none when it has none to show. Each value is written as
/// a JSON literal, so no value can break out of the script.
fn init_script(values: &Emulated) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(cores) = values.core_count {
        lines.push(format!(
            "define(Navigator.prototype, \"hardwareConcurrency\", {cores});"
        ));
    }
    if let Some(locale) = &values.locale {
        let locale = json!(locale);
        lines.push(format!(
            "define(Navigator.prototype, \"language\", {locale});\n  define(Navigator.prototype, \"languages\", Object.freeze([{locale}]));"
        ));
    }
    if let Some(zone) = &values.time_zone {
        let zone = json!(zone);
        lines.push(format!(
            "const resolved = Intl.DateTimeFormat.prototype.resolvedOptions;\n  Intl.DateTimeFormat.prototype.resolvedOptions = function () {{ return {{ ...resolved.call(this), timeZone: {zone} }}; }};"
        ));
    }
    if let Some(position) = values.geolocation {
        let coords = json!({
            "latitude": position.latitude,
            "longitude": position.longitude,
            "accuracy": position.accuracy,
            "altitude": null,
            "altitudeAccuracy": null,
            "heading": null,
            "speed": null,
        });
        lines.push(format!(
            "const at = () => ({{ coords: {coords}, timestamp: Date.now() }});\n  navigator.geolocation.getCurrentPosition = (found) => found(at());\n  navigator.geolocation.watchPosition = (found) => {{ found(at()); return 0; }};"
        ));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "(() => {{\n  const define = (target, name, value) => Object.defineProperty(target, name, {{ get: () => value, configurable: true }});\n  {}\n}})();",
        lines.join("\n  ")
    ))
}
