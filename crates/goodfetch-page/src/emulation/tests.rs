#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::settings::{Layers, Resolved, Values, resolve as resolve_settings};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::page::emulation::{
    BrowserPermission, DeviceScaleFactor, Geolocation, NetworkThrottle, Viewport,
};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::settings::session_source::SessionSource;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{ColorScheme, Refusal, SessionEmulation, SessionMode, resolve};
use crate::access_mode::{BrowserFamily, SessionSettings};

fn map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn read(layer: SettingsLayer, value: Value) -> Values {
    Values::read(layer, &map(value), Product::Collie).expect("valid settings")
}

fn resolve_run(run: Value, user: Value) -> Resolved {
    let layers = Layers {
        user_configuration: read(SettingsLayer::UserConfiguration, user),
        run: read(SettingsLayer::Run, run),
        ..Layers::default()
    };
    resolve_settings(&layers, Product::Collie, Product::Collie).expect("valid resolved settings")
}

fn source(value: Value) -> SessionSource {
    serde_json::from_value(value).expect("valid session source")
}

/// Covers: story:5-59, setting:browser.viewport, setting:browser.has_touch
#[test]
fn defaults_match_spec_578_and_require_no_device_metrics_override_on_open() {
    let own = Values::default();
    let run = resolve_run(json!({}), json!({}));
    let settings = SessionSettings::new(&own, Some(&run));
    let plan = resolve(&settings, None, None, BrowserFamily::Chromium).unwrap();

    assert_eq!(plan, SessionEmulation::default());
    assert_eq!(plan.viewport, Some(Viewport::DEFAULT));
    assert!(!plan.needs_device_metrics_on_open());
}

/// Covers: story:5-59, setting:browser.viewport, setting:browser.device_scale_factor, setting:browser.color_scheme, setting:browser.java_script_enabled, setting:browser.geolocation, setting:browser.has_touch, setting:browser.is_mobile, setting:network.throttle
#[test]
fn session_settings_override_defaults_and_trigger_device_metrics() {
    let own = read(
        SettingsLayer::Run,
        json!({
            "browser": {
                "viewport": { "width": 375, "height": 812 },
                "device_scale_factor": 2.0,
                "locale": "fr-FR",
                "timezone_id": "Europe/Paris",
                "color_scheme": "dark",
                "java_script_enabled": false,
                "geolocation": { "latitude": 48.8566, "longitude": 2.3522, "accuracy": 5.0 },
                "has_touch": true,
                "is_mobile": true,
            },
            "network": {
                "throttle": { "latency": 250, "download_bytes_per_second": 50_000 },
            },
        }),
    );
    let run = resolve_run(json!({}), json!({}));
    let settings = SessionSettings::new(&own, Some(&run));
    let plan = resolve(&settings, None, None, BrowserFamily::Chromium).unwrap();

    assert_eq!(
        plan.viewport,
        Some(Viewport {
            width: 375,
            height: 812
        })
    );
    assert_eq!(
        plan.device_scale_factor,
        DeviceScaleFactor::new(2.0).unwrap()
    );
    assert_eq!(plan.locale.as_deref(), Some("fr-FR"));
    assert_eq!(plan.timezone_id.as_deref(), Some("Europe/Paris"));
    assert_eq!(plan.color_scheme, ColorScheme::Dark);
    assert!(!plan.java_script_enabled);
    assert_eq!(
        plan.geolocation,
        Some(Geolocation {
            latitude: 48.8566,
            longitude: 2.3522,
            accuracy: Some(5.0),
        })
    );
    assert!(plan.has_touch);
    assert!(plan.is_mobile);
    assert_eq!(
        plan.throttle,
        Some(NetworkThrottle {
            offline: false,
            latency: 250,
            download_bytes_per_second: Some(50_000),
            upload_bytes_per_second: None,
        })
    );
    assert!(plan.needs_device_metrics_on_open());
}

