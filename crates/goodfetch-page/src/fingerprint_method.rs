//! The Fingerprint method driver (ADRs 0035 and 0129; ticket #913): what
//! every method shares lives here once, the warning, the Provenance mark
//! and the Audit record of a `stealth` Session's choice, and each method
//! fills only its primitives: where its Sessions are placed, its CDP
//! Emulation values and its init script.
//!
//! `none`, `js_injection` and `basic` share a browser process as contexts,
//! and none adds a Chromium launch flag of its own yet; [`wpt_launch_args`]
//! emits the engine's Chromium launch arguments for `none` and `js_injection`
//! with [`WPT_IDENTITY_SEED`] for `xtask codegen` (ADR 0074; ticket #395),
//! and `engine`'s flags join it when the Goodfetch Chromium build lands
//! (#969). `basic` has each page show its values
//! through the browser's own overrides, and `js_injection` through a script
//! run in the main world of each document of the page's own process, which
//! Provenance marks as a main-world injection (ADRs 0019 and 0048). None of
//! a page's overrides reaches its out-of-process frames or its workers, so
//! each takes them on its own target before it runs (`children`): a frame
//! `basic`'s values but the screen, which Chromium sets on a page alone,
//! and `js_injection`'s script; a dedicated, shared or service worker the
//! values a worker reads, its user agent, time zone and core count, and no
//! script. A shared or service worker's `navigator.userAgent` and every
//! worker's `navigator.platform` stay the browser's own. `engine` is not built: a
//! `stealth` Session that leaves the method unset falls back to `basic`
//! with its warning, and one that asks for `engine` is refused (ADR 0083).
//! The values come from the Session's Identity profile (#993), at the
//! version of the browser that runs it; with `stealth.generated_identity`
//! off the Session has none, and each method shows the browser's own.
//! Until the proxy exit gives them (#997), a profile's time zone and
//! geolocation stay the machine's.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_engine::audit::Origin;
use goodfetch_identity_profile::{
    BrowserVersion, Constraints, EngineFamily, ExitFacts, IdentityProfile, IdentitySeed, generate,
    refresh_version_fields,
};
use goodfetch_protocol::audit::{Actor, FINGERPRINT_METHOD_CHOSEN};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::{
    FINGERPRINT_BASIC, FINGERPRINT_BASIC_FALLBACK, FINGERPRINT_JS_INJECTION, FINGERPRINT_NONE,
    STEALTH_RUNTIME_ALLOWED,
};
use goodfetch_protocol::settings::SettingsLayer;
use serde::Serialize;
use serde_json::json;

use crate::access_mode::{FingerprintMethod, MethodSource, RuntimeDomain, SessionIdentityPlan};
use crate::browser_run::BrowserRun;
use crate::cdp::launch::{Window, wpt_base_arguments};
use crate::cdp::{CdpPage, Emulated, Geolocation, ScreenSize, TargetSession, UserAgentOverride};
use crate::page_children::{Child, ChildKind, OnChild};
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

    /// Method-specific Chromium launch arguments derived from `profile`.
    /// `none`, `js_injection` and `basic` share a browser process and need no
    /// extra flags of their own; `engine`'s flags join here in #969.
    fn launch_arguments(self, _profile: &IdentityProfile) -> Vec<String> {
        match self {
            Self::None | Self::JsInjection | Self::Basic => Vec::new(),
        }
    }
}

/// The fixed Identity profile seed `xtask codegen` uses when generating
/// `chromium/launch-args.json` for the Chromium build gate's WPT runs
/// (ADR 0074; ticket #395).
pub const WPT_IDENTITY_SEED: IdentitySeed = IdentitySeed(1);

/// Screen geometry from the drawn Identity profile recorded in
/// `chromium/launch-args.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WptScreen {
    pub width: u32,
    pub height: u32,
    pub avail_width: u32,
    pub avail_height: u32,
}

/// Deterministic Identity profile summary drawn from [`WPT_IDENTITY_SEED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WptIdentityProfile {
    pub user_agent: String,
    pub platform: String,
    pub languages: Vec<String>,
    pub hardware_concurrency: u32,
    pub screen: WptScreen,
}

/// Launch arguments (and, for `js_injection`, the init script) for one
/// Fingerprint method in `chromium/launch-args.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WptMethodArgs {
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_script: Option<String>,
}

