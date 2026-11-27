//! A Session's device emulation, network throttling, and permissions plan
//! (ADR 0083; spec #578, tickets #3289 and #3290): [`resolve`] reads the
//! `browser.*` emulation settings, `browser.permissions`, and
//! `network.throttle` from the Session's own settings over its Run's, with
//! its `session.source` and browser family, and produces a
//! [`SessionEmulation`] or the [`Refusal`] the Session's opening answers
//! with.
//!
//! - On a Borrowed browser (`cdp_url`, `running_browser`), any Session
//!   emulation, permission, or throttling key (`EMULATION_SETTINGS`) set
//!   where the Session opens is refused with `setting_not_applicable`;
//!   inherited values yield, and no emulation is applied to the person's
//!   browser.
//! - Outside Chromium (`Firefox`, `WebKit`), `browser.is_mobile: true` and
//!   a non-null `network.throttle` raise `capability_unavailable`.
//! - `browser.viewport: null` uses the window's own size and is refused
//!   with `setting_not_applicable` on a headless Session.
//! - Clipboard permissions (`clipboard-read`, `clipboard-write`) are
//!   refused with `setting_not_applicable` on a headful Session.
//! - On an imported login or a Profile directory used in place, a non-null
//!   `browser.locale` or `browser.timezone_id` set where the Session opens
//!   is refused with `setting_locked` naming `session.source`; an inherited
//!   one yields so the source's identity stands.

#[cfg(test)]
mod tests;

use goodfetch_engine::run::settings_error;
use goodfetch_engine::settings::{SettingsError, value_as};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::page::BROWSER_HEADLESS;
use goodfetch_protocol::page::emulation::{
    BROWSER_COLOR_SCHEME, BROWSER_DEVICE_SCALE_FACTOR, BROWSER_GEOLOCATION, BROWSER_HAS_TOUCH,
    BROWSER_IS_MOBILE, BROWSER_JAVA_SCRIPT_ENABLED, BROWSER_LOCALE, BROWSER_PERMISSIONS,
    BROWSER_TIMEZONE_ID, BROWSER_VIEWPORT, BrowserPermission, DeviceScaleFactor,
    EMULATION_SETTINGS, Geolocation, NETWORK_THROTTLE, NetworkThrottle, Viewport,
};
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SESSION_SOURCE, SettingLimit,
    SettingsLayer,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::access_mode::{BrowserFamily, Scope, SessionSettings, borrowed, carries_an_identity};

/// Deserializes a setting value the catalog has already validated.
fn decoded<T: DeserializeOwned>(setting: &Setting, value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap_or_else(|error| {
        panic!("invariant: the catalog checked `{}`: {error}", setting.name)
    })
}

/// Emulated `prefers-color-scheme` media feature (spec #578).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ColorScheme {
    #[default]
    Light,
    Dark,
    NoPreference,
}

impl ColorScheme {
    /// The value passed to CDP's `Emulation.setEmulatedMedia` feature
    /// `prefers-color-scheme`. An empty string clears the override.
    pub(crate) const fn cdp_value(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::NoPreference => "",
        }
    }
}

/// Whether a Session's browser is borrowed (untouched), headless, or headful.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SessionMode {
    /// A Borrowed browser keeps its own device metrics and identity.
    Borrowed,
    /// A managed headless browser (`browser.headless: true`).
    #[default]
    Headless,
    /// A managed headful browser (`browser.headless: false`).
    Headful,
}

/// The resolved device emulation and network throttling a Session applies
/// to every page it opens and re-applies after a Lent page's attach ends.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionEmulation {
    /// Viewport size in CSS pixels; `None` uses the window's own size
    /// (headful only) or leaves a Borrowed browser untouched.
    pub(crate) viewport: Option<Viewport>,
    /// Device pixel ratio (`window.devicePixelRatio`).
    pub(crate) device_scale_factor: DeviceScaleFactor,
    /// BCP 47 locale tag, if explicitly overridden.
    pub(crate) locale: Option<String>,
    /// IANA time zone ID, if explicitly overridden.
    pub(crate) timezone_id: Option<String>,
    /// Emulated `prefers-color-scheme`.
    pub(crate) color_scheme: ColorScheme,
    /// Whether page JavaScript execution is enabled.
    pub(crate) java_script_enabled: bool,
    /// Emulated geolocation coordinates, if set.
    pub(crate) geolocation: Option<Geolocation>,
    /// Whether touch events and `navigator.maxTouchPoints` are emulated.
    pub(crate) has_touch: bool,
    /// Whether mobile viewport layout is emulated (Chromium only).
    pub(crate) is_mobile: bool,
    /// Playwright permissions granted to every page of the Session.
    pub(crate) permissions: Vec<BrowserPermission>,
    /// Emulated network conditions (Chromium only).
    pub(crate) throttle: Option<NetworkThrottle>,
    /// Whether the Session's browser is borrowed, headless, or headful.
    pub(crate) mode: SessionMode,
}

