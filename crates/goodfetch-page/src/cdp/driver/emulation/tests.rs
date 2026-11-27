//! In-process CDP command verification for Session device emulation and
//! re-apply after a Lent page's attach ends (ADR 0089; ticket #3289).

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_engine::CallError;
use goodfetch_protocol::page::emulation::{Geolocation, NetworkThrottle, Viewport};
use goodfetch_test_support::runtime;
use serde_json::{Value, json};

use super::{grant_permissions_command, page_commands};
use crate::cdp::driver::replay::{self, SESSION};
use crate::cdp::driver::{Browser, Connect, PageDriver, SessionDriver, SessionEgress};
use crate::emulation::{ColorScheme, SessionEmulation, SessionMode};

struct Played(Browser);

impl Connect for Played {
    fn connect(&self) -> impl Future<Output = Result<Browser, CallError>> + Send {
        std::future::ready(Ok(self.0.clone()))
    }
}

/// Covers: story:5-59
#[test]
fn default_headless_emulation_sends_zero_commands_on_open() {
    runtime().block_on(async {
        let ((connection, driver), played) = replay::connected();
        let browser = Browser::played(connection);
        let session = TargetSession(SESSION.to_owned());
        let call = browser.opening();
        let emulation = SessionEmulation::default();
        assert!(page_commands(&emulation, false).is_empty());

        // `played.hold()` never answers any command, so if `apply_emulation`
        // sent even one command this test would hang until the deadline
        // instead of succeeding immediately.
        let (_, result, ()) = tokio::join!(
            driver.run(),
            async {
                let res = browser
                    .apply_emulation(&session, &emulation, &call, false)
                    .await;
                drop(browser);
                res
            },
            played.hold(),
        );

        result.unwrap();
    });
}

/// Covers: story:5-59
#[test]
fn reapply_after_attach_restores_every_emulation_command() {
    runtime().block_on(async {
        let ((connection, driver), mut played) = replay::connected();
        let page = PageDriver::played(connection);
        let browser = page.browser.clone();
        let emulation = SessionEmulation {
            viewport: Some(Viewport {
                width: 375,
                height: 812,
            }),
            device_scale_factor: 2.0,
            locale: Some("fr-FR".to_owned()),
            timezone_id: Some("Europe/Paris".to_owned()),
            color_scheme: ColorScheme::Dark,
            java_script_enabled: false,
            geolocation: Some(Geolocation {
                latitude: 48.8566,
                longitude: 2.3522,
                accuracy: Some(5.0),
            }),
            has_touch: true,
            is_mobile: true,
            permissions: vec![
                "geolocation".to_owned(),
                "clipboard-read".to_owned(),
                "accelerometer".to_owned(),
                "gyroscope".to_owned(),
            ],
            throttle: Some(NetworkThrottle {
                offline: false,
                latency: 200,
                download_bytes_per_second: Some(50_000),
                upload_bytes_per_second: None,
            }),
            mode: SessionMode::Headless,
        };
        let session_driver =
            SessionDriver::new(Played(browser.clone()), SessionEgress::default(), emulation);

        let (_, result, sent) = tokio::join!(
            driver.run(),
            async {
                let res = session_driver.reapply_after_attach(&page).await;
                drop((session_driver, page, browser));
                res
            },
            async {
                let mut commands: Vec<(String, Value)> = Vec::new();
                for _ in 0..10 {
                    let command = played.command().await;
                    let method = command["method"].as_str().unwrap_or_default().to_owned();
                    let params = command["params"].clone();
                    played.send(&[replay::answer(&command, json!({}))]).await;
                    commands.push((method, params));
                }
                drop(played);
                commands
            }
        );

        result.unwrap();
        let methods: Vec<&str> = sent.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            methods,
            vec![
                "Browser.resetPermissions",
                "Browser.grantPermissions",
                "Emulation.setDeviceMetricsOverride",
                "Emulation.setTouchEmulationEnabled",
                "Emulation.setEmulatedMedia",
                "Emulation.setScriptExecutionDisabled",
                "Emulation.setLocaleOverride",
                "Emulation.setTimezoneOverride",
                "Emulation.setGeolocationOverride",
                "Network.emulateNetworkConditions",
            ]
        );
        assert_eq!(
            sent[1].1["permissions"],
            json!(["geolocation", "clipboardReadWrite", "sensors"])
        );
        assert_eq!(sent[2].1["width"], json!(375));
        assert_eq!(sent[2].1["height"], json!(812));
        assert_eq!(sent[2].1["mobile"], json!(true));
        assert_eq!(sent[3].1["enabled"], json!(true));
        assert_eq!(sent[4].1["features"][0]["value"], json!("dark"));
        assert_eq!(sent[5].1["value"], json!(true));
        assert_eq!(sent[6].1["locale"], json!("fr-FR"));
        assert_eq!(sent[7].1["timezoneId"], json!("Europe/Paris"));
        assert_eq!(sent[8].1["latitude"], json!(48.8566));
        assert_eq!(sent[9].1["latency"], json!(200.0));
        assert_eq!(sent[9].1["downloadThroughput"], json!(50000.0));
        assert_eq!(sent[9].1["uploadThroughput"], json!(-1.0));
    });
}

/// Covers: story:5-39
#[test]
fn empty_permissions_send_zero_commands_on_context_open() {
    runtime().block_on(async {
        let ((connection, driver), played) = replay::connected();
        let browser = Browser::played(connection);
        let call = browser.opening();
        let emulation = SessionEmulation::default();
        let context = "ctx-1".to_owned();
        assert!(grant_permissions_command(&context, &emulation).is_none());

        let (_, result, ()) = tokio::join!(
            driver.run(),
            async {
                let res = browser.grant_permissions(&context, &emulation, &call).await;
                drop(browser);
                res
            },
            played.hold(),
        );

        result.unwrap();
    });
}