/// The committed `chromium/launch-args.json` document generated by
/// `xtask codegen` from the engine's own launch-argument code (ADR 0074;
/// ticket #395).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WptLaunchArgs {
    pub generated_by: &'static str,
    pub identity_seed: u64,
    pub identity_profile: WptIdentityProfile,
    pub methods: BTreeMap<String, WptMethodArgs>,
}

/// Computes the Chromium build gate's WPT launch arguments for `none` and
/// `js_injection` from the engine's own launch-argument builder and Identity
/// profile generator at `seed` (ADR 0074; ticket #395).
#[must_use]
pub fn wpt_launch_args(seed: IdentitySeed) -> WptLaunchArgs {
    let constraints = Constraints::narrowed(EngineFamily::Chromium, &[], &[], &[])
        .expect("invariant: unnarrowed Chromium population is non-empty");
    let profile = generate(seed, &constraints, ExitFacts::default());
    let base = wpt_base_arguments();
    let emulated = shown(&profile);

    let mut methods = BTreeMap::new();
    for method in [Method::None, Method::JsInjection] {
        let mut args = base.clone();
        args.extend(method.launch_arguments(&profile));
        let script = match method {
            Method::JsInjection => init_script(&emulated),
            Method::None | Method::Basic => None,
        };
        methods.insert(
            method.as_str().to_owned(),
            WptMethodArgs {
                args,
                init_script: script,
            },
        );
    }

    WptLaunchArgs {
        generated_by: "xtask codegen",
        identity_seed: seed.0,
        identity_profile: WptIdentityProfile {
            user_agent: profile.user_agent,
            platform: profile.navigator.platform,
            languages: profile.navigator.languages,
            hardware_concurrency: profile.navigator.hardware_concurrency,
            screen: WptScreen {
                width: profile.screen.width,
                height: profile.screen.height,
                avail_width: profile.screen.avail_width,
                avail_height: profile.screen.avail_height,
            },
        },
        methods,
    }
}

/// How a `stealth` Session masks its browser's fingerprint: its method,
/// the layer that set it or its fallback to `basic` for want of a
/// Goodfetch Chromium, the layer that chose `stealth`, the Identity profile
/// its pages show, none with `stealth.generated_identity` off, whether
/// `stealth.allow_runtime` lets them turn on the browser's `Runtime`
/// domain, and the window its browser shows them in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FingerprintPlan {
    method: Method,
    source: MethodSource,
    stealth: SettingsLayer,
    identity: Option<Box<IdentityProfile>>,
    runtime: RuntimeDomain,
    window: Window,
}

