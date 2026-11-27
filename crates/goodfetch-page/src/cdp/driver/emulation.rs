//! Applies a Session's device emulation and network throttling to a page
//! target over CDP (ADRs 0083 and 0089; spec #578, ticket #3289).
//!
//! At default headless settings (`1280×720`, scale `1`, light, JS enabled,
//! desktop, no touch/locale/timezone/geolocation/throttle), [`apply`] with
//! `force_all = false` sends zero extra CDP commands on page open so
//! `C7.opening_commands` stays unchanged: the headless launch's
//! `--window-size=1280,720` establishes the default window size. When
//! `force_all = true` (after a Lent page's attach ends), every emulation
//! command is sent explicitly to restore the Session's settings over
//! anything the attached library mutated.
#![expect(
    clippy::disallowed_types,
    reason = "per-context emulation is written once when a context opens and read when a popup opens, each lock held for one map operation and never across `.await` (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::{browser, emulation, network};
use goodfetch_engine::CallError;

use super::Browser;
use crate::emulation::{ColorScheme, SessionEmulation};
use crate::errors::Call;

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

/// Applies `emulation` to the page target `session` on `browser` under
/// `call`. When `force_all` is `false` (initial page preparation), settings
/// that already match Chromium's defaults send no command; when `force_all`
/// is `true` (re-applying after a Lent page's attach ends), every setting is
/// sent explicitly so mutations made by the attached library are undone.
pub(super) async fn apply(
    browser: &Browser,
    session: &TargetSession,
    emulation: &SessionEmulation,
    call: &Call,
    force_all: bool,
) -> Result<(), CallError> {
    if emulation.borrowed {
        return Ok(());
    }

    apply_device_metrics(browser, session, emulation, call, force_all).await?;

    if force_all || emulation.has_touch {
        browser
            .send(
                Some(session),
                &emulation::SetTouchEmulationEnabledParams {
                    enabled: emulation.has_touch,
                    max_touch_points: emulation.has_touch.then_some(1),
                },
                call,
            )
            .await?;
    }

    if force_all || emulation.color_scheme != ColorScheme::Light {
        browser
            .send(
                Some(session),
                &emulation::SetEmulatedMediaParams {
                    media: None,
                    features: Some(vec![emulation::MediaFeature {
                        name: "prefers-color-scheme".to_owned(),
                        value: emulation.color_scheme.cdp_value().to_owned(),
                    }]),
                },
                call,
            )
            .await?;
    }

    if force_all || !emulation.java_script_enabled {
        browser
            .send(
                Some(session),
                &emulation::SetScriptExecutionDisabledParams {
                    value: !emulation.java_script_enabled,
                },
                call,
            )
            .await?;
    }

    if force_all || emulation.locale.is_some() {
        browser
            .send(
                Some(session),
                &emulation::SetLocaleOverrideParams {
                    locale: emulation.locale.clone(),
                },
                call,
            )
            .await?;
    }

    if force_all || emulation.timezone_id.is_some() {
        browser
            .send(
                Some(session),
                &emulation::SetTimezoneOverrideParams {
                    timezone_id: emulation.timezone_id.clone().unwrap_or_default(),
                },
                call,
            )
            .await?;
    }

    apply_geolocation(browser, session, emulation, call, force_all).await?;
    apply_throttle(browser, session, emulation, call, force_all).await?;

    Ok(())
}

fn device_metrics(
    width: i64,
    height: i64,
    screen_size: Option<(i64, i64)>,
    device_scale_factor: f64,
    mobile: bool,
) -> emulation::SetDeviceMetricsOverrideParams {
    emulation::SetDeviceMetricsOverrideParams {
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
    }
}

async fn apply_device_metrics(
    browser: &Browser,
    session: &TargetSession,
    emulation: &SessionEmulation,
    call: &Call,
    force_all: bool,
) -> Result<(), CallError> {
    if !force_all && !emulation.needs_device_metrics_on_open() {
        return Ok(());
    }
    let non_default_scale = (emulation.device_scale_factor - 1.0).abs() > f64::EPSILON;
    match emulation.viewport {
        Some(viewport) => {
            let width = i64::from(viewport.width);
            let height = i64::from(viewport.height);
            let params = device_metrics(
                width,
                height,
                Some((width, height)),
                emulation.device_scale_factor,
                emulation.is_mobile,
            );
            browser.send(Some(session), &params, call).await.map(drop)
        }
        None if non_default_scale || emulation.is_mobile => {
            let params = device_metrics(
                0,
                0,
                None,
                emulation.device_scale_factor,
                emulation.is_mobile,
            );
            browser.send(Some(session), &params, call).await.map(drop)
        }
        None => browser
            .send(
                Some(session),
                &emulation::ClearDeviceMetricsOverrideParams {},
                call,
            )
            .await
            .map(drop),
    }
}

