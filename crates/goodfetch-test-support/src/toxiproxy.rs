//! Toxiproxy (ADR 0060): a TCP proxy on loopback, between each real engine
//! and the Politeness store or between Group members, that a test cuts,
//! slows, partitions and heals through its HTTP API, on every OS. The test's
//! `resolve` map points each host name at its proxy's address, and each proxy
//! listens on a loopback address of its own, since `resolve` maps names, not
//! ports: `127.0.0.x` on Linux and Windows, the `lo0` aliases on macOS.
//!
//! The server is the pinned build of `test-support/toxiproxy/pins.toml`,
//! which `ci/toxiproxy-fetch.sh` (part of `just fetch`) downloads before the
//! outbound-traffic deny; no test downloads it. It runs in the test's cleared
//! environment, on a free loopback port, with the test's seed, its output in
//! the test's folder, and its process joins the test's record. Dropping the
//! value stops it.
#![expect(
    clippy::disallowed_methods,
    reason = "test support starts Toxiproxy, writes its output file and polls it on the real clock, outside any async worker and any simulated clock (ADR 0062)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test support asks the operating system for a free loopback port for Toxiproxy's API, outside any async worker (ADR 0060)"
)]

use std::fs::File;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::browsers::downloads;
use crate::env::TestEnv;
use crate::process::Process;
use crate::variables::TOXIPROXY_DOWNLOADS_VARIABLE;

/// How long Toxiproxy may take to answer, from its start or for one call.
pub const TOXIPROXY_DEADLINE: Duration = Duration::from_secs(10);

/// The pinned build of each platform, which `ci/toxiproxy-fetch.sh` reads
/// too.
const PINS: &str = include_str!("../../../test-support/toxiproxy/pins.toml");

/// How often a starting Toxiproxy is asked whether it answers.
const POLL: Duration = Duration::from_millis(10);

/// How many times a start is tried again when Toxiproxy exits before it
/// answers, as it does when another program took the free port between the
/// test's look and Toxiproxy's bind.
const STARTS: usize = 3;

/// The toxics a partition adds: data dropped both ways, and the connection
/// held open until they go.
const PARTITION: [(&str, &str); 2] = [
    ("partition-upstream", "upstream"),
    ("partition-downstream", "downstream"),
];

/// The toxic latency adds.
const LATENCY: &str = "latency";

#[derive(Deserialize)]
struct Pins {
    version: String,
    platforms: std::collections::BTreeMap<String, Pin>,
}

#[derive(Deserialize)]
struct Pin {
    asset: String,
}

