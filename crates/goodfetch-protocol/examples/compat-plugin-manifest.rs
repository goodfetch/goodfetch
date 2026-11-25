//! The Plugin manifest's writer for the Compatibility corpus (ADR 0066):
//! three manifests that together fill every field, a Hook on Node.js at
//! `none`, a Model and Secret provider on Python at `direct`, and a
//! Browser backend executable at `proxy_tcp`, each built from the
//! protocol's types and read back through the manifest's own reader, then
//! written into the folder given as the only argument as `hook.json`,
//! `model-provider.json` and `browser-backend.json`. `xtask compat write`
//! runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use goodfetch_protocol::plugin::{
    Download, Grant, HookStageName, Limits, MANIFEST_VERSION, ManifestBrowser, ManifestModel,
    NetworkLevel, PLUGIN_PROTOCOL, Permissions, PluginKindName, PluginManifest, Runtime, Spend,
    StructuredOutputStrategy,
};
use serde_json::json;

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    for (name, manifest) in [
        ("hook.json", hook()),
        ("model-provider.json", model_provider()),
        ("browser-backend.json", browser_backend()),
    ] {
        let mut text =
            serde_json::to_string_pretty(&manifest).expect("invariant: a manifest serializes");
        text.push('\n');
        let read = PluginManifest::read(text.as_bytes())
            .expect("invariant: the corpus's manifests are valid");
        assert_eq!(read, manifest, "{name} reads back as it was written");
        std::fs::write(output.join(name), text).expect("invariant: the output folder is writable");
    }
}

/// A manifest with every field at its default, which `name`, `kinds` and
/// `entrypoint` complete.
fn manifest(name: &str, kinds: &[PluginKindName], entrypoint: &str) -> PluginManifest {
    PluginManifest {
        manifest_version: MANIFEST_VERSION,
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        kinds: kinds.iter().copied().collect(),
        hook_stages: BTreeSet::new(),
        protocol: PLUGIN_PROTOCOL,
        settings_schema: None,
        runtime: Runtime::None,
        entrypoint: entrypoint.to_owned(),
        network_level: NetworkLevel::None,
        permissions: Permissions::default(),
        env: BTreeMap::new(),
        files: Vec::new(),
        processes: Vec::new(),
        limits: Limits::default(),
        browsers: Vec::new(),
        spend: BTreeMap::new(),
        provider_name: None,
        models: Vec::new(),
        scheme: None,
    }
}

/// A Hook on Node.js at `none`, with settings, an environment, files and
/// processes, and limits of its own.
fn hook() -> PluginManifest {
    PluginManifest {
        hook_stages: [HookStageName::BEFORE_NAVIGATION, HookStageName::PAGE_READY].into(),
        settings_schema: Some(json!({
            "type": "object",
            "properties": { "banner_selector": { "type": "string" } },
        })),
        runtime: Runtime::Node {
            version: "22.11.0".to_owned(),
        },
        permissions: Permissions {
            grants: [Grant::PageScript, Grant::Effects].into(),
            ..Permissions::default()
        },
        env: [("BANNER_SELECTOR".to_owned(), "banner_selector".to_owned())].into(),
        files: vec!["~/.config/consent-hook/rules.json".to_owned()],
        processes: vec!["/usr/bin/convert".to_owned()],
        limits: Limits {
            memory: 512 * 1024 * 1024,
            cpu_time: 10 * 60,
            processes: 2,
            open_files: 64,
            concurrency: 8,
        },
        ..manifest("consent-hook", &[PluginKindName::HOOK], "dist/index.js")
    }
}

/// A Model and Secret provider on Python at `direct`, with domains, Secret
/// references, spend, its models and its reference scheme.
fn model_provider() -> PluginManifest {
    PluginManifest {
        runtime: Runtime::Python {
            version: "3.13.1".to_owned(),
        },
        network_level: NetworkLevel::Direct,
        permissions: Permissions {
            domains: vec!["api.example-models.com".to_owned()],
            secrets: vec!["vault://models/api-key".to_owned()],
            grants: [Grant::SecretValue].into(),
        },
        spend: [(
            "tokens".to_owned(),
            Spend {
                max_per_call: 200_000,
                price: Some("0.000003 USD".to_owned()),
            },
        )]
        .into(),
        provider_name: Some("example".to_owned()),
        models: vec![ManifestModel {
            id: "example-large".to_owned(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            image_input: true,
            tool_calls: true,
            strategies: vec![
                StructuredOutputStrategy::Native,
                StructuredOutputStrategy::Tool,
                StructuredOutputStrategy::Prompt,
            ],
            input_price: Some("3 USD".to_owned()),
            output_price: Some("15 USD".to_owned()),
        }],
        scheme: Some("vault".to_owned()),
        ..manifest(
            "example-models",
            &[PluginKindName::MODEL, PluginKindName::SECRET],
            "example_models/main.py",
        )
    }
}

/// A Browser backend executable at `proxy_tcp`, which adds a browser by
/// download and digest and asks for a logged-in Session's credentials.
fn browser_backend() -> PluginManifest {
    PluginManifest {
        network_level: NetworkLevel::ProxyTcp,
        permissions: Permissions {
            grants: [Grant::SessionCredentials].into(),
            ..Permissions::default()
        },
        browsers: vec![ManifestBrowser {
            name: "remote-chromium".to_owned(),
            platforms: [(
                "linux-x64".to_owned(),
                Download {
                    url: "https://downloads.example.com/remote-chromium-141-linux-x64.zip"
                        .to_owned(),
                    digest:
                        "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
                            .to_owned(),
                },
            )]
            .into(),
        }],
        ..manifest(
            "remote-browser",
            &[PluginKindName::BROWSER],
            "bin/remote-browser",
        )
    }
}
