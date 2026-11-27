//! The group-member settings scope for machine-specific settings (ADRs 0028,
//! 0055 and 0056; #1617): values under `[group_member.<name>]` in an Operator
//! policy or user configuration apply only on the named Group member, and
//! only machine-specific settings (local addresses, TLS files and browser
//! paths) may be set at that scope.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the settings files the engine reads themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::browser_cache::BROWSER_EXECUTABLE_PATH;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{TLS_CERT_FILE, TLS_KEY_FILE, TLS_UPSTREAM};
use goodfetch_protocol::egress::EGRESS_LOCAL_ADDRESSES;
use goodfetch_protocol::settings::{SESSION_SOURCE, SettingsLayer};
use goodfetch_protocol::tls::{TLS_CLIENT_CERTIFICATES, TLS_TRUSTED_CAS};
use goodfetch_test_support::TestEnv;
use serde_json::json;

use crate::home::Home;
use crate::settings::{SettingsError, SettingsFiles};

fn version() -> ProductVersion {
    ProductVersion::parse("0.4.0").unwrap()
}

fn setup_files(policy: Option<&str>, user: Option<&str>) -> (TestEnv, SettingsFiles) {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let files = SettingsFiles::of(&home, &version());
    std::fs::create_dir_all(files.operator_policy().parent().unwrap()).unwrap();
    if let Some(text) = policy {
        std::fs::write(files.operator_policy(), text).unwrap();
    }
    if let Some(text) = user {
        std::fs::write(files.user_configuration(), text).unwrap();
    }
    (env, files)
}

const MULTI_MEMBER_POLICY: &str = r#"
version = "0.1"

[group_member.pod-0.egress]
local_addresses = ["198.51.100.10"]

[group_member.pod-0.daemon.tls]
cert_file = "/etc/certs/pod-0.pem"
key_file = "/etc/certs/pod-0.key"

[group_member.pod-0.browser]
executable_path = "/opt/browsers/pod-0-chromium"

[group_member.pod-1.egress]
local_addresses = ["198.51.100.20"]

[group_member.pod-1.daemon.tls]
upstream = true
"#;

const MULTI_MEMBER_USER: &str = r#"
version = "0.1"

[group_member.pod-0.tls.trusted_cas]
"intranet.example" = ["/etc/ca/pod-0-ca.pem"]

[group_member.pod-0.tls.client_certificates."mtls.example"]
certificate = "/etc/client/pod-0.pem"
key = "env:POD0_CLIENT_KEY"

[group_member.pod-0.session.source]
kind = "profile_directory"
path = "/var/profiles/pod-0"
browser = "chrome"
domains = ["example.test"]
"#;