/// The name `pins.toml` gives this platform.
fn platform() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("mac-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("mac-x64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

/// The tests' copy of the pinned Toxiproxy build for this platform, which
/// `ci/toxiproxy-fetch.sh` downloads: `<downloads>/<version>/<asset>`.
///
/// # Errors
///
/// [`ToxiproxyError::NoBuild`] on a platform with no pin,
/// [`ToxiproxyError::NotFetched`] when the copy is missing.
pub fn toxiproxy_copy() -> Result<PathBuf, ToxiproxyError> {
    let pins: Pins = toml::from_str(PINS).expect("invariant: pins.toml is committed valid");
    let pin = platform()
        .and_then(|platform| pins.platforms.get(platform))
        .ok_or(ToxiproxyError::NoBuild)?;
    let copy = downloads(TOXIPROXY_DOWNLOADS_VARIABLE, "toxiproxy")
        .join(&pins.version)
        .join(&pin.asset);
    if copy.is_file() {
        Ok(copy)
    } else {
        Err(ToxiproxyError::NotFetched(copy))
    }
}

/// A Toxiproxy server on loopback until the value drops.
pub struct Toxiproxy {
    api: SocketAddr,
    process: Process,
    output: PathBuf,
    agent: ureq::Agent,
}

impl std::fmt::Debug for Toxiproxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Toxiproxy")
            .field("api", &self.api)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

impl Toxiproxy {
    /// Starts the tests' copy of the pinned build ([`toxiproxy_copy`]).
    ///
    /// # Errors
    ///
    /// When the copy is missing, or as [`Toxiproxy::start_program`].
    pub fn start(env: &TestEnv) -> Result<Self, ToxiproxyError> {
        Self::start_program(env, &toxiproxy_copy()?)
    }

    /// Starts the Toxiproxy server at `program` on a free loopback port, and
    /// returns once its API answers.
    ///
    /// # Errors
    ///
    /// When the program cannot start, exits before its API answers on each
    /// of its tries, or does not answer within [`TOXIPROXY_DEADLINE`].
    pub fn start_program(env: &TestEnv, program: &Path) -> Result<Self, ToxiproxyError> {
        let output = env
            .folder()
            .join(format!("{}.log", env.unique_name("toxiproxy")));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .timeout_global(Some(TOXIPROXY_DEADLINE))
            .build()
            .into();
        let mut tries = 0;
        loop {
            tries += 1;
            let api = free_loopback_port()?;
            let process = spawn(env, program, api, &output)?;
            let toxiproxy = Self {
                api,
                process,
                output: output.clone(),
                agent: agent.clone(),
            };
            match toxiproxy.wait_until_it_answers() {
                Err(ToxiproxyError::Exited { .. }) if tries < STARTS => {}
                answered => return answered.map(|()| toxiproxy),
            }
        }
    }

    /// The address of its HTTP API.
    pub fn api(&self) -> SocketAddr {
        self.api
    }

    /// Its process, which the test's record holds too.
    pub fn process(&self) -> &Process {
        &self.process
    }

    /// Creates the proxy `name` in front of `upstream`, listening on a free
    /// port of the loopback address `listen`, and returns that address.
    ///
    /// # Errors
    ///
    /// When Toxiproxy refuses it, such as a name already taken or an address
    /// the host does not have.
    pub fn proxy(
        &self,
        name: &str,
        upstream: SocketAddr,
        listen: IpAddr,
    ) -> Result<SocketAddr, ToxiproxyError> {
        let created = self.call(
            "POST",
            "/proxies",
            Some(json!({
                "name": name,
                "listen": SocketAddr::new(listen, 0).to_string(),
                "upstream": upstream.to_string(),
                "enabled": true,
            })),
        )?;
        created["listen"]
            .as_str()
            .and_then(|address| address.parse().ok())
            .ok_or_else(|| ToxiproxyError::Answer {
                call: format!("POST /proxies ({name})"),
                answer: created.to_string(),
            })
    }

    /// Cuts the proxy `name` off: it stops listening, so a connection is
    /// refused, and closes the connections it held.
    ///
    /// # Errors
    ///
    /// When Toxiproxy refuses the call, as it does for an unknown name.
    pub fn cut(&self, name: &str) -> Result<(), ToxiproxyError> {
        self.enable(name, false)
    }

    /// Delays everything the proxy `name` sends back by `delay`, in whole
    /// milliseconds rounded up.
    ///
    /// # Errors
    ///
    /// When Toxiproxy refuses the call, as it does for an unknown name or a
    /// latency already set.
    pub fn latency(&self, name: &str, delay: Duration) -> Result<(), ToxiproxyError> {
        let milliseconds = u64::try_from(delay.as_micros().div_ceil(1000)).unwrap_or(u64::MAX);
        self.add_toxic(
            name,
            json!({
                "name": LATENCY,
                "type": "latency",
                "stream": "downstream",
                "toxicity": 1.0,
                "attributes": { "latency": milliseconds, "jitter": 0 },
            }),
        )
    }

    /// Partitions the proxy `name`: connections still open, but no data
    /// passes either way until it heals.
    ///
    /// # Errors
    ///
    /// When Toxiproxy refuses the call, as it does for an unknown name or a
    /// partition already made.
    pub fn partition(&self, name: &str) -> Result<(), ToxiproxyError> {
        for (toxic, stream) in PARTITION {
            // A timeout of 0 drops the data and never closes the connection.
            self.add_toxic(
                name,
                json!({
                    "name": toxic,
                    "type": "timeout",
                    "stream": stream,
                    "toxicity": 1.0,
                    "attributes": { "timeout": 0 },
                }),
            )?;
        }
        Ok(())
    }

    /// Heals the proxy `name`: removes every toxic, its latency and
    /// partition among them, and listens again on the same address if it was
    /// cut.
    ///
    /// # Errors
    ///
    /// When Toxiproxy refuses the call, as it does for an unknown name.
    pub fn heal(&self, name: &str) -> Result<(), ToxiproxyError> {
        let path = format!("/proxies/{name}/toxics");
        let toxics = self.call("GET", &path, None)?;
        let names = toxics
            .as_array()
            .map(|toxics| {
                toxics
                    .iter()
                    .filter_map(|toxic| toxic["name"].as_str())
                    .collect::<Vec<_>>()
            })
            .ok_or_else(|| ToxiproxyError::Answer {
                call: format!("GET {path}"),
                answer: toxics.to_string(),
            })?;
        for toxic in names {
            self.call("DELETE", &format!("{path}/{toxic}"), None)?;
        }
        self.enable(name, true)
    }

    /// Makes the proxy `name` listen, or stop listening.
    fn enable(&self, name: &str, enabled: bool) -> Result<(), ToxiproxyError> {
        self.call(
            "POST",
            &format!("/proxies/{name}"),
            Some(json!({ "enabled": enabled })),
        )
        .map(drop)
    }

    /// Adds a toxic to the proxy `name`.
    fn add_toxic(&self, name: &str, toxic: Value) -> Result<(), ToxiproxyError> {
        self.call("POST", &format!("/proxies/{name}/toxics"), Some(toxic))
            .map(drop)
    }

    /// Asks `GET /version` until it answers, the process exits or the
    /// deadline passes.
    fn wait_until_it_answers(&self) -> Result<(), ToxiproxyError> {
        let started = Instant::now();
        let url = format!("http://{}/version", self.api);
        loop {
            if self
                .agent
                .get(&url)
                .call()
                .is_ok_and(|answer| answer.status().is_success())
            {
                return Ok(());
            }
            if let Some(status) = self
                .process
                .child()
                .try_wait()
                .map_err(ToxiproxyError::Wait)?
            {
                return Err(ToxiproxyError::Exited {
                    status,
                    output: self.output_text(),
                });
            }
            if started.elapsed() >= TOXIPROXY_DEADLINE {
                return Err(ToxiproxyError::NoAnswer {
                    api: self.api,
                    output: self.output_text(),
                });
            }
            std::thread::sleep(POLL);
        }
    }

    /// Sends one call to the API and returns its JSON answer, `null` for an
    /// answer with no body.
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, ToxiproxyError> {
        let url = format!("http://{}{path}", self.api);
        let call = format!("{method} {path}");
        let sent = match (method, body) {
            ("GET", _) => self.agent.get(&url).call(),
            ("DELETE", _) => self.agent.delete(&url).call(),
            (_, Some(body)) => self.agent.post(&url).send_json(body),
            (_, None) => self.agent.post(&url).send_empty(),
        };
        let mut answer = sent.map_err(|source| ToxiproxyError::Call {
            call: call.clone(),
            source: Box::new(source),
        })?;
        let status = answer.status().as_u16();
        let text = answer
            .body_mut()
            .read_to_string()
            .map_err(|source| ToxiproxyError::Call {
                call: call.clone(),
                source: Box::new(source),
            })?;
        if !answer.status().is_success() {
            return Err(ToxiproxyError::Refused {
                call,
                status,
                answer: text,
            });
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| ToxiproxyError::Answer { call, answer: text })
    }

    /// What Toxiproxy wrote so far, for an error.
    fn output_text(&self) -> String {
        std::fs::read_to_string(&self.output).unwrap_or_default()
    }
}

impl Drop for Toxiproxy {
    fn drop(&mut self) {
        // Best effort: one that already exited, or cannot be killed, is
        // reported by the test's teardown.
        let _ = self.process.stop();
    }
}

/// A loopback port no program listens on now.
fn free_loopback_port() -> Result<SocketAddr, ToxiproxyError> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map_err(ToxiproxyError::Port)
}

