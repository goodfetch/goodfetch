//! Pinned local model servers (ADRs 0065 and 0076; issue 427): starts a
//! pinned `llama.cpp` server (`llama-server`) or `Ollama` (`ollama serve`)
//! against a digest-verified small CPU GGUF model on a free loopback port,
//! reports the server version and model digest, and stops the server when
//! dropped.
//!
//! Binaries and model weights are fetched before ADR 0043's outbound-traffic
//! deny by `ci/local-models-fetch.sh` from `test-support/local-models/pins.toml`
//! into `$GOODFETCH_LOCAL_MODELS_DOWNLOADS` (default
//! `~/.cache/goodfetch/local-models`). `LocalModel::start` never downloads
//! anything; it fails with [`LocalModelError::NotFetched`] when missing and
//! [`LocalModelError::DigestMismatch`] when the model file's SHA-256 differs
//! from the expected pin.
#![expect(
    clippy::disallowed_methods,
    reason = "test support hashes the model file, starts local servers and polls health on the real clock, outside any async worker (ADR 0062)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test support binds a loopback listener on port 0 to pick a free port, outside any async worker (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::browsers::downloads;
use crate::env::TestEnv;
use crate::process::Process;
use crate::variables::LOCAL_MODELS_DOWNLOADS_VARIABLE;

/// Maximum time a local model server has to answer its health endpoint.
pub const LOCAL_MODEL_DEADLINE: Duration = Duration::from_secs(30);

const PINS: &str = include_str!("../../../test-support/local-models/pins.toml");
const POLL: Duration = Duration::from_millis(25);
const STARTS: usize = 3;

/// Local model server supported by [`LocalModel::start`] (ADR 0065).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalModelKind {
    /// Pinned `llama.cpp` HTTP server (`llama-server`).
    LlamaCpp,
    /// Pinned Ollama HTTP server (`ollama serve`).
    Ollama,
}

impl LocalModelKind {
    /// Short name used in logs and process records.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LlamaCpp => "llama-cpp",
            Self::Ollama => "ollama",
        }
    }

    const fn default_health_path(self) -> &'static str {
        match self {
            Self::LlamaCpp => "/health",
            Self::Ollama => "/api/version",
        }
    }
}

/// Pinned model and server definitions parsed from `test-support/local-models/pins.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalModelPins {
    pub model: PinnedModel,
    pub llama_cpp: PinnedServer,
    pub ollama: PinnedServer,
}

/// Pinned small CPU GGUF model.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedModel {
    pub name: String,
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// Pinned local model server release.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedServer {
    pub version: String,
    pub platforms: BTreeMap<String, PinnedServerPlatform>,
}

/// Per-platform download archive and binary path for a pinned server.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedServerPlatform {
    pub url: String,
    pub sha256: String,
    pub binary: String,
}

/// Parses the committed `test-support/local-models/pins.toml`.
///
/// # Errors
///
/// Returns [`LocalModelError::Pins`] if the pins file fails to parse.
pub fn local_model_pins() -> Result<LocalModelPins, LocalModelError> {
    toml::from_str(PINS).map_err(LocalModelError::Pins)
}

/// Configuration for [`LocalModel::start_with`], allowing Seam B tests to drive
/// any executable with explicit version and expected model SHA-256 digest.
#[derive(Debug, Clone)]
pub struct LocalModelConfig {
    pub kind: LocalModelKind,
    pub executable: PathBuf,
    pub version: String,
    pub model_path: PathBuf,
    pub expected_model_sha256: String,
    pub extra_args: Vec<String>,
    pub health_path: Option<String>,
}

