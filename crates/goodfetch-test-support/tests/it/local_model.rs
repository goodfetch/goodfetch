//! Seam B (`LocalModel::start`, ADRs 0065 and 0076; issue 427):
//! starts pinned llama.cpp server and Ollama on free loopback ports, verifies
//! model SHA-256 digests before launch (failing on mismatch), registers the
//! GGUF model in Ollama via `ollama create -f Modelfile`, reports server
//! version and model digest, and stops each server when dropped.
//! Declares no Feature (#325, Out of Scope, ambiguity 9).

use std::fs;
use std::path::PathBuf;

use goodfetch_test_support::{
    LOCAL_MODELS_DOWNLOADS_VARIABLE, LocalModel, LocalModelConfig, LocalModelError, LocalModelKind,
    TestEnv, local_model_pins,
};
use rstest::rstest;
use sha2::{Digest as _, Sha256};

/// Writes a tiny local HTTP server script (Node, zero outbound network) that
/// mimics either `llama-server` (`--host`/`--port`/`--model`, `/health`) or
/// `ollama serve` + `ollama create <name> -f <Modelfile>` (`OLLAMA_HOST`,
/// `/api/version`) plus `/v1/chat/completions`.
fn write_fixture_server(
    env: &TestEnv,
    name: &str,
    marker: &std::path::Path,
    registered_marker: &std::path::Path,
) -> PathBuf {
    let script = env.folder().join(name);
    let marker_json = serde_json::to_string(&marker.display().to_string()).unwrap();
    let reg_json = serde_json::to_string(&registered_marker.display().to_string()).unwrap();
    let body = format!(
        r#"#!/usr/bin/env node
import {{ readFileSync, writeFileSync }} from "node:fs";
import {{ createServer }} from "node:http";

const args = process.argv.slice(2);
if (args[0] === "create") {{
  const modelName = args[1] ?? "";
  const fIdx = args.indexOf("-f");
  const modelfile = fIdx >= 0 ? readFileSync(args[fIdx + 1], "utf8") : "";
  writeFileSync({reg_json}, JSON.stringify({{ modelName, modelfile, host: process.env.OLLAMA_HOST }}));
  process.exit(0);
}}

writeFileSync({marker_json}, "started\n");

let host = "127.0.0.1";
let port = 0;
for (let i = 0; i < args.length; i += 1) {{
  if (args[i] === "--host" && args[i + 1]) host = args[i + 1];
  if (args[i] === "--port" && args[i + 1]) port = Number(args[i + 1]);
}}
if (port === 0 && process.env.OLLAMA_HOST) {{
  const [h, p] = process.env.OLLAMA_HOST.split(":");
  host = h;
  port = Number(p);
}}

const server = createServer((req, res) => {{
  if (req.url === "/health" || req.url === "/api/version") {{
    res.writeHead(200, {{ "content-type": "application/json" }});
    res.end(JSON.stringify({{ status: "ok", version: "fixture-1.0" }}));
    return;
  }}
  if (req.url === "/v1/chat/completions") {{
    res.writeHead(200, {{ "content-type": "application/json" }});
    res.end(JSON.stringify({{
      id: "chatcmpl-local",
      choices: [{{ index: 0, message: {{ role: "assistant", content: "ok" }} }}],
      usage: {{ prompt_tokens: 4, completion_tokens: 1 }}
    }}));
    return;
  }}
  res.writeHead(404);
  res.end();
}});

server.listen(port, host);
process.on("SIGTERM", () => server.close(() => process.exit(0)));
"#
    );
    fs::write(&script, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }
    script
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    data_encoding::HEXLOWER.encode(&digest)
}

#[test]
fn pinned_manifest_declares_llama_cpp_ollama_and_model_digests() {
    let pins = local_model_pins().unwrap();
    assert_ne!(pins.llama_cpp.version, "");
    assert_ne!(pins.ollama.version, "");
    assert_eq!(pins.model.sha256.len(), 64);
    assert!(pins.llama_cpp.platforms.contains_key("linux-x64"));
    assert!(pins.ollama.platforms.contains_key("linux-x64"));
}