impl Default for SessionEmulation {
    fn default() -> Self {
        Self {
            viewport: Some(Viewport::DEFAULT),
            device_scale_factor: DeviceScaleFactor::DEFAULT,
            locale: None,
            timezone_id: None,
            color_scheme: ColorScheme::Light,
            java_script_enabled: true,
            geolocation: None,
            has_touch: false,
            is_mobile: false,
            permissions: Vec::new(),
            throttle: None,
            mode: SessionMode::Headless,
        }
    }
}

impl SessionEmulation {
    /// A Borrowed browser keeps its own device metrics and identity.
    pub(crate) fn borrowed() -> Self {
        Self {
            viewport: None,
            mode: SessionMode::Borrowed,
            ..Self::default()
        }
    }

    /// Whether the Session borrows an existing browser (no emulation sent).
    pub(crate) const fn is_borrowed(&self) -> bool {
        matches!(self.mode, SessionMode::Borrowed)
    }

    /// Whether `Emulation.setDeviceMetricsOverride` is needed when preparing
    /// a freshly opened page target: at default headless settings
    /// (`1280x720`, scale `1`, desktop) the launch `--window-size` sets the
    /// default window size with zero extra CDP commands (C7).
    pub(crate) fn needs_device_metrics_on_open(&self) -> bool {
        if self.is_borrowed() {
            return false;
        }
        let non_default_scale = !self.device_scale_factor.is_default();
        match self.viewport {
            Some(viewport) => {
                !matches!(self.mode, SessionMode::Headless)
                    || viewport != Viewport::DEFAULT
                    || non_default_scale
                    || self.is_mobile
            }
            None => non_default_scale || self.is_mobile,
        }
    }
}

/// Why a Session's emulation settings cannot open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Refusal {
    /// A Session emulation, permission, or throttling setting set where a
    /// Borrowed browser's Session opens.
    OnBorrowed {
        key: &'static str,
        source: &'static str,
    },
    /// A Chromium-only setting (`browser.is_mobile`, `network.throttle`)
    /// requested on another browser family.
    ChromiumOnly {
        key: &'static str,
        layer: SettingsLayer,
        family: BrowserFamily,
    },
    /// `browser.viewport: null` requested on a headless browser.
    NullViewportHeadless { layer: SettingsLayer },
    /// Clipboard permission (`clipboard-read` or `clipboard-write`) requested
    /// on a headful Session (`browser.headless: false`).
    ClipboardHeadful {
        permission: BrowserPermission,
        layer: SettingsLayer,
    },
    /// `browser.locale` or `browser.timezone_id` set where a Session opens
    /// from a source that fixes its identity.
    LockedBySource {
        key: &'static str,
        source: Value,
        source_kind: &'static str,
        source_layer: SettingsLayer,
    },
    /// A text emulation setting (`browser.locale` or `browser.timezone_id`)
    /// was given an empty or whitespace-only string.
    BlankText {
        key: &'static str,
        layer: SettingsLayer,
    },
}