/// Starts Toxiproxy with its API on `api`, writing to `output`.
fn spawn(
    env: &TestEnv,
    program: &Path,
    api: SocketAddr,
    output: &Path,
) -> Result<Process, ToxiproxyError> {
    let started = File::options()
        .create(true)
        .append(true)
        .open(output)
        .and_then(|out| {
            let err = out.try_clone()?;
            env.spawn_with_streams(
                env.process_command(program)
                    .arg("-host")
                    .arg(api.ip().to_string())
                    .arg("-port")
                    .arg(api.port().to_string())
                    .arg("-seed")
                    .arg(env.seed().to_string())
                    .stdin(Stdio::null())
                    .stdout(out)
                    .stderr(err),
            )
        });
    started.map_err(|source| ToxiproxyError::Start {
        program: program.to_owned(),
        source,
    })
}

/// Why Toxiproxy cannot start or act.
#[derive(Debug, thiserror::Error)]
pub enum ToxiproxyError {
    /// This platform has no pinned Toxiproxy build.
    #[error("Toxiproxy has no pinned build for this platform")]
    NoBuild,
    /// The tests' copy of the pinned build is missing: run
    /// `ci/toxiproxy-fetch.sh` (part of `just fetch`).
    #[error("Toxiproxy is not at {0}: run `ci/toxiproxy-fetch.sh` (part of `just fetch`)")]
    NotFetched(PathBuf),
    /// No free loopback port could be found for its API.
    #[error("no free loopback port for Toxiproxy's API: {0}")]
    Port(#[source] io::Error),
    /// The program could not start.
    #[error("{} cannot start: {source}", program.display())]
    Start {
        /// The program.
        program: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// Whether it exited could not be read.
    #[error("whether Toxiproxy exited cannot be read: {0}")]
    Wait(#[source] io::Error),
    /// It exited before its API answered, on each try.
    #[error("Toxiproxy exited ({status}) before its API answered; it wrote:\n{output}")]
    Exited {
        /// How it exited.
        status: ExitStatus,
        /// What it wrote.
        output: String,
    },
    /// Its API did not answer within [`TOXIPROXY_DEADLINE`].
    #[error("Toxiproxy's API at {api} did not answer within the deadline; it wrote:\n{output}")]
    NoAnswer {
        /// The API's address.
        api: SocketAddr,
        /// What it wrote.
        output: String,
    },
    /// A call to the API failed before any answer.
    #[error("Toxiproxy's {call} failed: {source}")]
    Call {
        /// The call, as `<method> <path>`.
        call: String,
        /// Why.
        source: Box<ureq::Error>,
    },
    /// The API refused a call.
    #[error("Toxiproxy refused {call} with {status}: {answer}")]
    Refused {
        /// The call, as `<method> <path>`.
        call: String,
        /// The answer's status.
        status: u16,
        /// The answer's body.
        answer: String,
    },
    /// The API answered something other than what the call expects.
    #[error("Toxiproxy answered {call} with {answer}")]
    Answer {
        /// The call, as `<method> <path>`.
        call: String,
        /// The answer's body.
        answer: String,
    },
}