/// Covers: story:5-59, setting:browser.viewport
#[test]
fn null_viewport_is_refused_headless_and_allowed_headful() {
    let own = read(
        SettingsLayer::Run,
        json!({ "browser": { "viewport": null } }),
    );

    let headless_run = resolve_run(json!({}), json!({}));
    let headless_settings = SessionSettings::new(&own, Some(&headless_run));
    assert_eq!(
        resolve(&headless_settings, None, None, BrowserFamily::Chromium),
        Err(Refusal::NullViewportHeadless {
            layer: SettingsLayer::Run,
        })
    );

    let headful_run = resolve_run(json!({ "browser": { "headless": false } }), json!({}));
    let headful_settings = SessionSettings::new(&own, Some(&headful_run));
    let plan = resolve(&headful_settings, None, None, BrowserFamily::Chromium).unwrap();
    assert_eq!(plan.viewport, None);
    assert_eq!(plan.mode, SessionMode::Headful);
}

/// Covers: story:5-59, setting:browser.locale, setting:browser.timezone_id
#[test]
fn imported_login_locks_explicit_locale_and_timezone_while_inherited_yields() {
    let import = source(json!({ "kind": "state_file", "path": "login.json" }));
    let run = resolve_run(
        json!({ "browser": { "locale": "de-DE", "timezone_id": "Europe/Berlin" } }),
        json!({}),
    );

    // Inherited locale and timezone yield to the imported login's identity.
    let empty = Values::default();
    let inherited = SessionSettings::new(&empty, Some(&run));
    let plan = resolve(
        &inherited,
        Some(&import),
        Some(SettingsLayer::Run),
        BrowserFamily::Chromium,
    )
    .unwrap();
    assert_eq!(plan.locale, None);
    assert_eq!(plan.timezone_id, None);

    // Explicit locale where the Session opens fails with setting_locked naming session.source.
    let explicit_locale = read(
        SettingsLayer::Run,
        json!({ "browser": { "locale": "fr-FR" } }),
    );
    let explicit = SessionSettings::new(&explicit_locale, Some(&run));
    assert!(matches!(
        resolve(
            &explicit,
            Some(&import),
            Some(SettingsLayer::UserConfiguration),
            BrowserFamily::Chromium,
        ),
        Err(Refusal::LockedBySource {
            key: "browser.locale",
            source_kind: "state_file",
            source_layer: SettingsLayer::UserConfiguration,
            ..
        })
    ));

    // Explicit null clears an inherited locale without conflicting.
    let cleared = read(SettingsLayer::Run, json!({ "browser": { "locale": null } }));
    let cleared_settings = SessionSettings::new(&cleared, Some(&run));
    let cleared_plan = resolve(
        &cleared_settings,
        Some(&import),
        Some(SettingsLayer::Run),
        BrowserFamily::Chromium,
    )
    .unwrap();
    assert_eq!(cleared_plan.locale, None);
}

/// Covers: story:5-59, setting:browser.viewport
#[test]
fn borrowed_browser_refuses_explicit_browser_settings_and_yields_inherited() {
    let cdp = source(json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }));
    let run = resolve_run(
        json!({ "browser": { "viewport": { "width": 800, "height": 600 }, "locale": "fr-FR" } }),
        json!({}),
    );

    let empty = Values::default();
    let inherited = SessionSettings::new(&empty, Some(&run));
    let plan = resolve(
        &inherited,
        Some(&cdp),
        Some(SettingsLayer::Run),
        BrowserFamily::Chromium,
    )
    .unwrap();
    assert!(plan.is_borrowed());
    assert_eq!(plan.viewport, None);
    assert!(!plan.needs_device_metrics_on_open());

    let explicit = read(
        SettingsLayer::Run,
        json!({ "browser": { "viewport": { "width": 375, "height": 812 } } }),
    );
    let explicit_settings = SessionSettings::new(&explicit, Some(&run));
    assert_eq!(
        resolve(
            &explicit_settings,
            Some(&cdp),
            Some(SettingsLayer::Run),
            BrowserFamily::Chromium,
        ),
        Err(Refusal::OnBorrowed {
            key: "browser.viewport",
            source: "cdp_url",
        })
    );

    let explicit_throttle = read(
        SettingsLayer::Run,
        json!({ "network": { "throttle": { "latency": 150 } } }),
    );
    let throttle_settings = SessionSettings::new(&explicit_throttle, Some(&run));
    assert_eq!(
        resolve(
            &throttle_settings,
            Some(&cdp),
            Some(SettingsLayer::Run),
            BrowserFamily::Chromium,
        ),
        Err(Refusal::OnBorrowed {
            key: "network.throttle",
            source: "cdp_url",
        })
    );
}

