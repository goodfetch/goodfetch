//! The Access mode a Session opens in, and its Identity profile (ADRs 0021,
//! 0035 and 0056; spec #599): [`resolve`] reads `browser.access_mode` and
//! the `stealth.*` items from the Session's own settings and its Run's,
//! with the Session's source and its browser family, and gives the
//! Session's [`SessionIdentityPlan`] or the [`Refusal`] it opens with.
//!
//! A non-`none` `stealth.fingerprint_method` makes the Session `stealth`
//! when set closer than the mode; under `stealth` the items set keep their
//! values and the rest take `stealth`'s defaults. A stealth item set on the
//! Session itself is refused where it would do nothing (ADR 0077's G3); one
//! the Session inherits from its Run yields to the Access mode. So do the
//! `identified.*` items, which an `identified` Session needs both of: the
//! Secret reference of the key it signs its requests with, and where a site
//! reads the key directory (ADR 0045; #1551).
//!
//! `browser.headless_shell` (#3224) is refused where the shell cannot take
//! effect: beside `browser.headless: false`, since the shell runs only
//! headless, and under `stealth`, which launches its own build. A Borrowed
//! browser launches nothing, so the shell yields to it as every other
//! `browser.*` key the Run sets does.

#[cfg(test)]
mod tests;

use goodfetch_engine::run::settings_error;
use goodfetch_engine::settings::{Resolved, SettingsError, Values, catalog_value, value_as};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::page::access_mode::{
    BROWSER_ACCESS_MODE, IDENTIFIED_ITEMS, IDENTIFIED_SIGNATURE_AGENT, IDENTIFIED_SIGNING_KEY,
    STEALTH_CHROMIUM_BUILD, STEALTH_FINGERPRINT_METHOD, STEALTH_GENERATED_IDENTITY,
    STEALTH_HUMAN_INPUT, STEALTH_IDENTITY_BRAND, STEALTH_IDENTITY_DEVICE, STEALTH_IDENTITY_OS,
    STEALTH_ITEMS, masks,
};
use goodfetch_protocol::page::{BROWSER_HEADLESS, BROWSER_HEADLESS_SHELL};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::session_source::{BrowserName, SessionSource};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use serde_json::Value;

/// A Session's Access mode (ADR 0021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessMode {
    /// The browser's own identity.
    Standard,
    /// A declared identity that masks nothing.
    Identified,
    /// The whole stealth catalog, each item of which can be switched off.
    Stealth,
}

impl AccessMode {
    const ALL: [Self; 3] = [Self::Standard, Self::Identified, Self::Stealth];

    /// Its name, as `browser.access_mode` writes it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Identified => "identified",
            Self::Stealth => "stealth",
        }
    }
}

/// How a `stealth` Session masks its browser's fingerprint (ADR 0035).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FingerprintMethod {
    /// No masking.
    None,
    /// Scripts injected into each page.
    JsInjection,
    /// The browser's own switches.
    Basic,
    /// A build of the browser engine that masks it.
    Engine,
}

impl FingerprintMethod {
    const ALL: [Self; 4] = [Self::None, Self::JsInjection, Self::Basic, Self::Engine];

    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::JsInjection => "js_injection",
            Self::Basic => "basic",
            Self::Engine => "engine",
        }
    }
}

/// The Chromium build a `stealth` Session runs (ADR 0035).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChromiumBuild {
    /// Goodfetch's own.
    Goodfetch,
    /// fingerprint-chromium, where `stealth.allowed_builds` allows it.
    FingerprintChromium,
    /// Clearcote, where `stealth.allowed_builds` allows it.
    Clearcote,
}

impl ChromiumBuild {
    const ALL: [Self; 3] = [Self::Goodfetch, Self::FingerprintChromium, Self::Clearcote];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Goodfetch => "goodfetch",
            Self::FingerprintChromium => "fingerprint_chromium",
            Self::Clearcote => "clearcote",
        }
    }
}

/// The value of `setting`, a choice the catalog checked, as one of `all`.
fn choice<T: Copy>(setting: &Setting, value: &Value, all: &[T], name: fn(T) -> &'static str) -> T {
    let written: &str = value_as(setting, value);
    all.iter()
        .copied()
        .find(|candidate| name(*candidate) == written)
        .expect("invariant: the catalog checked the setting's choice")
}

/// The browser family a Session runs on (ADR 0056).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserFamily {
    /// Goodfetch's Chromium, or a Chromium-family browser's own.
    Chromium,
    /// Firefox, run on Camoufox under `stealth`.
    Firefox,
    /// Goodfetch's `WebKit` build, which a Safari source runs on.
    WebKit,
}

