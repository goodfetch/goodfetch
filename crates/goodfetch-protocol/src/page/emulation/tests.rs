use serde_json::json;

use super::{
    BROWSER_COLOR_SCHEME, BROWSER_DEVICE_SCALE_FACTOR, BROWSER_GEOLOCATION, BROWSER_HAS_TOUCH,
    BROWSER_IS_MOBILE, BROWSER_JAVA_SCRIPT_ENABLED, BROWSER_LOCALE, BROWSER_PERMISSION_NAMES,
    BROWSER_PERMISSIONS, BROWSER_TIMEZONE_ID, BROWSER_VIEWPORT, DEVICE_SCALE_FACTOR_SHAPE,
    DeviceScaleFactor, GEOLOCATION_SHAPE, LOCALE_YIELDS_TO_AN_IMPORT, NETWORK_THROTTLE,
    NETWORK_THROTTLE_SHAPE, TIMEZONE_YIELDS_TO_AN_IMPORT, VIEWPORT_SHAPE, Viewport,
};

#[test]
fn viewport_shape_accepts_positive_dimensions_and_refuses_zero_or_unknown_fields() {
    assert!((VIEWPORT_SHAPE.check)(&json!({ "width": 375, "height": 812 })).is_ok());
    assert!((VIEWPORT_SHAPE.check)(&json!({ "width": 0, "height": 812 })).is_err());
    assert!((VIEWPORT_SHAPE.check)(&json!({ "width": 375, "height": 0 })).is_err());
    assert!((VIEWPORT_SHAPE.check)(&json!({ "width": 375, "height": 812, "extra": 1 })).is_err());
    assert_eq!(
        (BROWSER_VIEWPORT.default)(),
        json!({ "width": Viewport::DEFAULT.width, "height": Viewport::DEFAULT.height })
    );
    const { assert!(BROWSER_VIEWPORT.clearable) };
}

#[test]
fn device_scale_factor_accepts_positive_finite_numbers_only() {
    assert!((DEVICE_SCALE_FACTOR_SHAPE.check)(&json!(1)).is_ok());
    assert!((DEVICE_SCALE_FACTOR_SHAPE.check)(&json!(2.5)).is_ok());
    assert!((DEVICE_SCALE_FACTOR_SHAPE.check)(&json!(0)).is_err());
    assert!((DEVICE_SCALE_FACTOR_SHAPE.check)(&json!(-1.5)).is_err());
    assert!((DEVICE_SCALE_FACTOR_SHAPE.check)(&json!("2")).is_err());
    assert!(DeviceScaleFactor::new(f64::NAN).is_err());
    assert!(DeviceScaleFactor::new(f64::INFINITY).is_err());
    assert_eq!((BROWSER_DEVICE_SCALE_FACTOR.default)(), json!(1));
}

#[test]
fn geolocation_shape_checks_coordinate_ranges_and_accuracy() {
    assert!(
        (GEOLOCATION_SHAPE.check)(&json!({ "latitude": 48.8566, "longitude": 2.3522 })).is_ok()
    );
    assert!(
        (GEOLOCATION_SHAPE.check)(&json!({ "latitude": -90.0, "longitude": 180.0, "accuracy": 0 }))
            .is_ok()
    );
    assert!((GEOLOCATION_SHAPE.check)(&json!({ "latitude": 91.0, "longitude": 0.0 })).is_err());
    assert!((GEOLOCATION_SHAPE.check)(&json!({ "latitude": 0.0, "longitude": -181.0 })).is_err());
    assert!(
        (GEOLOCATION_SHAPE.check)(&json!({ "latitude": 0.0, "longitude": 0.0, "accuracy": -1 }))
            .is_err()
    );
    assert_eq!((BROWSER_GEOLOCATION.default)(), json!(null));
}

#[test]
fn network_throttle_shape_accepts_partial_conditions_and_refuses_negatives() {
    assert!((NETWORK_THROTTLE_SHAPE.check)(&json!({ "latency": 300 })).is_ok());
    assert!((NETWORK_THROTTLE_SHAPE.check)(&json!({ "offline": true })).is_ok());
    assert!(
        (NETWORK_THROTTLE_SHAPE.check)(&json!({
            "offline": false,
            "latency": 100,
            "download_bytes_per_second": 50_000,
            "upload_bytes_per_second": 20_000,
        }))
        .is_ok()
    );
    assert!((NETWORK_THROTTLE_SHAPE.check)(&json!({ "latency": -10 })).is_err());
    assert!((NETWORK_THROTTLE_SHAPE.check)(&json!({ "download_bytes_per_second": -1 })).is_err());
    assert_eq!((NETWORK_THROTTLE.default)(), json!(null));
}

#[test]
fn defaults_and_yielding_match_spec_578() {
    assert_eq!((BROWSER_COLOR_SCHEME.default)(), json!("light"));
    assert_eq!((BROWSER_JAVA_SCRIPT_ENABLED.default)(), json!(true));
    assert_eq!((BROWSER_HAS_TOUCH.default)(), json!(false));
    assert_eq!((BROWSER_IS_MOBILE.default)(), json!(false));
    assert_eq!((BROWSER_LOCALE.default)(), json!(null));
    assert_eq!((BROWSER_TIMEZONE_ID.default)(), json!(null));

    let state_file = json!({ "kind": "state_file", "path": "login.json" });
    let cdp_url = json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" });
    assert!((LOCALE_YIELDS_TO_AN_IMPORT.to_values)(&state_file));
    assert!(!(LOCALE_YIELDS_TO_AN_IMPORT.to_values)(&cdp_url));
    assert!((TIMEZONE_YIELDS_TO_AN_IMPORT.to_values)(&state_file));
    assert!(!(TIMEZONE_YIELDS_TO_AN_IMPORT.to_values)(&cdp_url));
}

#[test]
fn browser_permissions_lists_playwright_names_and_defaults_empty() {
    assert_eq!((BROWSER_PERMISSIONS.default)(), json!([]));
    const { assert!(!BROWSER_PERMISSIONS.clearable) };
    assert!(BROWSER_PERMISSION_NAMES.contains(&"geolocation"));
    assert!(BROWSER_PERMISSION_NAMES.contains(&"clipboard-read"));
    assert!(BROWSER_PERMISSION_NAMES.contains(&"clipboard-write"));
    assert!(
        BROWSER_PERMISSIONS
            .kind
            .check(&json!(["geolocation", "clipboard-read", "clipboard-write"]))
            .is_ok()
    );
    assert!(
        BROWSER_PERMISSIONS
            .kind
            .check(&json!(["unknown-permission"]))
            .is_err()
    );
}
