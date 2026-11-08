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
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::env::TestEnv;
use crate::process::{Process, ReportedAddressError, reported_address_within};
use crate::proving_ground_format::{
    OracleSubmission, ProxyCredentials, ROUTES, ReceivedRequests, ServeLine, ToolStatus,
};
use crate::tools::{Tool, ToolsError, tools};

/// How long `serve` has to print its first line, which it writes once its
/// listeners are bound. Its start takes about 0.3 s in a debug build, 1 s
/// with fourteen started at once on a loaded host, most of it the 250 ms
/// that fill the IPv6 blackhole's queue (it builds no bomb body and signs no
/// leaf certificate before it). The bound only catches a `serve` that hangs:
/// the line is awaited as it is written, never polled, so a wait ends as soon
/// as `serve` is ready, and a hang fails with this error well inside
/// nextest's slow timeout rather than as an unnamed test timeout (ADR 0064).
pub const PROVING_GROUND_DEADLINE: Duration = Duration::from_secs(30);

/// The address a test's Proving ground binds: port 0 of loopback.
pub const LOOPBACK_PORT_0: &str = "127.0.0.1:0";

/// The network profile of a Proving ground that shapes nothing.
pub const UNSHAPED: &str = "none";

/// The arguments of `serve` for a test's Proving ground on `seed`: the
/// local copy, bound to `address`, under the network profile
/// `network_profile`. The Rust tests that start the Proving ground spell its
/// command line through it, so a renamed flag fails there in one place. The
/// TypeScript and Python Journey document tests, `xtask rss` and the scale
/// measurement spell theirs.
pub fn serve_arguments(seed: u64, address: &str, network_profile: &str) -> [String; 9] {
    [
        "serve".to_owned(),
        "--seed".to_owned(),
        seed.to_string(),
        "--copy".to_owned(),
        "local".to_owned(),
        "--address".to_owned(),
        address.to_owned(),
        "--network-profile".to_owned(),
        network_profile.to_owned(),
    ]
}

/// The path of `url`, an absolute URL such as one of the Proving ground's
/// pages, without its query or fragment, `/` for an origin, as the URL
/// standard parses it: what a test compares of a URL the engine answers,
/// whose port the Proving ground chose. `None` when `url` is not absolute.
pub fn path_of(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let path = rest[rest.find(['/', '?', '#']).unwrap_or(rest.len())..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    Some(if path.is_empty() { "/" } else { path })
}

/// How long a Proving ground has to exit once asked to stop: it closes its
/// connections and undoes its network shaping first.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// The file name of the certificate authority, in its instance's folder.
const CERTIFICATE_AUTHORITY_FILE: &str = "certificate-authority.pem";

/// The file name of the client certificate, beside the authority's.
const CLIENT_CERTIFICATE_FILE: &str = "client-certificate.pem";

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
    /// The UDP fixture's address, whose every datagram the connection
    /// record lists.
    pub udp: String,
    /// The URL of the connection record, which lists every connection
    /// accepted and every relay a proxy made.
    pub connections: String,
}