impl BrowserFamily {
    /// The family a Session opened from `source` runs on: its browser's for
    /// a source that names one, Goodfetch's Chromium otherwise.
    pub(crate) fn of(source: Option<&SessionSource>) -> Self {
        let browser = match source {
            Some(
                SessionSource::ProfileDirectory { browser, .. }
                | SessionSource::InstalledBrowser { browser, .. }
                | SessionSource::RunningBrowser { browser, .. },
            ) => *browser,
            _ => return Self::Chromium,
        };
        match browser {
            BrowserName::Firefox => Self::Firefox,
            BrowserName::Safari => Self::WebKit,
            BrowserName::Chrome
            | BrowserName::Edge
            | BrowserName::Brave
            | BrowserName::Chromium => Self::Chromium,
        }
    }
}

/// Whether a Session opened from `source` borrows a browser the person runs,
/// which keeps its own identity.
const fn borrowed(source: Option<&SessionSource>) -> bool {
    matches!(
        source,
        Some(SessionSource::RunningBrowser { .. } | SessionSource::CdpUrl { .. })
    )
}

/// Whether `source` carries another browser's identity: an import, or a
/// Profile directory used in place.
const fn carries_an_identity(source: Option<&SessionSource>) -> bool {
    matches!(
        source,
        Some(
            SessionSource::StateFile { .. }
                | SessionSource::CookieHeader { .. }
                | SessionSource::InstalledBrowser { .. }
                | SessionSource::ProfileDirectory { .. }
        )
    )
}

/// Where a setting's value came from, nearest the Session last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    /// Nobody set it.
    Default,
    /// The Run's layer that set it.
    Inherited(SettingsLayer),
    /// The Session's own settings.
    Session,
}

impl Scope {
    /// The layer a refusal names: a Session's own settings are set on the
    /// Run's layer.
    const fn layer(self) -> SettingsLayer {
        match self {
            Self::Default => SettingsLayer::Default,
            Self::Inherited(layer) => layer,
            Self::Session => SettingsLayer::Run,
        }
    }
}

/// A Session's settings: its own, over its Run's.
pub(crate) struct SessionSettings<'a> {
    own: &'a Values,
    run: Option<&'a Resolved>,
}

impl<'a> SessionSettings<'a> {
    /// The Session's `own` settings over its Run's, none for a Run started
    /// without settings.
    pub(crate) const fn new(own: &'a Values, run: Option<&'a Resolved>) -> Self {
        Self { own, run }
    }

    /// `setting`'s value and where it came from.
    fn scoped(&self, setting: &Setting) -> (&'a Value, Scope) {
        if let Some(value) = self.own.get(setting.name) {
            return (value, Scope::Session);
        }
        match self.run.and_then(|run| run.effective(setting)) {
            Some(effective) if effective.layer != SettingsLayer::Default => {
                (&effective.value, Scope::Inherited(effective.layer))
            }
            Some(effective) => (&effective.value, Scope::Default),
            None => (catalog_value(setting), Scope::Default),
        }
    }

    /// `setting`'s value under `stealth`: the one set, or `stealth`'s
    /// default where nobody set it.
    fn under_stealth(&self, setting: &Setting, stealth_default: Value) -> Value {
        match self.scoped(setting) {
            (_, Scope::Default) => stealth_default,
            (value, _) => value.clone(),
        }
    }
}

/// What a Session opens with: its Access mode, the layer that chose it,
/// under `stealth` its Identity profile, and under `identified` what it
/// signs its requests with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionIdentityPlan {
    pub(crate) mode: AccessMode,
    pub(crate) layer: SettingsLayer,
    pub(crate) profile: Option<StealthProfile>,
    pub(crate) signing: Option<SigningPlan>,
}

impl SessionIdentityPlan {
    /// The plan, when this engine builds its Access mode's effects:
    /// `standard` and `identified` open, `stealth` not until the tickets
    /// that build it land (ADR 0083).
    pub(crate) fn built(self) -> Result<Self, Refusal> {
        match self.mode {
            AccessMode::Standard | AccessMode::Identified => Ok(self),
            mode @ AccessMode::Stealth => Err(Refusal::Unbuilt {
                mode,
                layer: self.layer,
            }),
        }
    }
}

/// What an `identified` Session signs its requests with (ADR 0045): the
/// Secret reference of its key, resolved once when the Session opens, the
/// layer that set it, and where a site reads the key directory.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SigningPlan {
    pub(crate) key: SecretReference,
    pub(crate) layer: SettingsLayer,
    pub(crate) agent: String,
}

