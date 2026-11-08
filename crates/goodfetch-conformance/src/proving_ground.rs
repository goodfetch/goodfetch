//! A running Proving ground as the harness reaches it over HTTP: its task
//! list, its pages and its oracle (ADRs 0025 and 0063). It is the one
//! translation point for the Proving ground's HTTP failures (ADR 0127).
//!
//! The local copy serves each page family on its own host name (such as
//! `catalog.test`) and the task list and oracle on any other, all from the
//! one address `serve` reported; every request connects to that address and
//! names the URL's host in its `Host` header, as the `resolve` setting maps
//! a host name to an address (ADR 0059). ureq opens its own sockets, the
//! third-party client exception of ADR 0060.

#[path = "../../../proving-ground/src/interface.rs"]
#[expect(
    dead_code,
    unreachable_pub,
    reason = "the Proving ground's own source, of which the harness reads the observations' requests"
)]
mod interface;

pub(crate) use self::interface::{ObservedRequest, ReceivedRequests};
use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::result_document::{OracleAnswer, ProvingGroundCopy};
use serde::{Deserialize, Serialize};

/// A running Proving ground.
#[derive(Clone, Debug)]
pub struct ProvingGround {
    address: String,
    online: bool,
    agent: ureq::Agent,
}

/// The task list a Proving ground serves (ADR 0063).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskList {
    /// The Proving ground's version and seed.
    pub proving_ground: ProvingGroundCopy,
    /// Its tasks.
    pub tasks: Vec<Task>,
}

/// One Proving ground task.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Task {
    /// `<family>/<ordinal>`.
    pub id: TaskId,
    /// Its page family.
    pub family: String,
    /// What the task asks, such as `crawl` or `extract`.
    pub kind: String,
    /// The URL a tool starts from, with the port `serve` bound.
    pub start_url: String,
    /// What the task asks, in words.
    pub description: String,
    /// Its digest, which ignores the port.
    pub digest: String,
    /// The account it logs in with, when it has one.
    #[serde(default)]
    pub account: Option<TaskAccount>,
}

/// The account a task logs in with.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TaskAccount {
    /// Its username.
    pub username: String,
    /// Its password.
    pub password: String,
}

/// The network fixtures a local copy serves beside its pages, as far as the
/// harness reaches them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct NetworkFixtures {
    /// Each forward proxy's address (`host:port`), by name: `open`,
    /// `refusing_tunnels`, `down`, `socks5` and one per proxy kind.
    pub proxies: std::collections::BTreeMap<String, String>,
    /// The credentials its proxies that ask for them take, when it serves
    /// any.
    #[serde(default)]
    pub proxy_credentials: Option<ProxyCredentials>,
}

/// The username and password a local copy's proxies take.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProxyCredentials {
    /// The username.
    pub username: String,
    /// The password.
    pub password: String,
}

/// A Secret canary the Proving ground planted (ADR 0067): its value, the
/// task whose login flow uses it, and the account that logs in there.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PlantedCanary {
    /// Its ID among the Proving ground's plants.
    pub id: String,
    /// The task of its login flow, such as `canaries/3`.
    pub task: String,
    /// The fake secret.
    pub value: String,
    /// The account its flow logs in with, when it has one.
    #[serde(default)]
    pub account: Option<CanaryAccount>,
}

/// The account a canary's login flow logs in with.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CanaryAccount {
    /// Its username.
    pub username: String,
    /// Its password.
    pub password: String,
}

/// A result submitted to the oracle: the task, the status the tool reported
/// and the Records it found, each in the task's family's own shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Submission<'a, R> {
    /// The task.
    pub task: &'a TaskId,
    /// The tool's status: `success`, `needs-review` or `failure`.
    pub status: &'a str,
    /// The Records.
    pub records: &'a [R],
}