/// The commit's Proving ground, serving one seed for one test. Its process
/// joins the test's record, and it is stopped when dropped: with `SIGINT`,
/// as [`ProvingGround::stop`] does, and killed only when it outlasts the
/// deadline.
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
        command.args(serve_arguments(seed, LOOPBACK_PORT_0, UNSHAPED));
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
                proxies: line.fixtures.proxies,
                proxy_credentials: line.fixtures.proxy_credentials,
                dns: line.fixtures.dns,
                target: line.fixtures.target,
                udp: line.fixtures.udp,
                connections: format!("http://{}{}", line.address, ROUTES.observations),
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

    /// `GET /observations` on its own address: every request its page hosts
    /// received, as the Proving ground saw it, and the connection record.
    pub fn observations(&self) -> Result<serde_json::Value, ProvingGroundError> {
        ureq::get(&self.description.fixtures.connections)
            .call()
            .and_then(|mut response| response.body_mut().read_json())
            .map_err(|error| ProvingGroundError::Observations(error.to_string()))
    }

    /// The source address of each query for `name` its DNS fixture received,
    /// in `observations`, read once from [`observations`](Self::observations),
    /// in arrival order.
    ///
    /// # Panics
    ///
    /// When `observations` hold no `dns_queries` list, or a query no source
    /// address.
    pub fn dns_query_sources(observations: &serde_json::Value, name: &str) -> Vec<IpAddr> {
        observations["dns_queries"]
            .as_array()
            .expect("the observations list the DNS queries")
            .iter()
            .filter(|query| query["name"] == name)
            .map(|query| {
                query["source"]
                    .as_str()
                    .and_then(|source| source.parse::<SocketAddr>().ok())
                    .expect("each DNS query names its source address")
                    .ip()
            })
            .collect()
    }

    /// The requests of its [`observations`](Self::observations), which a
    /// test counts through (see [`ReceivedRequests`]).
    pub fn received_requests(&self) -> Result<ReceivedRequests, ProvingGroundError> {
        self.observations()
            .map(|observations| ReceivedRequests::of(&observations))
    }

    /// `PUT /time-step` on its own address: the time step its pages answer
    /// at from now on, such as the `pacing` family's latency.
    ///
    /// # Errors
    ///
    /// `Oracle` when it gave no answer.
    pub fn set_time_step(&self, step: u64) -> Result<(), ProvingGroundError> {
        ureq::put(format!("http://{}{}", self.address, ROUTES.time_step))
            .send_json(serde_json::json!({ "step": step }))
            .map(drop)
            .map_err(|error| ProvingGroundError::Oracle(error.to_string()))
    }

    /// The client certificate its hosts that require mutual TLS accept,
    /// from `GET /client-certificate` and `GET /client-key`: the
    /// certificate written beside the certificate authority's file, the key
    /// as PEM.
    pub fn client_certificate(&self) -> Result<ClientCertificate, ProvingGroundError> {
        let read = |path: &str| {
            ureq::get(format!("http://{}{path}", self.address))
                .call()
                .and_then(|mut response| response.body_mut().read_to_string())
                .map_err(|error| ProvingGroundError::ClientCertificate(error.to_string()))
        };
        let (certificate, key) = (read(ROUTES.client_certificate)?, read(ROUTES.client_key)?);
        let file = self.description.ca.with_file_name(CLIENT_CERTIFICATE_FILE);
        std::fs::write(&file, certificate)
            .map_err(|error| ProvingGroundError::ClientCertificate(error.to_string()))?;
        Ok(ClientCertificate {
            certificate: file,
            key,
        })
    }

    /// The start URL of the task `id` of its task list, such as
    /// `actionability/2`.
    pub fn start_url(&self, id: &str) -> Result<String, ProvingGroundError> {
        let tasks: serde_json::Value =
            ureq::get(format!("http://{}{}", self.address, ROUTES.tasks))
                .call()
                .and_then(|mut response| response.body_mut().read_json())
                .map_err(|error| ProvingGroundError::Oracle(error.to_string()))?;
        tasks["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == id)
            .and_then(|task| task["start_url"].as_str())
            .map(str::to_owned)
            .ok_or_else(|| ProvingGroundError::Oracle(format!("the task list has no `{id}`")))
    }

    /// The oracle's verdict on the task `id`, which the tool under test
    /// reports as `status`, from the Effects the task's pages recorded: its
    /// `outcome`, such as `verified`, and what it found.
    pub fn judge(
        &self,
        id: &str,
        status: ToolStatus,
    ) -> Result<serde_json::Value, ProvingGroundError> {
        self.judge_records(id, status, &[])
    }

    /// The oracle's verdict on the task `id`, which the tool under test
    /// reports as `status` with the Records `records`, such as the
    /// Markdown of each page of `extraction/7`: its `outcome` and what it
    /// found.
    ///
    /// # Errors
    ///
    /// `Oracle` when it gave no answer.
    pub fn judge_records(
        &self,
        id: &str,
        status: ToolStatus,
        records: &[serde_json::Value],
    ) -> Result<serde_json::Value, ProvingGroundError> {
        ureq::post(format!("http://{}{}", self.address, ROUTES.oracle))
            .send_json(OracleSubmission {
                task: id.to_owned(),
                status,
                records: records.to_vec(),
                files: Vec::new(),
            })
            .and_then(|mut response| response.body_mut().read_json())
            .map_err(|error| ProvingGroundError::Oracle(error.to_string()))
    }
}

impl ProvingGround {
    /// Stops it as Ctrl-C does, with `SIGINT`, so that it ends its
    /// connections and undoes its network shaping, and answers how it
    /// exited once reaped. One that outlasts the deadline is killed. On
    /// Windows, it is killed.
    ///
    /// # Errors
    ///
    /// When the signal, the kill or the wait fails, or when it outlasted
    /// the deadline.
    pub fn stop(self) -> io::Result<std::process::ExitStatus> {
        self.process.interrupt_and_wait(STOP_DEADLINE)
    }
}

impl Drop for ProvingGround {
    fn drop(&mut self) {
        stop(&self.process);
    }
}

/// Stops a process that may already have exited, and reaps it.
fn stop(process: &Process) {
    // Best effort: a Proving ground that fails to stop is killed and reaped
    // all the same, and the test's teardown checks that none is left.
    let _ = process.interrupt_and_wait(STOP_DEADLINE);
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
    /// Its observations could not be read.
    #[error("the Proving ground's observations: {0}")]
    Observations(String),
    /// Its task list or its oracle gave no answer.
    #[error("the Proving ground's oracle: {0}")]
    Oracle(String),
    /// Its client certificate could not be read or written.
    #[error("the Proving ground's client certificate: {0}")]
    ClientCertificate(String),
}

/// The client certificate the Proving ground's hosts that require mutual
/// TLS accept: its file, and its private key as PEM, which a test hands
/// the engine through a Secret reference.
#[derive(Clone, Debug)]
pub struct ClientCertificate {
    /// The certificate's PEM file, in the test's folder.
    pub certificate: PathBuf,
    /// Its PKCS #8 private key, as PEM.
    pub key: String,
}

#[cfg(test)]
mod tests;