/// The Identity profile a `stealth` Session runs with (ADR 0035).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StealthProfile {
    pub(crate) fingerprint_method: FingerprintMethod,
    pub(crate) generated_identity: bool,
    pub(crate) os: Vec<String>,
    pub(crate) device: Vec<String>,
    pub(crate) brand: Vec<String>,
    pub(crate) human_input: bool,
    pub(crate) chromium_build: ChromiumBuild,
    pub(crate) headless: bool,
}

/// Why a Session does not open in the Access mode its settings ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A stealth item set on a Session that opens `standard`.
    ItemOnStandard { key: &'static str },
    /// `key` set on a Session that borrows a browser from `source`, which
    /// stays `standard`.
    OnBorrowed {
        key: &'static str,
        source: &'static str,
    },
    /// `identified`, chosen in `layer`, at a `source` that carries another
    /// browser's identity.
    IdentifiedAt {
        layer: SettingsLayer,
        source: &'static str,
    },
    /// An explicit `mode` beside a masking Fingerprint method in `layer`.
    ModeBesideMethod {
        mode: AccessMode,
        layer: SettingsLayer,
    },
    /// A stealth item set on a Session that opens `identified`.
    MaskingOnIdentified { key: &'static str },
    /// An `identified.*` item set on a Session that opens `mode`, which
    /// signs nothing.
    SigningNotIdentified { key: &'static str, mode: AccessMode },
    /// An `identified` Session, chosen in `layer`, without `key`, which it
    /// signs its requests with.
    UnsignedIdentified {
        key: &'static str,
        layer: SettingsLayer,
    },
    /// `stealth`, chosen in `layer`, on Goodfetch's `WebKit` build, which
    /// has no stealth mode.
    StealthOnWebKit { layer: SettingsLayer },
    /// `browser.headless_shell`, set in `layer`, beside `browser.headless:
    /// false`: the shell runs only headless.
    ShellNotHeadless { layer: SettingsLayer },
    /// `browser.headless_shell`, set in `layer`, on a `stealth` Session,
    /// which launches its own build.
    ShellUnderStealth { layer: SettingsLayer },
    /// `mode`, chosen in `layer`, whose effects this engine does not build
    /// yet.
    Unbuilt {
        mode: AccessMode,
        layer: SettingsLayer,
    },
}

impl Refusal {
    /// The failure a Session's opening answers with.
    pub(crate) fn call_error(self, context: &CallContext) -> CallError {
        let access_mode = BROWSER_ACCESS_MODE.name;
        let fingerprint_method = STEALTH_FINGERPRINT_METHOD.name;
        let error = match self {
            Self::ItemOnStandard { key } => SettingsError::NotApplicable {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: format!(
                    "a `standard` Session runs no stealth item: open it with `{access_mode}: stealth`, or leave `{key}` out"
                ),
            },
            Self::OnBorrowed { key, source } => SettingsError::NotApplicable {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: format!(
                    "a Session opened from a `{source}` source borrows the browser, which keeps its own identity as `standard`: leave `{key}` out"
                ),
            },
            Self::IdentifiedAt { layer, source } => SettingsError::NotApplicable {
                key: access_mode.to_owned(),
                layer,
                fix: format!(
                    "`identified` declares a fresh browser's identity, and a `{source}` source carries another's: open it `standard` or `stealth`"
                ),
            },
            Self::ModeBesideMethod { mode, layer } => SettingsError::Invalid {
                key: access_mode.to_owned(),
                layer: Some(layer),
                reason: format!(
                    "`{}` beside a `{fingerprint_method}` other than `none` in the same layer, which makes the Session `stealth`: set `{access_mode}: stealth`, or `{fingerprint_method}: none`",
                    mode.as_str()
                ),
            },
            Self::MaskingOnIdentified { key } => SettingsError::Invalid {
                key: key.to_owned(),
                layer: Some(SettingsLayer::Run),
                reason: format!(
                    "the Session opens with `{access_mode}: identified`, which masks nothing: leave `{key}` out, or open it `stealth`"
                ),
            },
            Self::SigningNotIdentified { key, mode } => SettingsError::NotApplicable {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: format!(
                    "only an `identified` Session signs its requests, and this one opens `{}`: open it with `{access_mode}: identified`, or leave `{key}` out",
                    mode.as_str()
                ),
            },
            Self::UnsignedIdentified { key, layer } => SettingsError::Invalid {
                key: key.to_owned(),
                layer: Some(layer),
                reason: format!(
                    "an `identified` Session signs each request, with `{}` and `{}`: set `{key}`, or open the Session `standard`",
                    IDENTIFIED_SIGNING_KEY.name, IDENTIFIED_SIGNATURE_AGENT.name
                ),
            },
            Self::ShellNotHeadless { layer } => SettingsError::NotApplicable {
                key: BROWSER_HEADLESS_SHELL.name.to_owned(),
                layer,
                fix: format!(
                    "the headless shell runs only headless, and `{}` is `false`: set it to `true`, or leave `{}` out",
                    BROWSER_HEADLESS.name, BROWSER_HEADLESS_SHELL.name
                ),
            },
            Self::ShellUnderStealth { layer } => SettingsError::NotApplicable {
                key: BROWSER_HEADLESS_SHELL.name.to_owned(),
                layer,
                fix: format!(
                    "a `stealth` Session launches its own Chromium build, never the headless shell: leave `{}` out, or open the Session `standard`",
                    BROWSER_HEADLESS_SHELL.name
                ),
            },
            Self::StealthOnWebKit { layer } => {
                return CallError::new(
                    &CAPABILITY_UNAVAILABLE,
                    format!(
                        "`{access_mode}` is `stealth`, chosen in {}, and a Safari source runs on Goodfetch's WebKit, which has no stealth mode",
                        layer.as_str()
                    ),
                    &CapabilityUnavailableDetails {
                        key: access_mode.to_owned(),
                        layer,
                        kind: "webkit".to_owned(),
                        fix: format!("`{access_mode}: standard`"),
                    },
                );
            }
            Self::Unbuilt { mode, layer } => {
                return CallError::new(
                    &CAPABILITY_UNAVAILABLE,
                    format!(
                        "the Session would open `{}`, chosen in {}, and this engine does not build that Access mode's effects yet: leave `{access_mode}` at `standard` and `{fingerprint_method}` at `none`",
                        mode.as_str(),
                        layer.as_str()
                    ),
                    &CapabilityUnavailableDetails {
                        key: access_mode.to_owned(),
                        layer,
                        kind: mode.as_str().to_owned(),
                        fix: format!("`{access_mode}: standard`"),
                    },
                );
            }
        };
        settings_error(&error, context)
    }
}

/// The Access mode and Identity profile a Session opens with, from its
/// `settings`, its `source` and the browser `family` it runs on; or why it
/// does not open.
pub(crate) fn resolve(
    settings: &SessionSettings<'_>,
    source: Option<&SessionSource>,
    family: BrowserFamily,
) -> Result<SessionIdentityPlan, Refusal> {
    let (mode_value, mode_scope) = settings.scoped(&BROWSER_ACCESS_MODE);
    let set_mode = choice(
        &BROWSER_ACCESS_MODE,
        mode_value,
        &AccessMode::ALL,
        AccessMode::as_str,
    );
    let (method_value, method_scope) = settings.scoped(&STEALTH_FINGERPRINT_METHOD);
    let masking = method_scope != Scope::Default && masks(method_value);
    if masking && method_scope == mode_scope && set_mode != AccessMode::Stealth {
        return Err(Refusal::ModeBesideMethod {
            mode: set_mode,
            layer: mode_scope.layer(),
        });
    }
    let (mode, scope) = if masking && method_scope > mode_scope {
        (AccessMode::Stealth, method_scope)
    } else {
        (set_mode, mode_scope)
    };
    let explicit = STEALTH_ITEMS
        .iter()
        .find(|item| settings.scoped(item).1 == Scope::Session)
        .map(|item| item.name);
    let signing_item = IDENTIFIED_ITEMS
        .iter()
        .find(|item| settings.scoped(item).1 == Scope::Session)
        .map(|item| item.name);
    let kind = source.map(SessionSource::kind);

    if let Some(source) = kind.filter(|_| borrowed(source)) {
        if mode_scope == Scope::Session && set_mode != AccessMode::Standard {
            return Err(Refusal::OnBorrowed {
                key: BROWSER_ACCESS_MODE.name,
                source,
            });
        }
        if let Some(key) = explicit.or(signing_item) {
            return Err(Refusal::OnBorrowed { key, source });
        }
        return Ok(SessionIdentityPlan {
            mode: AccessMode::Standard,
            layer: SettingsLayer::Default,
            profile: None,
            signing: None,
        });
    }
    match (mode, explicit) {
        (AccessMode::Standard, Some(key)) => return Err(Refusal::ItemOnStandard { key }),
        (AccessMode::Identified, Some(key)) => return Err(Refusal::MaskingOnIdentified { key }),
        _ => {}
    }
    if let Some(key) = signing_item.filter(|_| mode != AccessMode::Identified) {
        return Err(Refusal::SigningNotIdentified { key, mode });
    }
    let layer = scope.layer();
    let (shell, shell_scope) = settings.scoped(&BROWSER_HEADLESS_SHELL);
    if shell == &Value::Bool(true) {
        let shell = shell_scope.layer();
        if mode == AccessMode::Stealth {
            return Err(Refusal::ShellUnderStealth { layer: shell });
        }
        if settings.scoped(&BROWSER_HEADLESS).0 == &Value::Bool(false) {
            return Err(Refusal::ShellNotHeadless { layer: shell });
        }
    }
    match mode {
        AccessMode::Standard => Ok(SessionIdentityPlan {
            mode,
            layer,
            profile: None,
            signing: None,
        }),
        AccessMode::Identified => match kind.filter(|_| carries_an_identity(source)) {
            Some(source) => Err(Refusal::IdentifiedAt { layer, source }),
            None => Ok(SessionIdentityPlan {
                mode,
                layer,
                profile: None,
                signing: Some(signing_plan(settings, layer)?),
            }),
        },
        AccessMode::Stealth if family == BrowserFamily::WebKit => {
            Err(Refusal::StealthOnWebKit { layer })
        }
        AccessMode::Stealth => Ok(SessionIdentityPlan {
            mode,
            layer,
            profile: Some(stealth_profile(settings)),
            signing: None,
        }),
    }
}

/// What an `identified` Session, chosen in `layer`, signs with: both
/// `identified.*` items, or a refusal naming the first one missing.
fn signing_plan(
    settings: &SessionSettings<'_>,
    layer: SettingsLayer,
) -> Result<SigningPlan, Refusal> {
    let missing = |key: &'static str| Refusal::UnsignedIdentified { key, layer };
    let (key, key_scope) = settings.scoped(&IDENTIFIED_SIGNING_KEY);
    let key: Option<SecretReference> = value_as(&IDENTIFIED_SIGNING_KEY, key);
    let key = key.ok_or_else(|| missing(IDENTIFIED_SIGNING_KEY.name))?;
    let agent = settings
        .scoped(&IDENTIFIED_SIGNATURE_AGENT)
        .0
        .as_str()
        .ok_or_else(|| missing(IDENTIFIED_SIGNATURE_AGENT.name))?;
    Ok(SigningPlan {
        key,
        layer: key_scope.layer(),
        agent: agent.to_owned(),
    })
}

/// A `stealth` Session's Identity profile: the items set keep their values,
/// the rest take `stealth`'s defaults.
fn stealth_profile(settings: &SessionSettings<'_>) -> StealthProfile {
    let on = || Value::Bool(true);
    let method = settings.under_stealth(
        &STEALTH_FINGERPRINT_METHOD,
        Value::String(FingerprintMethod::Engine.as_str().to_owned()),
    );
    let build = settings.under_stealth(
        &STEALTH_CHROMIUM_BUILD,
        catalog_value(&STEALTH_CHROMIUM_BUILD).clone(),
    );
    let list = |setting: &Setting| -> Vec<String> {
        settings
            .under_stealth(setting, catalog_value(setting).clone())
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    StealthProfile {
        fingerprint_method: choice(
            &STEALTH_FINGERPRINT_METHOD,
            &method,
            &FingerprintMethod::ALL,
            FingerprintMethod::as_str,
        ),
        generated_identity: value_as(
            &STEALTH_GENERATED_IDENTITY,
            &settings.under_stealth(&STEALTH_GENERATED_IDENTITY, on()),
        ),
        os: list(&STEALTH_IDENTITY_OS),
        device: list(&STEALTH_IDENTITY_DEVICE),
        brand: list(&STEALTH_IDENTITY_BRAND),
        human_input: value_as(
            &STEALTH_HUMAN_INPUT,
            &settings.under_stealth(&STEALTH_HUMAN_INPUT, on()),
        ),
        chromium_build: choice(
            &STEALTH_CHROMIUM_BUILD,
            &build,
            &ChromiumBuild::ALL,
            ChromiumBuild::as_str,
        ),
        headless: value_as(
            &BROWSER_HEADLESS,
            &settings.under_stealth(&BROWSER_HEADLESS, Value::Bool(false)),
        ),
    }
}
