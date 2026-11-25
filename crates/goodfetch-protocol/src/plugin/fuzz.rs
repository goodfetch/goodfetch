//! The bodies of the plugin protocol's fuzz targets (ADR 0067), the
//! engine's decoders of what a Plugin wrote: its manifest, and the
//! newline-delimited messages it sends over its stdio. Each body is a plain
//! function, which `fuzz/fuzz_targets/` calls under libFuzzer and
//! `fuzz_replay!` replays over the committed seeds in the Quick suite.

use serde::Deserialize;
use serde_json::Value;

use super::{
    CANCEL, HELLO, PING, PLUGIN_PROTOCOL, PluginCancelParams, PluginHelloParams, PluginHelloResult,
    PluginManifest, PluginPingParams, check_hello,
};
use crate::jsonrpc::{META, Message, Notification, Request, RequestMeta, Response};

/// The manifest every fuzzed `hello` result is held to: a Sample store
/// that speaks the engine's protocol range.
const APPROVED: &str = r#"{
    "manifest_version": "0.1",
    "name": "sample-store",
    "version": "1.0.0",
    "kinds": ["sample_store"],
    "protocol": { "min": "0.1", "max": "0.1" },
    "entrypoint": "sample-store"
}"#;

/// Reads `data` as a Plugin manifest: one that reads has the version
/// `version_of` reads first, and writes JSON that reads back as itself, its
/// settings schema aside, whose numbers may not round-trip exactly.
pub fn fuzz_plugin_manifest(data: &[u8]) {
    let Ok(manifest) = PluginManifest::read(data) else {
        return;
    };
    assert_eq!(
        PluginManifest::version_of(data).ok(),
        Some(manifest.manifest_version),
        "the version read first is the manifest's"
    );
    let written = serde_json::to_vec(&manifest).expect("invariant: a manifest serializes");
    let mut again = PluginManifest::read(&written).expect("a manifest that read reads back");
    assert_eq!(
        again.settings_schema.is_some(),
        manifest.settings_schema.is_some(),
        "a settings schema reads back"
    );
    again.settings_schema.clone_from(&manifest.settings_schema);
    assert_eq!(again, manifest, "a manifest that read reads back as itself");
}

/// Reads each line of `data` as a plugin protocol message, as a Plugin's
/// stdout carries them: one that reads writes a line that reads back as
/// the same kind of message; the params of `hello`, `$/ping` and
/// `$/cancel`, their `_meta` included, decode or are refused; and a result
/// that decodes as a `hello` result is held to an approved manifest, which
/// accepts it only in a version both sides speak.
pub fn fuzz_plugin_messages(data: &[u8]) {
    let approved =
        PluginManifest::read(APPROVED.as_bytes()).expect("invariant: the approved manifest reads");
    for line in data.split(|byte| *byte == b'\n') {
        let Ok(message) = serde_json::from_slice::<Message>(line) else {
            continue;
        };
        let written = serde_json::to_vec(&message).expect("invariant: a message serializes");
        let again: Message =
            serde_json::from_slice(&written).expect("a message that read reads back");
        assert_eq!(
            std::mem::discriminant(&again),
            std::mem::discriminant(&message),
            "a message reads back as the same kind of message"
        );
        match message {
            Message::Request(Request { method, params, .. }) => {
                let params = params.unwrap_or_default();
                if let Some(meta) = params.get(META) {
                    let _ = RequestMeta::deserialize(meta);
                }
                if method == HELLO {
                    let _ = PluginHelloParams::deserialize(&params);
                } else if method == PING {
                    let _ = PluginPingParams::deserialize(&params);
                }
            }
            Message::Notification(Notification { method, params, .. }) if method == CANCEL => {
                let _ = PluginCancelParams::deserialize(&params.unwrap_or_default());
            }
            Message::Response(Response { result, .. }) => hold_hello(&approved, &result),
            Message::Notification(_) | Message::Error(_) => {}
        }
    }
}

/// Holds `result`, when it decodes as a `hello` result, to `approved`.
fn hold_hello(approved: &PluginManifest, result: &Value) {
    let Ok(hello) = PluginHelloResult::deserialize(result) else {
        return;
    };
    if check_hello(PLUGIN_PROTOCOL, approved, &hello).is_ok() {
        assert!(
            PLUGIN_PROTOCOL.contains(hello.protocol_version)
                && approved.protocol.contains(hello.protocol_version),
            "an accepted hello speaks a version both sides speak"
        );
    }
}

#[cfg(test)]
mod plugin_manifest {
    goodfetch_test_support::fuzz_replay!(super::fuzz_plugin_manifest, "fuzz/plugin_manifest");
}

#[cfg(test)]
mod plugin_messages {
    goodfetch_test_support::fuzz_replay!(super::fuzz_plugin_messages, "fuzz/plugin_messages");
}
