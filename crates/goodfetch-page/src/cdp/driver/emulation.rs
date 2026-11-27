//! Pure CDP command planning for a Session's device emulation, network
//! throttling, and permissions (ADRs 0083 and 0089; spec #578, tickets
//! #3289 and #3290).
//!
//! At default headless settings (`1280×720`, scale `1`, light, JS enabled,
//! desktop, no touch/locale/timezone/geolocation/throttle),
//! [`page_commands`] with `force_all = false` returns an empty plan so
//! `C7.opening_commands` stays unchanged: the headless launch's
//! `--window-size=1280,720` establishes the default window size. When
//! `force_all = true` (after a Lent page's attach ends), every emulation
//! command is emitted explicitly to restore the Session's settings over
//! anything the attached library mutated.
#![expect(
    clippy::disallowed_types,
    reason = "per-context emulation is written once when a context opens and read when a popup opens, each lock held for one map operation and never across `.await` (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::generated::{browser, emulation, network};
use goodfetch_protocol::page::emulation::BrowserPermission;

use crate::emulation::{ColorScheme, SessionEmulation};

/// Per-browser-context [`SessionEmulation`] lookup so popups opened in a
/// context inherit its Session's emulation before they run.
#[derive(Default)]
pub(super) struct ContextEmulations {
    by_context: Mutex<BTreeMap<browser::BrowserContextID, SessionEmulation>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ContextEmulations {
    pub(super) fn remember(
        &self,
        context: &browser::BrowserContextID,
        emulation: SessionEmulation,
    ) {
        lock(&self.by_context).insert(context.clone(), emulation);
    }

    pub(super) fn of(&self, context: &browser::BrowserContextID) -> SessionEmulation {
        lock(&self.by_context)
            .get(context)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn forget(&self, context: &browser::BrowserContextID) {
        lock(&self.by_context).remove(context);
    }
}

/// One CDP command emitted for a page target's Session emulation plan.
pub(super) enum PageCommand {
    SetDeviceMetricsOverride(Box<emulation::SetDeviceMetricsOverrideParams>),
    ClearDeviceMetricsOverride(emulation::ClearDeviceMetricsOverrideParams),
    SetTouchEmulationEnabled(emulation::SetTouchEmulationEnabledParams),
    SetEmulatedMedia(emulation::SetEmulatedMediaParams),
    SetScriptExecutionDisabled(emulation::SetScriptExecutionDisabledParams),
    SetLocaleOverride(emulation::SetLocaleOverrideParams),
    SetTimezoneOverride(emulation::SetTimezoneOverrideParams),
    SetGeolocationOverride(emulation::SetGeolocationOverrideParams),
    ClearGeolocationOverride(emulation::ClearGeolocationOverrideParams),
    EmulateNetworkConditions(network::EmulateNetworkConditionsParams),
}

/// Builds the ordered CDP commands to apply `emulation` to a page target.
/// When `force_all` is `false` (initial page preparation), settings that
/// already match Chromium's defaults emit no command; when `force_all` is
/// `true` (re-applying after a Lent page's attach ends), every setting is
/// emitted explicitly so mutations made by the attached library are undone.
pub(super) fn page_commands(emulation: &SessionEmulation, force_all: bool) -> Vec<PageCommand> {
    if emulation.is_borrowed() {
        return Vec::new();
    }

    let mut commands = Vec::new();
    if let Some(command) = device_metrics_command(emulation, force_all) {
        commands.push(command);
    }
    if force_all || emulation.has_touch {
        commands.push(PageCommand::SetTouchEmulationEnabled(
            emulation::SetTouchEmulationEnabledParams {
                enabled: emulation.has_touch,
                max_touch_points: emulation.has_touch.then_some(1),
            },
        ));
    }
    if force_all || emulation.color_scheme != ColorScheme::Light {
        commands.push(PageCommand::SetEmulatedMedia(
            emulation::SetEmulatedMediaParams {
                media: force_all.then(String::new),
                features: Some(vec![emulation::MediaFeature {
                    name: "prefers-color-scheme".to_owned(),
                    value: emulation.color_scheme.cdp_value().to_owned(),
                }]),
            },
        ));
    }
    if force_all || !emulation.java_script_enabled {
        commands.push(PageCommand::SetScriptExecutionDisabled(
            emulation::SetScriptExecutionDisabledParams {
                value: !emulation.java_script_enabled,
            },
        ));
    }
    if force_all || emulation.locale.is_some() {
        commands.push(PageCommand::SetLocaleOverride(
            emulation::SetLocaleOverrideParams {
                locale: emulation.locale.clone(),
            },
        ));
    }
    if force_all || emulation.timezone_id.is_some() {
        commands.push(PageCommand::SetTimezoneOverride(
            emulation::SetTimezoneOverrideParams {
                timezone_id: emulation.timezone_id.clone().unwrap_or_default(),
            },
        ));
    }
    if let Some(command) = geolocation_command(emulation, force_all) {
        commands.push(command);
    }
    if let Some(command) = throttle_command(emulation, force_all) {
        commands.push(command);
    }
    commands
}

fn device_metrics(
    width: i64,
    height: i64,
    screen_size: Option<(i64, i64)>,
    device_scale_factor: f64,
    mobile: bool,
) -> Box<emulation::SetDeviceMetricsOverrideParams> {
    Box::new(emulation::SetDeviceMetricsOverrideParams {
        width,
        height,
        device_scale_factor,
        mobile,
        scale: None,
        screen_width: screen_size.map(|(w, _)| w),
        screen_height: screen_size.map(|(_, h)| h),
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
        scrollbar_type: None,
        screen_orientation_lock_emulation: None,
        viewport_meta: None,
        text_layout_mode: None,
    })
}

fn device_metrics_command(emulation: &SessionEmulation, force_all: bool) -> Option<PageCommand> {
    if !force_all && !emulation.needs_device_metrics_on_open() {
        return None;
    }
    let non_default_scale = !emulation.device_scale_factor.is_default();
    let scale = emulation.device_scale_factor.get();
    match emulation.viewport {
        Some(viewport) => {
            let width = i64::from(viewport.width);
            let height = i64::from(viewport.height);
            Some(PageCommand::SetDeviceMetricsOverride(device_metrics(
                width,
                height,
                Some((width, height)),
                scale,
                emulation.is_mobile,
            )))
        }
        None if non_default_scale || emulation.is_mobile => {
            Some(PageCommand::SetDeviceMetricsOverride(device_metrics(
                0,
                0,
                None,
                scale,
                emulation.is_mobile,
            )))
        }
        None => Some(PageCommand::ClearDeviceMetricsOverride(
            emulation::ClearDeviceMetricsOverrideParams {},
        )),
    }
}

fn geolocation_command(emulation: &SessionEmulation, force_all: bool) -> Option<PageCommand> {
    match emulation.geolocation {
        Some(geo) => Some(PageCommand::SetGeolocationOverride(
            emulation::SetGeolocationOverrideParams {
                latitude: Some(geo.latitude),
                longitude: Some(geo.longitude),
                accuracy: Some(geo.accuracy.unwrap_or(0.0)),
                ..emulation::SetGeolocationOverrideParams::default()
            },
        )),
        None if force_all => Some(PageCommand::ClearGeolocationOverride(
            emulation::ClearGeolocationOverrideParams {},
        )),
        None => None,
    }
}

fn throttle_command(emulation: &SessionEmulation, force_all: bool) -> Option<PageCommand> {
    let params = match emulation.throttle {
        Some(throttle) => network::EmulateNetworkConditionsParams {
            offline: throttle.offline,
            #[expect(
                clippy::cast_precision_loss,
                reason = "CDP latency is an f64 in milliseconds; realistic throttle latencies are far below 2^53 ms"
            )]
            latency: throttle.latency as f64,
            #[expect(
                clippy::cast_precision_loss,
                reason = "CDP downloadThroughput is an f64 in bytes/s; realistic throughputs are far below 2^53 B/s"
            )]
            download_throughput: throttle
                .download_bytes_per_second
                .map_or(-1.0, |bytes| bytes as f64),
            #[expect(
                clippy::cast_precision_loss,
                reason = "CDP uploadThroughput is an f64 in bytes/s; realistic throughputs are far below 2^53 B/s"
            )]
            upload_throughput: throttle
                .upload_bytes_per_second
                .map_or(-1.0, |bytes| bytes as f64),
            connection_type: None,
            packet_loss: None,
            packet_queue_length: None,
            packet_reordering: None,
        },
        None if force_all => network::EmulateNetworkConditionsParams {
            offline: false,
            latency: 0.0,
            download_throughput: -1.0,
            upload_throughput: -1.0,
            connection_type: None,
            packet_loss: None,
            packet_queue_length: None,
            packet_reordering: None,
        },
        None => return None,
    };
    Some(PageCommand::EmulateNetworkConditions(params))
}