/// Covers: story:3-93, setting:egress.local_addresses, setting:daemon.tls.cert_file,
/// setting:daemon.tls.key_file, setting:tls.trusted_cas, setting:tls.client_certificates,
/// setting:browser.executable_path, setting:session.source
#[test]
fn group_member_scope_applies_machine_specific_settings_only_on_its_member() {
    let (_env, files) = setup_files(Some(MULTI_MEMBER_POLICY), Some(MULTI_MEMBER_USER));

    // pod-0 sees every machine-specific setting scoped to pod-0.
    let pod0 = files
        .load_for(Product::Collie, Some("pod-0"))
        .unwrap()
        .resolve(Product::Collie)
        .unwrap();
    for (setting, expected) in [
        (&EGRESS_LOCAL_ADDRESSES, json!(["198.51.100.10"])),
        (&TLS_CERT_FILE, json!("/etc/certs/pod-0.pem")),
        (&TLS_KEY_FILE, json!("/etc/certs/pod-0.key")),
        (
            &TLS_TRUSTED_CAS,
            json!({ "intranet.example": ["/etc/ca/pod-0-ca.pem"] }),
        ),
        (
            &TLS_CLIENT_CERTIFICATES,
            json!({
                "mtls.example": {
                    "certificate": "/etc/client/pod-0.pem",
                    "key": "env:POD0_CLIENT_KEY"
                }
            }),
        ),
        (
            &BROWSER_EXECUTABLE_PATH,
            json!("/opt/browsers/pod-0-chromium"),
        ),
        (
            &SESSION_SOURCE,
            json!({
                "kind": "profile_directory",
                "path": "/var/profiles/pod-0",
                "browser": "chrome",
                "domains": ["example.test"]
            }),
        ),
    ] {
        let effective = pod0.effective(setting).unwrap();
        assert_eq!(
            effective.layer,
            SettingsLayer::GroupMember,
            "{}",
            setting.name
        );
        assert_eq!(effective.value, expected, "{}", setting.name);
    }

    // pod-1 sees only pod-1's settings and NONE of pod-0's.
    let pod1 = files
        .load_for(Product::Collie, Some("pod-1"))
        .unwrap()
        .resolve(Product::Collie)
        .unwrap();
    assert_eq!(
        pod1.effective(&EGRESS_LOCAL_ADDRESSES).unwrap().value,
        json!(["198.51.100.20"])
    );
    assert_eq!(pod1.effective(&TLS_UPSTREAM).unwrap().value, json!(true));
    assert_eq!(pod1.effective(&TLS_CERT_FILE).unwrap().value, json!(null));
    assert_eq!(
        pod1.effective(&BROWSER_EXECUTABLE_PATH).unwrap().value,
        json!(null)
    );
    assert_eq!(pod1.effective(&TLS_TRUSTED_CAS).unwrap().value, json!({}));
    assert_eq!(pod1.effective(&SESSION_SOURCE).unwrap().value, json!(null));

    // A standalone Daemon (no Group member name) applies neither member's scope.
    let standalone = files
        .load(Product::Collie)
        .unwrap()
        .resolve(Product::Collie)
        .unwrap();
    assert_eq!(
        standalone.effective(&EGRESS_LOCAL_ADDRESSES).unwrap().value,
        json!(null)
    );
    assert_eq!(
        standalone.effective(&TLS_CERT_FILE).unwrap().value,
        json!(null)
    );
    assert_eq!(
        standalone
            .effective(&BROWSER_EXECUTABLE_PATH)
            .unwrap()
            .value,
        json!(null)
    );
}

#[test]
fn non_machine_specific_setting_in_group_member_scope_is_refused() {
    let (_env, files) = setup_files(
        Some(
            r#"
version = "0.1"

[group_member.pod-0.timeout]
navigation = 5000
"#,
        ),
        None,
    );

    let error = files.load_for(Product::Collie, Some("pod-0")).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsError::NotApplicable {
                ref key,
                layer: SettingsLayer::GroupMember,
                ..
            } if key == "timeout.navigation"
        ),
        "expected setting_not_applicable for timeout.navigation, got {error:?}"
    );
}

#[test]
fn operator_policy_lock_holds_over_group_member_scope() {
    let (_env, files) = setup_files(
        Some(
            r#"
version = "0.1"

[lock.browser]
executable_path = "/usr/bin/approved-chromium"

[group_member.pod-0.browser]
executable_path = "/tmp/other-chromium"
"#,
        ),
        None,
    );

    let error = files
        .load_for(Product::Collie, Some("pod-0"))
        .unwrap()
        .resolve(Product::Collie)
        .unwrap_err();
    assert!(
        matches!(
            error,
            SettingsError::Locked {
                ref key,
                layer: SettingsLayer::GroupMember,
                ..
            } if key == "browser.executable_path"
        ),
        "expected setting_locked when Operator policy locks the key, got {error:?}"
    );
}

#[test]
fn config_explain_reports_group_member_layer() {
    let (_env, files) = setup_files(
        Some(
            r#"
version = "0.1"

[group_member.pod-0.browser]
executable_path = "/opt/chrome"
"#,
        ),
        None,
    );

    let loaded = files.load_for(Product::Collie, Some("pod-0")).unwrap();
    let file_list = loaded.files.clone();
    let resolved = loaded.resolve(Product::Collie).unwrap();
    let explained = resolved.explain(&["browser.executable_path".to_owned()], file_list);
    assert_eq!(explained.settings.len(), 1);
    assert_eq!(explained.settings[0].layer, SettingsLayer::GroupMember);
    assert_eq!(explained.settings[0].value, json!("/opt/chrome"));
}