/// Why a local model server failed to start or verify.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LocalModelError {
    /// `test-support/local-models/pins.toml` failed to parse.
    #[error("test-support/local-models/pins.toml does not parse: {0}")]
    Pins(#[from] toml::de::Error),
    /// No local model binary is pinned for this OS/architecture.
    #[error("no pinned local model server for this platform")]
    UnsupportedPlatform,
    /// The pinned server or model file has not been fetched before the deny.
    #[error(
        "{what} is not at {path}: run `GOODFETCH_FETCH_LOCAL_MODELS=yes ci/local-models-fetch.sh` before the deny"
    )]
    NotFetched { what: String, path: PathBuf },
    /// The model file's SHA-256 digest differs from the pinned digest.
    #[error("model digest mismatch for {path}: expected sha256:{expected}, got sha256:{actual}")]
    DigestMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// Failed to allocate a free loopback port, read the model, or spawn the child.
    #[error("local model I/O error: {0}")]
    Io(#[from] io::Error),
    /// The server exited early or did not answer its health endpoint before the deadline.
    #[error("local model server {kind} did not become ready on {address}: {detail}")]
    NotReady {
        kind: &'static str,
        address: SocketAddr,
        detail: String,
    },
}

/// A running local model server on loopback, stopped automatically on [`Drop`].
pub struct LocalModel {
    kind: LocalModelKind,
    address: SocketAddr,
    version: String,
    model_digest: String,
    process: Process,
}

impl LocalModel {
    /// Starts the pinned server `kind` from `$GOODFETCH_LOCAL_MODELS_DOWNLOADS`
    /// (default `~/.cache/goodfetch/local-models`).
    ///
    /// # Errors
    ///
    /// Returns [`LocalModelError`] if the server or model is missing, the model's
    /// SHA-256 does not match the pin, or the server fails to become ready.
    pub fn start(env: &TestEnv, kind: LocalModelKind) -> Result<Self, LocalModelError> {
        let cache = downloads(LOCAL_MODELS_DOWNLOADS_VARIABLE, "local-models");
        Self::start_from_cache(env, kind, &cache)
    }

    /// Starts the pinned server `kind` from an explicit local-models cache directory.
    ///
    /// # Errors
    ///
    /// Returns [`LocalModelError`] if the server or model is missing, the model's
    /// SHA-256 does not match the pin, or the server fails to become ready.
    pub fn start_from_cache(
        env: &TestEnv,
        kind: LocalModelKind,
        cache: &Path,
    ) -> Result<Self, LocalModelError> {
        let pins = local_model_pins()?;
        let platform = current_platform().ok_or(LocalModelError::UnsupportedPlatform)?;
        let (server_pin, subdir) = match kind {
            LocalModelKind::LlamaCpp => (&pins.llama_cpp, "llama-cpp"),
            LocalModelKind::Ollama => (&pins.ollama, "ollama"),
        };
        let plat = server_pin
            .platforms
            .get(platform)
            .ok_or(LocalModelError::UnsupportedPlatform)?;
        let executable = cache
            .join(subdir)
            .join(&server_pin.version)
            .join(&plat.binary);
        if !executable.is_file() {
            return Err(LocalModelError::NotFetched {
                what: format!("{} {}", kind.name(), server_pin.version),
                path: executable,
            });
        }
        let model_path = cache.join("models").join(&pins.model.file);
        if !model_path.is_file() {
            return Err(LocalModelError::NotFetched {
                what: format!("model {}", pins.model.name),
                path: model_path,
            });
        }
        Self::start_with(
            env,
            LocalModelConfig {
                kind,
                executable,
                version: server_pin.version.clone(),
                model_path,
                expected_model_sha256: pins.model.sha256,
                extra_args: Vec::new(),
                health_path: None,
            },
        )
    }