impl ProvingGround {
    /// The local copy `serve` reported at `address` (`host:port`).
    pub fn local(address: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            address: address.to_owned(),
            online: false,
            agent,
        }
    }

    /// The address it answers on (`host:port`), to which a target maps the
    /// host names of its pages (the `resolve` setting, ADR 0059).
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Whether it is the online copy, whose results report and never block
    /// a release; the local copy is not.
    pub fn is_online(&self) -> bool {
        self.online
    }

    /// The task list.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached, answers another status
    /// than 200 or serves no task list.
    pub fn task_list(&self) -> Result<TaskList, ProvingGroundError> {
        let url = format!("http://{}/tasks", self.address);
        let body = Self::answer(&url, self.agent.get(&url).call())?;
        serde_json::from_str(&body).map_err(|source| ProvingGroundError::Unreadable { url, source })
    }

    /// A page's body, for a URL on one of the Proving ground's host names.
    ///
    /// # Errors
    ///
    /// When the URL is not an `http` URL, or the page cannot be fetched or
    /// answers another status than 200.
    pub fn page(&self, url: &str) -> Result<String, ProvingGroundError> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| ProvingGroundError::NotHttp {
                url: url.to_owned(),
            })?;
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let response = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .header("host", host)
            .call();
        Self::answer(url, response)
    }

    /// Its own observations (`GET /observations`): every request its page
    /// hosts received and what their pages' scripts reported.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached, answers another status
    /// than 200 or serves no JSON.
    pub fn observations(&self) -> Result<serde_json::Value, ProvingGroundError> {
        let url = format!("http://{}/observations", self.address);
        let body = Self::answer(&url, self.agent.get(&url).call())?;
        serde_json::from_str(&body).map_err(|source| ProvingGroundError::Unreadable { url, source })
    }

    /// Its seed's certificate authority as PEM (`GET
    /// /certificate-authority`), which signs every host name's leaf: a
    /// client that trusts it reaches the host names over TLS.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached or answers another status
    /// than 200.
    pub fn certificate_authority(&self) -> Result<String, ProvingGroundError> {
        let url = format!("http://{}/certificate-authority", self.address);
        Self::answer(&url, self.agent.get(&url).call())
    }

    /// The requests of its [`observations`](Self::observations).
    ///
    /// # Errors
    ///
    /// As [`observations`](Self::observations), and when they hold no list
    /// of requests the Proving ground's format describes.
    pub(crate) fn received_requests(&self) -> Result<ReceivedRequests, ProvingGroundError> {
        let observations = self.observations()?;
        ReceivedRequests::read(&observations).map_err(|source| ProvingGroundError::Unreadable {
            url: format!("http://{}/observations", self.address),
            source,
        })
    }

    /// Its network fixtures (`GET /network-fixtures`): each forward proxy's
    /// address by name; none for a copy that serves none, such as the
    /// online one.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached, answers another status
    /// than 200 or 404, or serves no JSON.
    pub fn network_fixtures(&self) -> Result<Option<NetworkFixtures>, ProvingGroundError> {
        let url = format!("http://{}/network-fixtures", self.address);
        let body = match Self::answer(&url, self.agent.get(&url).call()) {
            Ok(body) => body,
            Err(ProvingGroundError::Status { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        serde_json::from_str(&body)
            .map(Some)
            .map_err(|source| ProvingGroundError::Unreadable { url, source })
    }

    /// Sets the Proving ground's time step, which its time-dependent pages
    /// follow: the `pacing` family holds each answer longer at each step.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached or refuses the step.
    pub fn set_time_step(&self, step: u64) -> Result<(), ProvingGroundError> {
        let url = format!("http://{}/time-step", self.address);
        Self::answer(
            &url,
            self.agent
                .put(&url)
                .send_json(serde_json::json!({ "step": step })),
        )
        .map(drop)
    }

    /// Turns the `limited` proxy's hold on or off: while it is on, the
    /// proxy holds the first request it counts until it is turned off, so
    /// two requests sent at once meet there however late the second comes.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached or refuses the hold.
    pub fn set_limited_proxy_hold(&self, held: bool) -> Result<(), ProvingGroundError> {
        let url = format!("http://{}/limited-proxy-hold", self.address);
        Self::answer(
            &url,
            self.agent
                .put(&url)
                .send_json(serde_json::json!({ "held": held })),
        )
        .map(drop)
    }

    /// Plants a Secret canary of `kind`, such as `session_cookie`, through
    /// its login flow (`POST /canaries`, ADR 0067): the Proving ground draws
    /// the value and records every request that carries it.
    ///
    /// # Errors
    ///
    /// When the Proving ground cannot be reached, refuses the kind or
    /// answers no plant.
    pub fn plant_canary(&self, kind: &str) -> Result<PlantedCanary, ProvingGroundError> {
        let url = format!("http://{}/canaries", self.address);
        let body = Self::answer(
            &url,
            self.agent
                .post(&url)
                .send_json(serde_json::json!({ "kind": kind })),
        )?;
        serde_json::from_str(&body).map_err(|source| ProvingGroundError::Unreadable { url, source })
    }

    /// The oracle's answer for a submitted result.
    ///
    /// # Errors
    ///
    /// When the oracle cannot be reached, refuses the submission (an
    /// unknown task, a malformed result) or gives no answer.
    pub fn judge<R: Serialize>(
        &self,
        submission: &Submission<'_, R>,
    ) -> Result<OracleAnswer, ProvingGroundError> {
        let url = format!("http://{}/oracle", self.address);
        let body = Self::answer(&url, self.agent.post(&url).send_json(submission))?;
        serde_json::from_str(&body).map_err(|source| ProvingGroundError::Unreadable { url, source })
    }

    fn answer(
        url: &str,
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<String, ProvingGroundError> {
        let mut response = response.map_err(|source| ProvingGroundError::Unreachable {
            url: url.to_owned(),
            source: Box::new(source),
        })?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(ProvingGroundError::Status {
                url: url.to_owned(),
                status,
            });
        }
        response
            .body_mut()
            .read_to_string()
            .map_err(|source| ProvingGroundError::Unreachable {
                url: url.to_owned(),
                source: Box::new(source),
            })
    }
}

/// Why the Proving ground gave no usable answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProvingGroundError {
    /// A URL that is not an `http` one.
    #[error("`{url}` is not an http URL")]
    NotHttp {
        /// The URL.
        url: String,
    },
    /// The request failed before an answer.
    #[error("the Proving ground did not answer {url}: {source}")]
    Unreachable {
        /// The URL.
        url: String,
        /// Why.
        source: Box<ureq::Error>,
    },
    /// It answered another status than 200.
    #[error("the Proving ground answered {url} with status {status}")]
    Status {
        /// The URL.
        url: String,
        /// The status.
        status: u16,
    },
    /// Its answer is not the JSON expected.
    #[error("the Proving ground's answer to {url} cannot be read: {source}")]
    Unreadable {
        /// The URL.
        url: String,
        /// Why.
        source: serde_json::Error,
    },
}