impl Refusal {
    /// Converts this refusal into the wire `CallError`.
    pub(crate) fn call_error(self, context: &CallContext) -> CallError {
        match self {
            Self::OnBorrowed { key, source } => settings_error(
                &SettingsError::NotApplicable {
                    key: key.to_owned(),
                    layer: SettingsLayer::Run,
                    fix: format!(
                        "a Session opened from a `{source}` source borrows the browser, which keeps its own device and network settings: leave `{key}` out"
                    ),
                },
                context,
            ),
            Self::ChromiumOnly { key, layer, family } => {
                let family_name = match family {
                    BrowserFamily::Chromium => "chromium",
                    BrowserFamily::Firefox => "firefox",
                    BrowserFamily::WebKit => "webkit",
                };
                CallError::new(
                    &CAPABILITY_UNAVAILABLE,
                    format!(
                        "`{key}` (set in {}) is only available on Chromium, not on `{family_name}`",
                        layer.as_str()
                    ),
                    &CapabilityUnavailableDetails {
                        key: key.to_owned(),
                        layer,
                        kind: format!("{key}_on_{family_name}"),
                        fix: format!("leave `{key}` unset or use a Chromium Session"),
                    },
                )
            }
            Self::NullViewportHeadless { layer } => settings_error(
                &SettingsError::NotApplicable {
                    key: BROWSER_VIEWPORT.name.to_owned(),
                    layer,
                    fix: "set `browser.headless: false` to use the window's own size, or pass an explicit `{ width, height }` viewport".to_owned(),
                },
                context,
            ),
            Self::ClipboardHeadful { permission, layer } => settings_error(
                &SettingsError::NotApplicable {
                    key: BROWSER_PERMISSIONS.name.to_owned(),
                    layer,
                    fix: format!(
                        "`{}` is not granted in a headful browser (`browser.headless: false`), whose clipboard is the person's own desktop clipboard: remove `{}` from `browser.permissions` or run headless",
                        permission.as_str(),
                        permission.as_str(),
                    ),
                },
                context,
            ),
            Self::LockedBySource {
                key,
                source,
                source_kind,
                source_layer,
            } => settings_error(
                &SettingsError::Locked {
                    key: key.to_owned(),
                    layer: SettingsLayer::Run,
                    fix: format!(
                        "leave `{key}` out of the Session's settings: `{}` ({source_kind}) in `{}` fixes the Session's locale and time zone",
                        SESSION_SOURCE.name,
                        source_layer.as_str()
                    ),
                    limit: SettingLimit {
                        key: SESSION_SOURCE.name.to_owned(),
                        value: source,
                        layer: source_layer,
                        origin: None,
                    },
                },
                context,
            ),
            Self::BlankText { key, layer } => settings_error(
                &SettingsError::Invalid {
                    key: key.to_owned(),
                    layer: Some(layer),
                    reason: "expected a non-empty value, or `null` to clear".to_owned(),
                },
                context,
            ),
        }
    }
}

/// Resolves the Session's emulation and throttling settings from `settings`
/// (its own over its Run's), its `source` (and the layer that named it) and
/// its browser `family`.
pub(crate) fn resolve(
    settings: &SessionSettings<'_>,
    source: Option<&SessionSource>,
    source_layer: Option<SettingsLayer>,
    family: BrowserFamily,
) -> Result<SessionEmulation, Refusal> {
    if borrowed(source) {
        let source_kind = source.map_or("borrowed", SessionSource::kind);
        if let Some(key) = EMULATION_SETTINGS
            .iter()
            .find(|setting| settings.scoped(setting).1 == Scope::Session)
            .map(|setting| setting.name)
        {
            return Err(Refusal::OnBorrowed {
                key,
                source: source_kind,
            });
        }
        return Ok(SessionEmulation::borrowed());
    }

    let headless: bool = value_as(&BROWSER_HEADLESS, settings.scoped(&BROWSER_HEADLESS).0);
    let (is_mobile, throttle) = resolve_chromium_only(settings, family)?;
    let viewport = resolve_viewport(settings, headless)?;
    let device_scale_factor: DeviceScaleFactor = decoded(
        &BROWSER_DEVICE_SCALE_FACTOR,
        settings.scoped(&BROWSER_DEVICE_SCALE_FACTOR).0,
    );
    let locale = resolve_identity_text(settings, &BROWSER_LOCALE, source, source_layer)?;
    let timezone_id = resolve_identity_text(settings, &BROWSER_TIMEZONE_ID, source, source_layer)?;
    let color_scheme: ColorScheme = decoded(
        &BROWSER_COLOR_SCHEME,
        settings.scoped(&BROWSER_COLOR_SCHEME).0,
    );
    let java_script_enabled: bool = value_as(
        &BROWSER_JAVA_SCRIPT_ENABLED,
        settings.scoped(&BROWSER_JAVA_SCRIPT_ENABLED).0,
    );
    let geolocation: Option<Geolocation> = decoded(
        &BROWSER_GEOLOCATION,
        settings.scoped(&BROWSER_GEOLOCATION).0,
    );
    let has_touch: bool = value_as(&BROWSER_HAS_TOUCH, settings.scoped(&BROWSER_HAS_TOUCH).0);
    let permissions = resolve_permissions(settings, headless)?;

    Ok(SessionEmulation {
        viewport,
        device_scale_factor,
        locale,
        timezone_id,
        color_scheme,
        java_script_enabled,
        geolocation,
        has_touch,
        is_mobile,
        permissions,
        throttle,
        mode: if headless {
            SessionMode::Headless
        } else {
            SessionMode::Headful
        },
    })
}