#[cfg(unix)]
#[rstest]
#[case::llama_cpp(LocalModelKind::LlamaCpp, "b12810")]
#[case::ollama(LocalModelKind::Ollama, "v0.40.18")]
fn start_with_starts_on_free_port_reports_version_and_digest_and_stops_on_drop(
    #[case] kind: LocalModelKind,
    #[case] version: &str,
) {
    let env = TestEnv::new();
    let model_path = env.folder().join("tiny.gguf");
    let model_bytes = b"GGUF-fixture-weights-0123456789";
    fs::write(&model_path, model_bytes).unwrap();
    let expected_hex = sha256_hex(model_bytes);

    let marker = env.folder().join(format!("started-{kind:?}.flag"));
    let registered = env.folder().join(format!("registered-{kind:?}.json"));
    let script = write_fixture_server(&env, &format!("server-{kind:?}.mjs"), &marker, &registered);
    let server = LocalModel::start_with(
        &env,
        LocalModelConfig {
            kind,
            executable: script,
            version: version.to_owned(),
            model_name: "tiny-gguf".to_owned(),
            model_path: model_path.clone(),
            expected_model_sha256: expected_hex.clone(),
            extra_args: Vec::new(),
            health_path: None,
        },
    )
    .unwrap();

    assert_eq!(server.kind(), kind);
    assert_eq!(server.version(), version);
    assert_eq!(server.model_name(), "tiny-gguf");
    assert_eq!(server.model_digest(), format!("sha256:{expected_hex}"));
    assert!(server.address().port() > 0);
    assert!(marker.is_file());
    if kind == LocalModelKind::Ollama {
        let reg_text = fs::read_to_string(&registered).unwrap();
        assert!(reg_text.contains("FROM "));
        assert!(reg_text.contains("tiny.gguf"));
    }

    let resp = ureq::get(&format!("{}/v1/chat/completions", server.base_url()))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);

    let base_url = server.base_url();
    drop(server);

    // Server is stopped on drop so requests to its address fail.
    assert!(
        ureq::get(&format!("{base_url}/health")).call().is_err(),
        "server still accepting connections after drop on {base_url}"
    );
}

#[cfg(unix)]
#[test]
fn start_with_fails_on_mismatched_model_digest_without_starting_server() {
    let env = TestEnv::new();
    let model_path = env.folder().join("tampered.gguf");
    fs::write(&model_path, b"tampered-bytes").unwrap();
    let marker = env.folder().join("must-not-start.flag");
    let registered = env.folder().join("must-not-register.json");
    let script = write_fixture_server(&env, "never-started.mjs", &marker, &registered);

    let err = LocalModel::start_with(
        &env,
        LocalModelConfig {
            kind: LocalModelKind::LlamaCpp,
            executable: script,
            version: "b12810".to_owned(),
            model_name: "tampered".to_owned(),
            model_path,
            expected_model_sha256: "0".repeat(64),
            extra_args: Vec::new(),
            health_path: None,
        },
    )
    .unwrap_err();

    assert!(
        matches!(err, LocalModelError::DigestMismatch { .. }),
        "expected DigestMismatch, got {err:?}"
    );
    assert!(
        !marker.exists(),
        "server must not start when model digest does not match"
    );
}

#[test]
fn start_fails_with_not_fetched_when_local_model_cache_is_empty() {
    let env = TestEnv::new();
    let empty_cache = env.folder().join("empty-cache");
    fs::create_dir_all(&empty_cache).unwrap();

    // Point lookup at an empty folder without touching global process env.
    let err =
        LocalModel::start_from_cache(&env, LocalModelKind::LlamaCpp, &empty_cache).unwrap_err();
    assert!(
        matches!(err, LocalModelError::NotFetched { .. }),
        "expected NotFetched, got {err:?}"
    );
    assert_eq!(
        LOCAL_MODELS_DOWNLOADS_VARIABLE,
        "GOODFETCH_LOCAL_MODELS_DOWNLOADS"
    );
}

/// Release-suite Linux cell local-model check (ADR 0065): runs `LocalModel::start`
/// when `ci/local-models-fetch.sh` has populated the pinned servers before the
/// deny (as `release-suite.yml` sets `GOODFETCH_FETCH_LOCAL_MODELS=yes`), and
/// returns cleanly when invoked outside a fetched Release cell.
#[cfg(target_os = "linux")]
#[test]
fn release_suite_local_model_start_runs_when_fetched() {
    let env = TestEnv::new();
    for kind in [LocalModelKind::LlamaCpp, LocalModelKind::Ollama] {
        match LocalModel::start(&env, kind) {
            Ok(server) => {
                assert_ne!(server.version(), "");
                assert!(server.model_digest().starts_with("sha256:"));
                let resp = ureq::get(&format!("{}/v1/models", server.base_url()))
                    .call()
                    .unwrap();
                assert_eq!(resp.status(), 200);
                drop(server);
            }
            Err(LocalModelError::NotFetched { .. }) => {
                // Only Release suite Linux cells set GOODFETCH_FETCH_LOCAL_MODELS=yes.
            }
            Err(other) => panic!("unexpected LocalModel::start({kind:?}) error: {other}"),
        }
    }
}
