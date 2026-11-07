//! The Proving ground's interface, as its callers read it (ADRs 0032 and
//! 0063): the first line `serve` prints, the paths of its own routes and
//! the status a tool reports to its oracle. The Proving ground writes and
//! reads them through these types; test support compiles this same file
//! (`goodfetch-test-support`'s `proving_ground_format`), since the
//! Proving ground takes no Goodfetch crate in its release build, and
//! `xtask codegen` commits their JSON Schemas and route list for the
//! TypeScript and Python test support. A renamed field fails to compile or
//! to validate instead of failing a test far away at run time.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The paths of the Proving ground's own routes, served on its bound
/// address rather than on a page family's host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Routes {
    /// `GET`: the task list.
    pub tasks: &'static str,
    /// `POST`: judges a tool's result for one task.
    pub oracle: &'static str,
    /// `PUT`: the time step its pages answer at from now on.
    pub time_step: &'static str,
    /// `GET`: every request its page hosts received, and the connection
    /// record.
    pub observations: &'static str,
    /// `GET`: the client certificate its mutual-TLS hosts accept.
    pub client_certificate: &'static str,
    /// `GET`: that certificate's private key.
    pub client_key: &'static str,
}

/// The Proving ground's own routes.
pub const ROUTES: Routes = Routes {
    tasks: "/tasks",
    oracle: "/oracle",
    time_step: "/time-step",
    observations: "/observations",
    client_certificate: "/client-certificate",
    client_key: "/client-key",
};

/// The first line `serve` prints once its listeners are bound: where it
/// answers, what it serves and where its network fixtures listen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServeLine {
    /// The address it answers on, such as `127.0.0.1:51234`.
    #[schemars(with = "String")]
    pub address: SocketAddr,
    /// Its version, `<workspace version>+<commit>`.
    pub version: String,
    /// The seed every page and expected result comes from.
    #[schemars(range(max = 9_007_199_254_740_991_u64))]
    pub seed: u64,
    /// Every host name it serves.
    pub hosts: Vec<String>,
    /// The certificate authority its TLS leaves chain to, as PEM.
    pub certificate_authority: String,
    /// Where its network fixtures listen.
    #[serde(flatten)]
    pub fixtures: FixtureAddresses,
    /// The network profile it serves under and how it shapes traffic.
    pub network: Value,
}

/// Where the network fixtures `serve` starts beside its pages listen, as
/// its first line and `GET /network-fixtures` report them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureAddresses {
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
    /// record lists; empty in an older tag's line, which has none.
    #[serde(default)]
    pub udp: String,
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

/// What a tool said of its own result, as it submits it to the oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ToolStatus {
    /// It claims success.
    Success,
    /// Done, with its success left to a person's review (ADR 0013).
    NeedsReview,
    /// It claims failure.
    Failure,
}