fn resolve_chromium_only(
    settings: &SessionSettings<'_>,
    family: BrowserFamily,
) -> Result<(bool, Option<NetworkThrottle>), Refusal> {
    let (is_mobile_val, is_mobile_scope) = settings.scoped(&BROWSER_IS_MOBILE);
    let is_mobile: bool = value_as(&BROWSER_IS_MOBILE, is_mobile_val);
    if is_mobile && family != BrowserFamily::Chromium {
        return Err(Refusal::ChromiumOnly {
            key: BROWSER_IS_MOBILE.name,
            layer: is_mobile_scope.layer(),
            family,
        });
    }

    let (throttle_val, throttle_scope) = settings.scoped(&NETWORK_THROTTLE);
    let throttle: Option<NetworkThrottle> = decoded(&NETWORK_THROTTLE, throttle_val);
    if throttle.is_some() && family != BrowserFamily::Chromium {
        return Err(Refusal::ChromiumOnly {
            key: NETWORK_THROTTLE.name,
            layer: throttle_scope.layer(),
            family,
        });
    }
    Ok((is_mobile, throttle))
}

fn resolve_viewport(
    settings: &SessionSettings<'_>,
    headless: bool,
) -> Result<Option<Viewport>, Refusal> {
    let (viewport_val, viewport_scope) = settings.scoped(&BROWSER_VIEWPORT);
    let viewport: Option<Viewport> = decoded(&BROWSER_VIEWPORT, viewport_val);
    if viewport.is_none() && headless {
        return Err(Refusal::NullViewportHeadless {
            layer: viewport_scope.layer(),
        });
    }
    Ok(viewport)
}

fn resolve_permissions(
    settings: &SessionSettings<'_>,
    headless: bool,
) -> Result<Vec<BrowserPermission>, Refusal> {
    let (permissions_val, permissions_scope) = settings.scoped(&BROWSER_PERMISSIONS);
    let declared: Vec<BrowserPermission> = decoded(&BROWSER_PERMISSIONS, permissions_val);
    let mut permissions = Vec::with_capacity(declared.len());
    for permission in declared {
        if !headless && permission.is_clipboard() {
            return Err(Refusal::ClipboardHeadful {
                permission,
                layer: permissions_scope.layer(),
            });
        }
        if !permissions.contains(&permission) {
            permissions.push(permission);
        }
    }
    Ok(permissions)
}

/// Resolves `browser.locale` or `browser.timezone_id`: rejects blank strings,
/// locks against an imported login or Profile directory used in place when
/// set directly where the Session opens, and yields (`None`) when inherited.
fn resolve_identity_text(
    settings: &SessionSettings<'_>,
    setting: &Setting,
    source: Option<&SessionSource>,
    source_layer: Option<SettingsLayer>,
) -> Result<Option<String>, Refusal> {
    let (value, scope) = settings.scoped(setting);
    let Some(text): Option<&str> = value_as(setting, value) else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Err(Refusal::BlankText {
            key: setting.name,
            layer: scope.layer(),
        });
    }
    if carries_an_identity(source) {
        if scope == Scope::Session {
            let source_ref = source.expect("invariant: `carries_an_identity` implies Some");
            return Err(Refusal::LockedBySource {
                key: setting.name,
                source: serde_json::to_value(source_ref)
                    .expect("invariant: SessionSource serializes to JSON"),
                source_kind: source_ref.kind(),
                source_layer: source_layer.unwrap_or(SettingsLayer::Run),
            });
        }
        return Ok(None);
    }
    Ok(Some(text.to_owned()))
}