/// Maps a Playwright permission from `browser.permissions` to its CDP
/// `Browser.PermissionType`.
const fn cdp_permission(permission: BrowserPermission) -> browser::PermissionType {
    use browser::PermissionType;
    match permission {
        BrowserPermission::Geolocation => PermissionType::Geolocation,
        BrowserPermission::Midi => PermissionType::Midi,
        BrowserPermission::MidiSysex => PermissionType::MidiSysex,
        BrowserPermission::Notifications => PermissionType::Notifications,
        BrowserPermission::Camera => PermissionType::VideoCapture,
        BrowserPermission::Microphone => PermissionType::AudioCapture,
        BrowserPermission::BackgroundSync => PermissionType::BackgroundSync,
        BrowserPermission::AmbientLightSensor
        | BrowserPermission::Accelerometer
        | BrowserPermission::Gyroscope
        | BrowserPermission::Magnetometer => PermissionType::Sensors,
        BrowserPermission::ClipboardRead => PermissionType::ClipboardReadWrite,
        BrowserPermission::ClipboardWrite => PermissionType::ClipboardSanitizedWrite,
        BrowserPermission::PaymentHandler => PermissionType::PaymentHandler,
        BrowserPermission::StorageAccess => PermissionType::StorageAccess,
        BrowserPermission::LocalFonts => PermissionType::LocalFonts,
        BrowserPermission::ScreenWakeLock => PermissionType::WakeLockScreen,
        BrowserPermission::IdleDetection => PermissionType::IdleDetection,
        BrowserPermission::PeriodicBackgroundSync => PermissionType::PeriodicBackgroundSync,
        BrowserPermission::WindowManagement => PermissionType::WindowManagement,
        BrowserPermission::LocalNetworkAccess => PermissionType::LocalNetworkAccess,
    }
}

/// Deduplicated CDP permissions for `emulation.permissions`.
fn cdp_permissions(emulation: &SessionEmulation) -> Vec<browser::PermissionType> {
    let mut mapped = Vec::with_capacity(emulation.permissions.len());
    for &permission in &emulation.permissions {
        let cdp = cdp_permission(permission);
        if !mapped.contains(&cdp) {
            mapped.push(cdp);
        }
    }
    mapped
}

/// Builds `Browser.grantPermissions` for `context` when `emulation` is
/// managed and grants at least one mapped permission; `None` otherwise.
pub(super) fn grant_permissions_command(
    context: &browser::BrowserContextID,
    emulation: &SessionEmulation,
) -> Option<browser::GrantPermissionsParams> {
    if emulation.is_borrowed() || emulation.permissions.is_empty() {
        return None;
    }
    let permissions = cdp_permissions(emulation);
    if permissions.is_empty() {
        return None;
    }
    Some(browser::GrantPermissionsParams {
        permissions,
        origin: None,
        browser_context_id: Some(context.clone()),
    })
}

#[cfg(test)]
mod tests;