async fn apply_geolocation(
    browser: &Browser,
    session: &TargetSession,
    emulation: &SessionEmulation,
    call: &Call,
    force_all: bool,
) -> Result<(), CallError> {
    match emulation.geolocation {
        Some(geo) => browser
            .send(
                Some(session),
                &emulation::SetGeolocationOverrideParams {
                    latitude: Some(geo.latitude),
                    longitude: Some(geo.longitude),
                    accuracy: Some(geo.accuracy.unwrap_or(0.0)),
                    ..emulation::SetGeolocationOverrideParams::default()
                },
                call,
            )
            .await
            .map(drop),
        None if force_all => browser
            .send(
                Some(session),
                &emulation::ClearGeolocationOverrideParams {},
                call,
            )
            .await
            .map(drop),
        None => Ok(()),
    }
}

async fn apply_throttle(
    browser: &Browser,
    session: &TargetSession,
    emulation: &SessionEmulation,
    call: &Call,
    force_all: bool,
) -> Result<(), CallError> {
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
        None => return Ok(()),
    };
    browser.send(Some(session), &params, call).await.map(drop)
}

/// Maps a Playwright permission name from `browser.permissions` to its CDP
/// `Browser.PermissionType`.
fn cdp_permission(name: &str) -> Option<browser::PermissionType> {
    use browser::PermissionType;
    match name {
        "geolocation" => Some(PermissionType::Geolocation),
        "midi" => Some(PermissionType::Midi),
        "midi-sysex" => Some(PermissionType::MidiSysex),
        "notifications" => Some(PermissionType::Notifications),
        "camera" => Some(PermissionType::VideoCapture),
        "microphone" => Some(PermissionType::AudioCapture),
        "background-sync" => Some(PermissionType::BackgroundSync),
        "ambient-light-sensor" | "accelerometer" | "gyroscope" | "magnetometer" => {
            Some(PermissionType::Sensors)
        }
        "clipboard-read" => Some(PermissionType::ClipboardReadWrite),
        "clipboard-write" => Some(PermissionType::ClipboardSanitizedWrite),
        "payment-handler" => Some(PermissionType::PaymentHandler),
        "storage-access" => Some(PermissionType::StorageAccess),
        "local-fonts" => Some(PermissionType::LocalFonts),
        "screen-wake-lock" => Some(PermissionType::WakeLockScreen),
        "idle-detection" => Some(PermissionType::IdleDetection),
        "periodic-background-sync" => Some(PermissionType::PeriodicBackgroundSync),
        "window-management" => Some(PermissionType::WindowManagement),
        "local-network-access" => Some(PermissionType::LocalNetworkAccess),
        _ => None,
    }
}

/// Deduplicated CDP permissions for `emulation.permissions`.
fn cdp_permissions(emulation: &SessionEmulation) -> Vec<browser::PermissionType> {
    let mut mapped = Vec::with_capacity(emulation.permissions.len());
    for name in &emulation.permissions {
        if let Some(permission) = cdp_permission(name)
            && !mapped.contains(&permission)
        {
            mapped.push(permission);
        }
    }
    mapped
}

/// Grants `emulation.permissions` on `context` over CDP (`Browser.grantPermissions`).
/// Sends zero commands when the Session is borrowed or declares no permissions.
pub(super) async fn grant_permissions(
    browser: &Browser,
    context: &browser::BrowserContextID,
    emulation: &SessionEmulation,
    call: &Call,
) -> Result<(), CallError> {
    if emulation.borrowed || emulation.permissions.is_empty() {
        return Ok(());
    }
    let permissions = cdp_permissions(emulation);
    if permissions.is_empty() {
        return Ok(());
    }
    browser
        .send(
            None,
            &browser::GrantPermissionsParams {
                permissions,
                origin: None,
                browser_context_id: Some(context.clone()),
            },
            call,
        )
        .await
        .map(drop)
}

/// Resets any permission overrides an attached library may have changed on
/// `context`, then re-grants `emulation.permissions` (ADR 0089; ticket #3290).
pub(super) async fn reset_and_grant_permissions(
    browser: &Browser,
    context: &browser::BrowserContextID,
    emulation: &SessionEmulation,
    call: &Call,
) -> Result<(), CallError> {
    if emulation.borrowed {
        return Ok(());
    }
    browser
        .send(
            None,
            &browser::ResetPermissionsParams {
                browser_context_id: Some(context.clone()),
            },
            call,
        )
        .await?;
    grant_permissions(browser, context, emulation, call).await
}

#[cfg(test)]
mod tests;
