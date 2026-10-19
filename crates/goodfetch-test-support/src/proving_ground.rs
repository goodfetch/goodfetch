//! The Proving ground started for one test (ADRs 0059 and 0063): the
//! commit's `serve` on a seed, on port 0 under the test's environment, read
//! from its first output line into what a test needs to reach it with no
//! setup of its own: its address, version and seed, the `resolve` mapping of
//! every host name it serves to loopback, the file of its certificate
//! authority, and its network fixtures (the forward proxies, the DNS fixture,
//! the watched target and the connection record).
#![expect(
    clippy::disallowed_methods,
    reason = "test support writes the certificate authority's file and stops its process outside any async worker (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::env::TestEnv;
use crate::process::{Process, ReportedAddressError, reported_address_within};
use crate::tools::{Tool, ToolsError, tools};

/// How long `serve` has to print its first line: it makes a leaf
/// certificate for every host name before it does.
pub const PROVING_GROUND_DEADLINE: Duration = Duration::from_secs(30);

/// The file name of the certificate authority, in its instance's folder.
const CERTIFICATE_AUTHORITY_FILE: &str = "certificate-authority.pem";

/// A started Proving ground, as a test and `goodfetch-test-env`'s
/// description see it.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ProvingGroundDescription {
    /// The address `serve` bound and answers on, such as `127.0.0.1:51234`.
    pub address: String,
    /// The Proving ground's version, `<workspace version>+<commit>`.
    pub version: String,
    /// The seed every page and expected result comes from.
    #[schemars(range(max = 9_007_199_254_740_991_u64))]
    pub seed: u64,
    /// Every host name it serves, mapped to the IP of its address, as the
    /// `resolve` setting takes them.
    pub resolve: BTreeMap<String, String>,
    /// The file of the certificate authority its TLS leaves chain to, as
    /// PEM, in the test's folder.
    pub ca: PathBuf,
    /// The addresses of its network fixtures.
    pub fixtures: ProvingGroundFixtures,
}

/// The network fixtures `serve` starts beside its pages.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ProvingGroundFixtures {
    /// Each forward proxy's address, by name: `open`, `refusing_tunnels`,
    /// `down` (nothing listens), and one per proxy kind behind Basic
    /// authentication.
    pub proxies: BTreeMap<String, String>,
    /// The credentials the authenticated proxies accept.
    pub proxy_credentials: ProxyCredentials,
    /// The DNS fixture's UDP address.
    pub dns: String,
    /// The watched target's address, which answers `200 reached`.
    pub target: String,
    /// The URL of the connection record, which lists every connection
    /// accepted and every relay a proxy made.
    pub connections: String,
}

/// A forward proxy's Basic credentials, derived from the seed.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ProxyCredentials {
    /// The username.
    pub username: String,
    /// The password.
    pub password: String,
}

/// `serve`'s first line, the fields test support reads.
#[derive(serde::Deserialize)]
struct ServeLine {
    address: SocketAddr,
    version: String,
    seed: u64,
    hosts: Vec<String>,
    certificate_authority: String,
    proxies: BTreeMap<String, String>,
    proxy_credentials: ProxyCredentials,
    dns: String,
    target: String,
}

/// The commit's Proving ground, serving one seed for one test. Its process
/// joins the test's record, and it is stopped when dropped.
pub struct ProvingGround {
    process: Process,
    address: SocketAddr,
    description: ProvingGroundDescription,
}

impl ProvingGround {
    /// Starts the test tool manifest's Proving ground on `seed`.
    pub fn start(env: &TestEnv, seed: u64) -> Result<Self, ProvingGroundError> {
        let tools = tools()?;
        Self::start_program(
            env,
            tools.path(Tool::ProvingGround)?,
            seed,
            PROVING_GROUND_DEADLINE,
        )
    }

    /// Starts `program` as the Proving ground on `seed` (an old tag's
    /// executable, say), waiting at most `deadline` for its first line.
    pub fn start_program(
        env: &TestEnv,
        program: &Path,
        seed: u64,
        deadline: Duration,
    ) -> Result<Self, ProvingGroundError> {
        let mut command = env.process_command(program);
        command.args([
            "serve",
            "--seed",
            &seed.to_string(),
            "--copy",
            "local",
            "--address",
            "127.0.0.1:0",
            "--network-profile",
            "none",
        ]);
        let process = env.spawn(&mut command).map_err(ProvingGroundError::Start)?;
        // On any failure the process is stopped, so the test's teardown finds
        // none alive.
        let started = Self::read(env, &process, deadline);
        match started {
            Ok((address, description)) => Ok(Self {
                process,
                address,
                description,
            }),
            Err(error) => {
                stop(&process);
                Err(error)
            }
        }
    }

    fn read(
        env: &TestEnv,
        process: &Process,
        deadline: Duration,
    ) -> Result<(SocketAddr, ProvingGroundDescription), ProvingGroundError> {
        let reported = reported_address_within(&mut process.child(), deadline)?;
        let line: ServeLine = serde_json::from_value(reported.line.clone()).map_err(|error| {
            ProvingGroundError::NotServeLine {
                line: reported.line.to_string(),
                reason: error.to_string(),
            }
        })?;
        let folder = env.folder().join(env.unique_name("proving-ground"));
        std::fs::create_dir_all(&folder).map_err(ProvingGroundError::CertificateAuthority)?;
        let ca = folder.join(CERTIFICATE_AUTHORITY_FILE);
        std::fs::write(&ca, &line.certificate_authority)
            .map_err(ProvingGroundError::CertificateAuthority)?;
        let ip = line.address.ip().to_string();
        let description = ProvingGroundDescription {
            address: line.address.to_string(),
            version: line.version,
            seed: line.seed,
            resolve: line
                .hosts
                .into_iter()
                .map(|host| (host, ip.clone()))
                .collect(),
            ca,
            fixtures: ProvingGroundFixtures {
                proxies: line.proxies,
                proxy_credentials: line.proxy_credentials,
                dns: line.dns,
                target: line.target,
                connections: format!("http://{}/observations", line.address),
            },
        };
        Ok((line.address, description))
    }

    /// What a test reads of it, and what `goodfetch-test-env` describes.
    pub fn description(&self) -> &ProvingGroundDescription {
        &self.description
    }

    /// The address it answers on.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The URL of `path` on its host name `host`, with the bound port, as a
    /// tool reaches it through the `resolve` mapping.
    pub fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.address.port())
    }
}

impl Drop for ProvingGround {
    fn drop(&mut self) {
        stop(&self.process);
    }
}

/// Kills a process that may already have exited, and reaps it.
fn stop(process: &Process) {
    let mut child = process.child();
    // Best effort: a process that already exited needs only its wait.
    let _ = child.kill();
    let _ = child.wait();
}

/// Why a Proving ground did not start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProvingGroundError {
    /// The test tool manifest names no Proving ground.
    #[error(transparent)]
    Tools(#[from] ToolsError),
    /// Its process did not start.
    #[error("the Proving ground did not start: {0}")]
    Start(io::Error),
    /// It printed no first line within the deadline, or not `serve`'s.
    #[error("the Proving ground's first line: {0}")]
    Line(#[from] ReportedAddressError),
    /// Its first line lacks a field of `serve`'s.
    #[error("the Proving ground's first line is not serve's ({reason}): {line}")]
    NotServeLine {
        /// The line.
        line: String,
        /// What it lacks.
        reason: String,
    },
    /// Its certificate authority's file cannot be written.
    #[error("the Proving ground's certificate authority cannot be written: {0}")]
    CertificateAuthority(io::Error),
}