    /// Verifies `config.model_path`'s SHA-256 against `config.expected_model_sha256`
    /// and starts the server on a free loopback port.
    ///
    /// # Errors
    ///
    /// Returns [`LocalModelError::DigestMismatch`] before spawning if the model
    /// bytes do not match, or [`LocalModelError::NotReady`] if the server fails
    /// to answer its health endpoint.
    pub fn start_with(env: &TestEnv, config: LocalModelConfig) -> Result<Self, LocalModelError> {
        let verified_hex = verify_model_digest(&config.model_path, &config.expected_model_sha256)?;
        let model_digest = format!("sha256:{verified_hex}");
        let health_path = config
            .health_path
            .as_deref()
            .unwrap_or_else(|| config.kind.default_health_path());

        let mut last_error = None;
        for attempt in 0..STARTS {
            let address = free_loopback_port()?;
            let log_path = env
                .folder()
                .join(format!("local-model-{}-{attempt}.log", config.kind.name()));
            let process = spawn_server(env, &config, address, &log_path)?;
            match wait_ready(&process, address, health_path, config.kind) {
                Ok(()) => {
                    return Ok(Self {
                        kind: config.kind,
                        address,
                        version: config.version,
                        model_digest,
                        process,
                    });
                }
                Err(error) => {
                    let _ = process.stop();
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.expect("invariant: STARTS >= 1 populates last_error on failure"))
    }

    /// Which server kind is running.
    #[must_use]
    pub const fn kind(&self) -> LocalModelKind {
        self.kind
    }

    /// Loopback socket address the server listens on.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Base HTTP URL (`http://127.0.0.1:<port>`).
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Pinned server version reported for result records (ADR 0065).
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Verified model SHA-256 digest (`sha256:<hex>`) reported for result records.
    #[must_use]
    pub fn model_digest(&self) -> &str {
        &self.model_digest
    }
}

impl fmt::Debug for LocalModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalModel")
            .field("kind", &self.kind)
            .field("address", &self.address)
            .field("version", &self.version)
            .field("model_digest", &self.model_digest)
            .finish_non_exhaustive()
    }
}

impl Drop for LocalModel {
    fn drop(&mut self) {
        let _ = self.process.stop();
    }
}

/// Computes the SHA-256 of `path` and verifies it matches `expected` (with or
/// without a leading `sha256:` prefix).
///
/// # Errors
///
/// Returns [`LocalModelError::DigestMismatch`] on mismatch or [`LocalModelError::Io`]
/// if `path` cannot be read.
pub fn verify_model_digest(path: &Path, expected: &str) -> Result<String, LocalModelError> {
    let expected_hex = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .to_ascii_lowercase();
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual_hex = data_encoding::HEXLOWER.encode(&hasher.finalize());
    if actual_hex != expected_hex {
        return Err(LocalModelError::DigestMismatch {
            path: path.to_path_buf(),
            expected: expected_hex,
            actual: actual_hex,
        });
    }
    Ok(actual_hex)
}

fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-arm64")
    } else {
        None
    }
}

fn free_loopback_port() -> Result<SocketAddr, LocalModelError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?)
}

fn spawn_server(
    env: &TestEnv,
    config: &LocalModelConfig,
    address: SocketAddr,
    log_path: &Path,
) -> Result<Process, LocalModelError> {
    let output = File::create(log_path)?;
    let stderr = output.try_clone()?;
    let mut cmd = env.process_command(&config.executable);
    // Preserve PATH so script-based or dynamically-linked servers resolve their runtime.
    if let Ok(path_env) = std::env::var("PATH") {
        cmd.env("PATH", path_env);
    }
    match config.kind {
        LocalModelKind::LlamaCpp => {
            cmd.args([
                "--host",
                &address.ip().to_string(),
                "--port",
                &address.port().to_string(),
                "--model",
            ])
            .arg(&config.model_path);
        }
        LocalModelKind::Ollama => {
            let models_dir = config
                .model_path
                .parent()
                .map_or_else(|| env.folder().to_path_buf(), Path::to_path_buf);
            fs::create_dir_all(&models_dir)?;
            cmd.arg("serve")
                .env("OLLAMA_HOST", address.to_string())
                .env("OLLAMA_MODELS", models_dir);
        }
    }
    cmd.args(&config.extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(stderr));
    Ok(env.spawn_with_streams(&mut cmd)?)
}

fn wait_ready(
    process: &Process,
    address: SocketAddr,
    health_path: &str,
    kind: LocalModelKind,
) -> Result<(), LocalModelError> {
    let url = format!("http://{address}{health_path}");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let deadline = Instant::now() + LOCAL_MODEL_DEADLINE;
    while Instant::now() < deadline {
        if let Some(status) = process.child().try_wait()? {
            return Err(LocalModelError::NotReady {
                kind: kind.name(),
                address,
                detail: format!("exited before ready with {status}"),
            });
        }
        if let Ok(resp) = agent.get(&url).call()
            && resp.status().as_u16() < 500
        {
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
    Err(LocalModelError::NotReady {
        kind: kind.name(),
        address,
        detail: format!("timed out after {LOCAL_MODEL_DEADLINE:?} polling {url}"),
    })
}