impl FingerprintPlan {
    /// The plan of a Session that opens as `plan` says, showing `identity`:
    /// none for a Session that is not `stealth`, nor for a method this
    /// engine does not build, which [`SessionIdentityPlan::built`] refuses
    /// first.
    pub(crate) fn of(
        plan: &SessionIdentityPlan,
        identity: Option<IdentityProfile>,
    ) -> Option<Self> {
        let profile = plan.profile.as_ref()?;
        Some(Self {
            method: Method::built(profile.fingerprint_method)?,
            source: profile.method_source,
            stealth: plan.layer,
            identity: identity.map(Box::new),
            runtime: profile.runtime,
            window: Window::of(profile.headless),
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
    /// domain on one of its pages, which a detector sees: none once
    /// `stealth.allow_runtime` allows it (#599's Q2).
    pub(crate) fn runtime_refusal(&self) -> Option<goodfetch_engine::CallError> {
        (self.runtime == RuntimeDomain::Refused)
            .then(|| crate::access_mode::runtime_under_stealth(self.stealth))
    }

    /// The warning the Session `session`'s Run report carries when
    /// `stealth.allow_runtime` lets its pages turn on the browser's
    /// `Runtime` domain.
    fn runtime_warning(&self, session: &SessionId) -> Option<RunWarning> {
        (self.runtime == RuntimeDomain::Allowed).then(|| RunWarning {
            code: STEALTH_RUNTIME_ALLOWED.to_owned(),
            message: "A `stealth` Session ran with `stealth.allow_runtime`: its pages could turn on the browser's `Runtime` domain, which a detector sees.".to_owned(),
            sessions: vec![session.0.clone()],
            folder: None,
        })
    }

    /// Where its Sessions are placed: every built method shares a browser
    /// process as contexts, and needs no launch argument of its own.
    pub(crate) const fn placement(&self) -> Placement {
        match self.method {
            Method::None | Method::JsInjection | Method::Basic => Placement::Shared,
        }
    }

    /// How its browser shows its pages: in a window unless the Session's
    /// `browser.headless` says otherwise (ADR 0035).
    pub(crate) const fn window(&self) -> Window {
        self.window
    }

    /// The values its pages show through the browser's own overrides,
    /// `basic`'s, and the script they run before each document's own,
    /// `js_injection`'s: its Identity profile at the version of the browser
    /// that reports `product`. Neither without an Identity profile.
    fn primitives(&self, product: Option<&str>) -> (Option<Emulated>, Option<String>) {
        match (self.method, self.identity.as_deref()) {
            (Method::None, _) | (_, None) => (None, None),
            (Method::Basic, Some(identity)) => (Some(shown_at(identity, product)), None),
            (Method::JsInjection, Some(identity)) => {
                (None, init_script(&shown_at(identity, product)))
            }
        }
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
        let unmasked = match (self.method, self.identity.as_deref()) {
            (Method::None, _) => None,
            (_, None) => Some(
                "Its pages show the browser's own values, as `stealth.generated_identity` is off.",
            ),
            // Until the proxy exit gives them (#997).
            (_, Some(identity)) => identity.exit.time_zone.is_none().then_some(
                "Its time zone and geolocation are the machine's own, as no proxy exit gives them values yet.",
            ),
        };
        let warning = RunWarning {
            code: code.to_owned(),
            message: unmasked.map_or_else(
                || message.to_owned(),
                |unmasked| format!("{message} {unmasked}"),
            ),
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
            "runtime_allowed": self.runtime == RuntimeDomain::Allowed,
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
    pub(crate) async fn chosen(&self, run: &BrowserRun, session: &SessionId) {
        let details = self.details(session);
        let mut mark = details.clone();
        mark["action"] = json!(FINGERPRINT_METHOD);
        trace::provenance(&run.trace, mark).await;
        let (warning, once) = self.warning(session);
        let kept = if once {
            run.warnings.warn_once(warning).await
        } else {
            run.warnings.warn(warning).await
        };
        if let Err(error) = kept {
            tracing::warn!(%error, "the warning of a Session's Fingerprint method was not kept");
        }
        if let Some(warning) = self.runtime_warning(session)
            && let Err(error) = run.warnings.warn(warning).await
        {
            tracing::warn!(%error, "the warning of a Session's `Runtime` domain was not kept");
        }
        let origin = Origin {
            actor: Actor::Engine,
            surface: None,
            layer: Some(self.layer()),
            run_id: Some(run.identity.id().clone()),
        };
        if let Err(error) = run
            .engine
            .audit()
            .record(
                &origin,
                &FINGERPRINT_METHOD_CHOSEN,
                details,
                run.identity.clock().timestamp(),
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
        (run, session, page_id): (&BrowserRun, &SessionId, &PageId),
        injections: &Injections,
    ) -> Result<(), goodfetch_engine::CallError> {
        let (values, script) = self.primitives(page.browser_product());
        if let Some(values) = values {
            page.emulate_opening(&values).await?;
            page.emulate_in_context_workers(values);
        }
        if let Some(script) = script {
            page.add_init_script_opening(script).await?;
            injections
                .mark(&run.trace, session, page_id, Injection::FingerprintMethod)
                .await;
        }
        Ok(())
    }
}

/// What a page's out-of-process frames and dedicated workers show, which
/// none of the page's own overrides reach: its browser's identity, then
/// what `plan`'s method has them show.
struct ChildMasks {
    page: CdpPage,
    /// `basic`'s values in a frame: all of the page's but its screen.
    frame: Option<Emulated>,
    /// `basic`'s values in a worker, of which it reads its own.
    worker: Option<Emulated>,
    /// `js_injection`'s script, run in a frame's documents; a worker runs
    /// no page script.
    script: Option<String>,
}

impl ChildMasks {
    /// Has the child on `session`, of kind `kind`, show them.
    async fn show(&self, kind: ChildKind, session: &TargetSession) -> Result<(), CallError> {
        let child = self.page.child(session);
        child.show_identity_opening(kind).await?;
        match kind {
            ChildKind::Frame => {
                if let Some(values) = &self.frame {
                    child.emulate_opening(values).await?;
                }
                if let Some(script) = &self.script {
                    child.add_init_script_opening(script.clone()).await?;
                }
            }
            ChildKind::Worker => {
                if let Some(values) = &self.worker {
                    child.emulate_worker_opening(values).await?;
                }
            }
        }
        Ok(())
    }
}

/// What a page of `page`'s Session does with each of its out-of-process
/// frames and dedicated workers, which `page_children::follow` hands over
/// before it runs: the child shows its headless browser's identity, as its
/// page does, then what the Session's Fingerprint method `plan`, if any,
/// shows on the page (ADR 0035's rows). A child that cannot take them does
/// not run.
pub(crate) fn children(plan: Option<&FingerprintPlan>, page: &CdpPage) -> OnChild {
    let (values, script) =
        plan.map_or((None, None), |plan| plan.primitives(page.browser_product()));
    let masks = Arc::new(ChildMasks {
        page: page.clone(),
        frame: values.as_ref().map(Emulated::in_frame),
        worker: values,
        script,
    });
    Arc::new(move |child: &Child| {
        let (masks, kind, session) = (Arc::clone(&masks), child.kind, child.session.clone());
        Box::pin(async move {
            match masks.show(kind, &session).await {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(?kind, %error, "a page's child could not take its Session's identity");
                    false
                }
            }
        })
    })
}

/// The values a page shows of `identity` at the version of the browser
/// that reports `product`, such as `HeadlessChrome/131.0.6778.85`, or at
/// the drawn version when `product` names none it can read.
fn shown_at(identity: &IdentityProfile, product: Option<&str>) -> Emulated {
    let mut identity = identity.clone();
    if let Some(version) = product
        .and_then(|product| product.split_once('/'))
        .and_then(|(_, version)| BrowserVersion::parse(version))
    {
        refresh_version_fields(&mut identity, version);
    }
    shown(&identity)
}

/// The values a page shows of `identity`: its user agent with its
/// `Accept-Language` header, `navigator.platform` and client hints, its
/// screen, its first language, its core count, and the exit's time zone
/// and geolocation, when it has them.
fn shown(identity: &IdentityProfile) -> Emulated {
    let accept_language = identity
        .http
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
        .map(|(_, value)| value.clone());
    Emulated {
        user_agent: Some(UserAgentOverride {
            user_agent: identity.user_agent.clone(),
            accept_language,
            platform: identity.navigator.platform.clone(),
            client_hints: identity.client_hints.clone(),
        }),
        screen: Some(ScreenSize {
            width: identity.screen.width,
            height: identity.screen.height,
            avail_width: identity.screen.avail_width,
            avail_height: identity.screen.avail_height,
        }),
        time_zone: identity.exit.time_zone.clone(),
        locale: identity.navigator.languages.first().cloned(),
        languages: identity.navigator.languages.clone(),
        geolocation: identity.exit.geolocation.map(|at| Geolocation {
            latitude: at.latitude,
            longitude: at.longitude,
            accuracy: at.accuracy,
        }),
        core_count: Some(identity.navigator.hardware_concurrency),
    }
}

/// `js_injection`'s script, which has a document read `values` in place of
/// its machine's: none when it has none to show. Each value is written as
/// a JSON literal, so no value can break out of the script.
fn init_script(values: &Emulated) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(agent) = &values.user_agent {
        let user_agent = json!(agent.user_agent);
        let platform = json!(agent.platform);
        lines.push(format!(
            "define(Navigator.prototype, \"userAgent\", {user_agent});\n  define(Navigator.prototype, \"platform\", {platform});"
        ));
    }
    if let Some(screen) = values.screen {
        let ScreenSize {
            width,
            height,
            avail_width,
            avail_height,
        } = screen;
        lines.push(format!(
            "define(Screen.prototype, \"width\", {width});\n  define(Screen.prototype, \"height\", {height});\n  define(Screen.prototype, \"availWidth\", {avail_width});\n  define(Screen.prototype, \"availHeight\", {avail_height});"
        ));
    }
    if let Some(cores) = values.core_count {
        lines.push(format!(
            "define(Navigator.prototype, \"hardwareConcurrency\", {cores});"
        ));
    }
    if let Some(locale) = &values.locale {
        let locale_json = json!(locale);
        let languages = if values.languages.is_empty() {
            json!([locale])
        } else {
            json!(values.languages)
        };
        lines.push(format!(
            "define(Navigator.prototype, \"language\", {locale_json});\n  define(Navigator.prototype, \"languages\", Object.freeze({languages}));"
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
