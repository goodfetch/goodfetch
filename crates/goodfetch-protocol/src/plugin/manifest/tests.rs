use rstest::rstest;
use serde_json::{Value, json};

use super::{Limits, ManifestError, NetworkLevel, PluginManifest, Runtime};
use crate::catalog::ProtocolVersion;

/// The least a manifest says: a Proxy provider run as an executable.
fn least() -> Value {
    json!({
        "manifest_version": "0.1",
        "name": "residential-proxies",
        "version": "0.3.0",
        "kinds": ["proxy"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "bin/proxies",
    })
}

/// `manifest` with `field` set to `value`.
fn with(mut manifest: Value, field: &str, value: Value) -> Vec<u8> {
    manifest[field] = value;
    manifest.to_string().into_bytes()
}

/// Covers: story:7-100
#[test]
fn a_manifest_silent_on_its_limits_and_network_level_takes_the_defaults() {
    let manifest = PluginManifest::read(least().to_string().as_bytes()).unwrap();

    assert_eq!(
        (
            manifest.limits,
            manifest.network_level,
            manifest.runtime,
            manifest.env.is_empty()
        ),
        (
            Limits {
                memory: 256 * 1024 * 1024,
                cpu_time: 3600,
                processes: 1,
                open_files: 128,
                concurrency: 4,
            },
            NetworkLevel::None,
            Runtime::None,
            true,
        )
    );
}

/// Covers: story:7-100
#[test]
fn a_limit_the_manifest_sets_replaces_only_that_default() {
    let manifest =
        PluginManifest::read(&with(least(), "limits", json!({ "concurrency": 16 }))).unwrap();

    assert_eq!(
        manifest.limits,
        Limits {
            concurrency: 16,
            ..Limits::default()
        }
    );
}

/// Covers: story:7-100
#[rstest]
#[case::a_hook_at_none(&["hook"], "none", true)]
#[case::a_hook_at_proxy_tcp(&["hook"], "proxy_tcp", false)]
#[case::an_extractor_at_direct(&["extract"], "direct", false)]
#[case::a_hook_and_a_model_provider_at_direct(&["hook", "model"], "direct", false)]
#[case::a_proxy_provider_at_proxy_tcp(&["proxy"], "proxy_tcp", true)]
#[case::a_proxy_provider_at_direct(&["proxy"], "direct", false)]
#[case::a_browser_backend_at_direct(&["browser"], "direct", false)]
#[case::a_model_and_secret_provider_at_direct(&["model", "secret"], "direct", true)]
#[case::a_decision_provider_code_source_and_notification_channel_at_direct(
    &["decision", "code", "notification"],
    "direct",
    true
)]
#[case::a_sink_at_direct(&["sink"], "direct", true)]
#[case::a_model_provider_and_search_provider_at_direct(&["model", "search"], "direct", false)]
#[case::an_unknown_kind_at_proxy_tcp(&["added_by_a_newer_engine"], "proxy_tcp", true)]
#[case::an_unknown_kind_at_direct(&["added_by_a_newer_engine"], "direct", false)]
fn a_network_level_is_accepted_only_where_every_kind_allows_it(
    #[case] kinds: &[&str],
    #[case] level: &str,
    #[case] accepted: bool,
) {
    let mut manifest = least();
    manifest["kinds"] = json!(kinds);
    manifest["network_level"] = json!(level);

    let read = PluginManifest::read(manifest.to_string().as_bytes());

    assert_eq!(
        read.map(|manifest| manifest.network_level.as_str())
            .map_err(|error| matches!(error, ManifestError::NetworkLevel { .. })),
        if accepted { Ok(level) } else { Err(true) }
    );
}

/// Covers: story:7-100
#[rstest]
#[case::the_dynamic_linkers_preload("LD_PRELOAD")]
#[case::the_dynamic_linkers_path("LD_LIBRARY_PATH")]
#[case::macos_insertion("DYLD_INSERT_LIBRARIES")]
#[case::node_options("NODE_OPTIONS")]
#[case::node_path("NODE_PATH")]
#[case::python_path("PYTHONPATH")]
#[case::python_home("PYTHONHOME")]
#[case::python_startup("PYTHONSTARTUP")]
#[case::in_lowercase("ld_preload")]
fn a_loader_variable_in_the_environment_is_refused(#[case] variable: &str) {
    let mut env = json!({ "SHOP_REGION": "region" });
    env[variable] = json!("loader");
    let manifest = with(least(), "env", env);

    let refused = PluginManifest::read(&manifest).unwrap_err();

    assert!(
        matches!(&refused, ManifestError::LoaderVariable(named) if named == variable),
        "{refused:?}"
    );
}

/// Covers: story:7-100
#[test]
fn a_variable_that_loads_no_code_is_accepted() {
    let manifest = with(
        least(),
        "env",
        json!({ "SHOP_REGION": "region", "OLD_PRELOAD": "preload" }),
    );

    assert_eq!(PluginManifest::read(&manifest).unwrap().env.len(), 2);
}

/// Covers: story:7-100
#[test]
fn a_field_the_manifest_format_does_not_have_is_refused() {
    let manifest = with(least(), "added_in_a_minor", json!(true));

    assert!(matches!(
        PluginManifest::read(&manifest),
        Err(ManifestError::Malformed(_))
    ));
}

/// The version is read before the rest, so that a manifest of a newer
/// minor, which may add fields, is held to the format's rule rather than
/// failing on a field (ADR 0066).
///
/// Covers: story:7-100
#[test]
fn the_version_of_a_newer_manifest_is_read_without_the_rest() {
    let newer = json!({ "manifest_version": "0.2", "added_in_a_minor": { "any": "shape" } });

    assert_eq!(
        PluginManifest::version_of(newer.to_string().as_bytes()).unwrap(),
        ProtocolVersion { major: 0, minor: 2 }
    );
}

/// Covers: story:7-100
#[test]
fn a_manifest_reads_back_as_it_was_written() {
    let written = json!({
        "manifest_version": "0.1",
        "name": "shop",
        "version": "2.0.0",
        "kinds": ["model", "secret"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "settings_schema": { "type": "object" },
        "runtime": { "kind": "node", "version": "22.11.0" },
        "entrypoint": "dist/main.js",
        "network_level": "direct",
        "permissions": { "domains": ["api.example.com"], "grants": ["secret_value", "effects"] },
        "env": { "SHOP_REGION": "region" },
        "files": ["/etc/ssl/certs"],
        "processes": ["/usr/bin/git"],
        "limits": {
            "memory": 1_073_741_824_u64,
            "cpu_time": 7200,
            "processes": 2,
            "open_files": 256,
            "concurrency": 8
        },
        "browsers": [{
            "name": "shop-browser",
            "platforms": {
                "linux-x64": { "url": "https://example.com/b.zip", "digest": "sha256:00" }
            }
        }],
        "spend": { "tokens": { "max_per_call": 4096, "price": "0.000002 USD" } },
        "provider_name": "shop",
        "models": [{
            "id": "large",
            "context_window": 200_000,
            "max_output_tokens": 8192,
            "image_input": true,
            "tool_calls": true,
            "strategies": ["native", "prompt"],
            "input_price": "3 USD",
            "output_price": "15 USD"
        }],
        "scheme": "shop",
    });

    let read = PluginManifest::read(written.to_string().as_bytes()).unwrap();

    assert_eq!(serde_json::to_value(&read).unwrap(), written);
}
