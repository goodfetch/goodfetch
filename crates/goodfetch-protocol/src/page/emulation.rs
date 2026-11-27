//! Session device emulation and network throttling settings (ADRs 0083 and
//! 0092; spec #578, ticket #3289): set where a Session opens or above,
//! never per page (`setting_not_applicable`), and fixed for the Session's
//! life. `browser.locale` and `browser.timezone_id` yield when inherited to
//! an imported login or a Profile directory used in place, which
//! `config explain` names in their `yields_to`, and conflict with one when
//! set where the Session opens (`setting_locked`).

use std::fmt;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ARRIVED_IN;
use crate::catalog::{Authority, Entries, Setting, SettingJson, ValueKind, ValueShape, Yielding};
use crate::settings::SESSION_SOURCE;

/// The color schemes `browser.color_scheme` takes, Playwright's (spec #578).
pub const COLOR_SCHEMES: &[&str] = &["light", "dark", "no-preference"];

/// The permission names `browser.permissions` accepts, Playwright's (ADR 0083;
/// spec #578, ticket #3290).
pub const BROWSER_PERMISSION_NAMES: &[&str] = &[
    "geolocation",
    "midi",
    "midi-sysex",
    "notifications",
    "camera",
    "microphone",
    "background-sync",
    "ambient-light-sensor",
    "accelerometer",
    "gyroscope",
    "magnetometer",
    "clipboard-read",
    "clipboard-write",
    "payment-handler",
    "storage-access",
    "local-fonts",
    "screen-wake-lock",
    "idle-detection",
    "periodic-background-sync",
    "window-management",
    "local-network-access",
];

/// A page's viewport size in CSS pixels (spec #578, SA-23): both dimensions
/// at least 1. Default is 1280×720, Playwright's; `null` clears it for the
/// window's own size in a headful browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    /// Width in CSS pixels, at least 1.
    #[schemars(range(min = 1), extend("arrived_in" = "0.1"))]
    pub width: u32,
    /// Height in CSS pixels, at least 1.
    #[schemars(range(min = 1), extend("arrived_in" = "0.1"))]
    pub height: u32,
}

impl Viewport {
    /// The default viewport, 1280×720 (Playwright's).
    pub const DEFAULT: Self = Self {
        width: 1280,
        height: 720,
    };
}

impl SettingJson for Viewport {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// `browser.viewport`'s values: a [`Viewport`].
pub const VIEWPORT_SHAPE: ValueShape = ValueShape {
    name: "Viewport",
    check: |value| {
        let viewport: Viewport = serde_json::from_value(value.clone()).map_err(|error| {
            format!("expected a viewport with positive integer `width` and `height`: {error}")
        })?;
        if viewport.width == 0 || viewport.height == 0 {
            return Err("viewport `width` and `height` must each be at least 1".to_owned());
        }
        Ok(())
    },
    schema: shape_schema::<Viewport>,
    examples: || vec![json!({ "width": 1280, "height": 720 })],
};

/// A positive finite device pixel ratio for `browser.device_scale_factor`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct DeviceScaleFactor(f64);

/// A number that is not a positive finite device pixel ratio.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("`{0}` is no positive finite device scale factor")]
pub struct InvalidDeviceScaleFactor(pub f64);

impl DeviceScaleFactor {
    /// The default device scale factor, `1`.
    pub const DEFAULT: Self = Self(1.0);

    /// Validates `factor` as a positive finite device scale factor.
    ///
    /// # Errors
    ///
    /// [`InvalidDeviceScaleFactor`] when `factor` is not finite or not
    /// strictly greater than 0.
    pub fn new(factor: f64) -> Result<Self, InvalidDeviceScaleFactor> {
        if factor.is_finite() && factor > 0.0 {
            Ok(Self(factor))
        } else {
            Err(InvalidDeviceScaleFactor(factor))
        }
    }