/// Covers: story:5-59, setting:browser.is_mobile, setting:network.throttle
#[test]
fn is_mobile_and_network_throttle_outside_chromium_raise_capability_unavailable() {
    let run = resolve_run(json!({}), json!({}));

    let mobile = read(
        SettingsLayer::Run,
        json!({ "browser": { "is_mobile": true } }),
    );
    let mobile_settings = SessionSettings::new(&mobile, Some(&run));
    assert_eq!(
        resolve(&mobile_settings, None, None, BrowserFamily::Firefox),
        Err(Refusal::ChromiumOnly {
            key: "browser.is_mobile",
            layer: SettingsLayer::Run,
            family: BrowserFamily::Firefox,
        })
    );

    let throttle = read(
        SettingsLayer::Run,
        json!({ "network": { "throttle": { "latency": 200 } } }),
    );
    let throttle_settings = SessionSettings::new(&throttle, Some(&run));
    assert_eq!(
        resolve(&throttle_settings, None, None, BrowserFamily::WebKit),
        Err(Refusal::ChromiumOnly {
            key: "network.throttle",
            layer: SettingsLayer::Run,
            family: BrowserFamily::WebKit,
        })
    );
}

/// Covers: story:5-39, story:5-87, setting:browser.permissions
#[test]
fn headless_deduplicates_permissions_and_borrowed_refuses_permissions() {
    let clipboard = read(
        SettingsLayer::Run,
        json!({ "browser": { "permissions": ["geolocation", "clipboard-read", "clipboard-write", "geolocation"] } }),
    );

    // Headless allows clipboard permissions and deduplicates entries.
    let headless_run = resolve_run(json!({}), json!({}));
    let headless_settings = SessionSettings::new(&clipboard, Some(&headless_run));
    let plan = resolve(&headless_settings, None, None, BrowserFamily::Chromium).unwrap();
    assert_eq!(
        plan.permissions,
        vec![
            BrowserPermission::Geolocation,
            BrowserPermission::ClipboardRead,
            BrowserPermission::ClipboardWrite,
        ]
    );

    // Non-clipboard permissions work headful.
    let headful_run = resolve_run(json!({ "browser": { "headless": false } }), json!({}));
    let geo = read(
        SettingsLayer::Run,
        json!({ "browser": { "permissions": ["geolocation"] } }),
    );
    let geo_settings = SessionSettings::new(&geo, Some(&headful_run));
    assert_eq!(
        resolve(&geo_settings, None, None, BrowserFamily::Chromium)
            .unwrap()
            .permissions,
        vec![BrowserPermission::Geolocation]
    );

    // Borrowed browser refuses explicit `browser.permissions`.
    let cdp = source(json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }));
    let borrowed_settings = SessionSettings::new(&geo, Some(&headless_run));
    assert_eq!(
        resolve(
            &borrowed_settings,
            Some(&cdp),
            Some(SettingsLayer::Run),
            BrowserFamily::Chromium
        ),
        Err(Refusal::OnBorrowed {
            key: "browser.permissions",
            source: "cdp_url",
        })
    );
}

/// Covers: story:5-39, setting:browser.permissions
#[rstest]
#[case::clipboard_read(BrowserPermission::ClipboardRead)]
#[case::clipboard_write(BrowserPermission::ClipboardWrite)]
fn headful_session_refuses_each_clipboard_permission(#[case] permission: BrowserPermission) {
    let headful_run = resolve_run(json!({ "browser": { "headless": false } }), json!({}));
    let own = read(
        SettingsLayer::Run,
        json!({ "browser": { "permissions": [permission.as_str()] } }),
    );
    let settings = SessionSettings::new(&own, Some(&headful_run));
    assert_eq!(
        resolve(&settings, None, None, BrowserFamily::Chromium),
        Err(Refusal::ClipboardHeadful {
            permission,
            layer: SettingsLayer::Run,
        })
    );
}