    /// The numeric scale factor.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for DeviceScaleFactor {
    type Error = InvalidDeviceScaleFactor;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DeviceScaleFactor> for f64 {
    fn from(factor: DeviceScaleFactor) -> Self {
        factor.0
    }
}

impl fmt::Display for DeviceScaleFactor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl SettingJson for DeviceScaleFactor {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// `browser.device_scale_factor`'s values: a [`DeviceScaleFactor`].
pub const DEVICE_SCALE_FACTOR_SHAPE: ValueShape = ValueShape {
    name: "DeviceScaleFactor",
    check: |value| match value.as_f64() {
        Some(factor) => DeviceScaleFactor::new(factor)
            .map(drop)
            .map_err(|error| error.to_string()),
        None => Err("expected a positive number".to_owned()),
    },
    schema: || {
        json!({
            "type": "number",
            "exclusiveMinimum": 0.0,
            "description": "The page's device pixel ratio (`window.devicePixelRatio`): a positive number, `1` by default.",
        })
    },
    examples: || vec![json!(1.5)],
};

/// Coordinates emulated for `browser.geolocation` (spec #578, Playwright's
/// shape).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Geolocation {
    /// Latitude in degrees, from -90 to 90.
    #[schemars(range(min = -90.0, max = 90.0), extend("arrived_in" = "0.1"))]
    pub latitude: f64,
    /// Longitude in degrees, from -180 to 180.
    #[schemars(range(min = -180.0, max = 180.0), extend("arrived_in" = "0.1"))]
    pub longitude: f64,
    /// Non-negative accuracy in meters; `0` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0), extend("arrived_in" = "0.1"))]
    pub accuracy: Option<f64>,
}

impl SettingJson for Geolocation {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// `browser.geolocation`'s values: a [`Geolocation`].
pub const GEOLOCATION_SHAPE: ValueShape = ValueShape {
    name: "Geolocation",
    check: |value| {
        let geo: Geolocation = serde_json::from_value(value.clone()).map_err(|error| {
            format!("expected geolocation coordinates (`latitude`, `longitude`, optional `accuracy`): {error}")
        })?;
        if !geo.latitude.is_finite() || !(-90.0..=90.0).contains(&geo.latitude) {
            return Err("`latitude` must be between -90 and 90".to_owned());
        }
        if !geo.longitude.is_finite() || !(-180.0..=180.0).contains(&geo.longitude) {
            return Err("`longitude` must be between -180 and 180".to_owned());
        }
        if let Some(accuracy) = geo.accuracy
            && (!accuracy.is_finite() || accuracy < 0.0)
        {
            return Err("`accuracy` must be a non-negative finite number".to_owned());
        }
        Ok(())
    },
    schema: shape_schema::<Geolocation>,
    examples: || {
        vec![json!({
            "latitude": 48.8566,
            "longitude": 2.3522,
            "accuracy": 10.0,
        })]
    },
};

/// Chromium network throttling conditions for `network.throttle` (ADR 0083;
/// spec #578).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkThrottle {
    /// Whether the browser appears offline; `false` by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub offline: bool,
    /// Minimum additional round-trip latency in milliseconds; `0` by default.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub latency: u64,
    /// Maximum download throughput in bytes per second; unthrottled when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub download_bytes_per_second: Option<u64>,
    /// Maximum upload throughput in bytes per second; unthrottled when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub upload_bytes_per_second: Option<u64>,
}

impl SettingJson for NetworkThrottle {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// `network.throttle`'s values: a [`NetworkThrottle`].
pub const NETWORK_THROTTLE_SHAPE: ValueShape = ValueShape {
    name: "NetworkThrottle",
    check: |value| {
        serde_json::from_value::<NetworkThrottle>(value.clone())
            .map(drop)
            .map_err(|error| {
                format!(
                    "expected network throttle conditions (`offline`, `latency`, `download_bytes_per_second`, `upload_bytes_per_second`): {error}"
                )
            })
    },
    schema: shape_schema::<NetworkThrottle>,
    examples: || {
        vec![json!({
            "offline": false,
            "latency": 150,
            "download_bytes_per_second": 50_000,
            "upload_bytes_per_second": 20_000,
        })]
    },
};

fn shape_schema<T: JsonSchema>() -> Value {
    let schema = SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<T>();
    let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    schema
}

fn default_viewport() -> Value {
    json!({
        "width": Viewport::DEFAULT.width,
        "height": Viewport::DEFAULT.height,
    })
}

fn default_scale() -> Value {
    json!(1)
}

fn light() -> Value {
    Value::String("light".to_owned())
}

fn on() -> Value {
    Value::Bool(true)
}

fn off() -> Value {
    Value::Bool(false)
}

fn null() -> Value {
    Value::Null
}

fn empty_list() -> Value {
    Value::Array(Vec::new())
}

/// `browser.viewport` (ADR 0083; spec #578, SA-23).
pub const BROWSER_VIEWPORT: Setting = Setting {
    name: "browser.viewport",
    description: "The viewport size in CSS pixels for every page of the Session: `{ width: 1280, height: 720 }` by default, Playwright's. `null` uses the window's own size and applies only to a headful browser (`browser.headless: false`). Set where a Session opens or above, never per page (`setting_not_applicable`), and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Shape(&VIEWPORT_SHAPE),
    default: default_viewport,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.device_scale_factor` (ADR 0083; spec #578, SA-23).
pub const BROWSER_DEVICE_SCALE_FACTOR: Setting = Setting {
    name: "browser.device_scale_factor",
    description: "The device pixel ratio (`window.devicePixelRatio`) for every page of the Session: `1` by default. Set where a Session opens or above, never per page (`setting_not_applicable`), and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Shape(&DEVICE_SCALE_FACTOR_SHAPE),
    default: default_scale,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.locale` (ADRs 0021 and 0083; spec #578).
pub const BROWSER_LOCALE: Setting = Setting {
    name: "browser.locale",
    description: "The BCP 47 locale tag emulated for every page of the Session, such as `fr-FR`; `null` by default, matched to the proxy exit or the Session source's for an import. Set where a Session opens or above, never per page. Refused with `setting_locked` where an imported login or a Profile directory used in place fixes it, yielding when inherited; refused with `setting_not_applicable` where a Borrowed browser's Session opens.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.timezone_id` (ADRs 0021 and 0083; spec #578).
pub const BROWSER_TIMEZONE_ID: Setting = Setting {
    name: "browser.timezone_id",
    description: "The IANA time zone identifier emulated for every page of the Session, such as `Europe/Paris`; `null` by default, matched to the proxy exit or the Session source's for an import. Set where a Session opens or above, never per page. Refused with `setting_locked` where an imported login or a Profile directory used in place fixes it, yielding when inherited; refused with `setting_not_applicable` where a Borrowed browser's Session opens.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.color_scheme` (ADR 0083; spec #578).
pub const BROWSER_COLOR_SCHEME: Setting = Setting {
    name: "browser.color_scheme",
    description: "The `prefers-color-scheme` media feature emulated for every page of the Session: `light` (the default, Playwright's), `dark`, or `no-preference`. Set where a Session opens or above, never per page, and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Choice(COLOR_SCHEMES),
    default: light,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.java_script_enabled` (ADR 0083; spec #578).
pub const BROWSER_JAVA_SCRIPT_ENABLED: Setting = Setting {
    name: "browser.java_script_enabled",
    description: "Whether page scripts run in the Session's pages: `true` by default. Set where a Session opens or above, never per page, and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Boolean,
    default: on,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.geolocation` (ADR 0083; spec #578).
pub const BROWSER_GEOLOCATION: Setting = Setting {
    name: "browser.geolocation",
    description: "The geographic coordinates emulated for every page of the Session; `null` by default. Pages observe the coordinates when `geolocation` is granted in `browser.permissions`. Set where a Session opens or above, never per page, and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Shape(&GEOLOCATION_SHAPE),
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.has_touch` (ADR 0083; spec #578, SA-23).
pub const BROWSER_HAS_TOUCH: Setting = Setting {
    name: "browser.has_touch",
    description: "Whether the Session's pages emulate a touch screen (`navigator.maxTouchPoints` and touch events): `false` by default (Playwright's `hasTouch`). Set where a Session opens or above, never per page, and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.is_mobile` (ADR 0083; spec #578, SA-23).
pub const BROWSER_IS_MOBILE: Setting = Setting {
    name: "browser.is_mobile",
    description: "Whether the Session's pages emulate a mobile viewport (`meta viewport` tag and mobile layout): `false` by default (Playwright's `isMobile`). Chromium only: `true` outside Chromium raises `capability_unavailable`. Set where a Session opens or above, never per page, and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `browser.permissions` (ADR 0083; spec #578, ticket #3290).
pub const BROWSER_PERMISSIONS: Setting = Setting {
    name: "browser.permissions",
    description: "Browser permissions (Playwright's names) granted to every page of the Session; `[]` by default. Clipboard permissions (`clipboard-read`, `clipboard-write`) are refused with `setting_not_applicable` on a headful Session (`browser.headless: false`) so the engine does not touch the person's desktop clipboard. Set where a Session opens or above, never per page, and refused where a Borrowed browser's Session opens.",
    kind: ValueKind::List(&ValueKind::Choice(BROWSER_PERMISSION_NAMES)),
    default: empty_list,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `network.throttle` (ADR 0083; spec #578).
pub const NETWORK_THROTTLE: Setting = Setting {
    name: "network.throttle",
    description: "Emulated network conditions (`offline`, `latency` in milliseconds, optional `download_bytes_per_second` and `upload_bytes_per_second`) applied to every page of the Session; `null` by default. Chromium only: set outside Chromium it raises `capability_unavailable`. Set where a Session opens or above, never per page.",
    kind: ValueKind::Shape(&NETWORK_THROTTLE_SHAPE),
    default: null,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The Session emulation and permission settings that a Session keeps for its
/// life (ADR 0083).
pub const EMULATION_SETTINGS: [&Setting; 11] = [
    &BROWSER_VIEWPORT,
    &BROWSER_DEVICE_SCALE_FACTOR,
    &BROWSER_LOCALE,
    &BROWSER_TIMEZONE_ID,
    &BROWSER_COLOR_SCHEME,
    &BROWSER_JAVA_SCRIPT_ENABLED,
    &BROWSER_GEOLOCATION,
    &BROWSER_HAS_TOUCH,
    &BROWSER_IS_MOBILE,
    &BROWSER_PERMISSIONS,
    &NETWORK_THROTTLE,
];

fn set(value: &Value) -> bool {
    !value.is_null()
}

/// Whether `source` (the JSON value of `session.source`) is an imported
/// login or a Profile directory used in place, whose identity fixes
/// `browser.locale` and `browser.timezone_id` (ADRs 0021, 0056 and 0083).
fn fixes_locale_and_timezone(source: &Value) -> bool {
    matches!(
        source.get("kind").and_then(Value::as_str),
        Some("state_file" | "cookie_header" | "installed_browser" | "profile_directory")
    )
}

/// An inherited `browser.locale` yields to an imported login or a Profile
/// directory used in place (ADR 0083; ticket #3289).
pub const LOCALE_YIELDS_TO_AN_IMPORT: Yielding = Yielding {
    key: &BROWSER_LOCALE,
    when: set,
    to: &SESSION_SOURCE,
    to_values: fixes_locale_and_timezone,
    unless: None,
    reason: "its Session source fixes the Session's locale and time zone",
};

/// An inherited `browser.timezone_id` yields to an imported login or a
/// Profile directory used in place (ADR 0083; ticket #3289).
pub const TIMEZONE_YIELDS_TO_AN_IMPORT: Yielding = Yielding {
    key: &BROWSER_TIMEZONE_ID,
    when: set,
    to: &SESSION_SOURCE,
    to_values: fixes_locale_and_timezone,
    unless: None,
    reason: "its Session source fixes the Session's locale and time zone",
};

/// The emulation module's catalog entries: its settings.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        BROWSER_VIEWPORT,
        BROWSER_DEVICE_SCALE_FACTOR,
        BROWSER_LOCALE,
        BROWSER_TIMEZONE_ID,
        BROWSER_COLOR_SCHEME,
        BROWSER_JAVA_SCRIPT_ENABLED,
        BROWSER_GEOLOCATION,
        BROWSER_HAS_TOUCH,
        BROWSER_IS_MOBILE,
        BROWSER_PERMISSIONS,
        NETWORK_THROTTLE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
