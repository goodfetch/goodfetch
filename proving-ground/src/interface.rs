//! The Proving ground's interface, as its callers read it (ADRs 0032 and
//! 0063): the first line `serve` prints, the paths of its own routes, the
//! status a tool reports to its oracle and the requests `GET /observations`
//! lists, with which of them a case asked for. The Proving ground writes
//! and reads the first three through these types; test support
//! (`goodfetch-test-support`'s `proving_ground_format`) and the conformance
//! harness compile this same file, since the Proving ground takes no
//! Goodfetch crate in its release build, and `xtask codegen` commits JSON
//! Schemas and the route list from it. In the Rust that reads through these
//! types a renamed field fails to compile. The TypeScript and Python test
//! support read only the route list, and one Python test the serve line's
//! and oracle submission's schemas, so a test there that spells a route or
//! reads the serve line itself finds a rename when it runs.

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
    /// `GET <path>/<size>`: the right answer of each page of the article
    /// site of `<size>` pages ([`ArticleAnswers`]).
    pub article_answers: &'static str,
}

/// The Proving ground's own routes.
pub const ROUTES: Routes = Routes {
    tasks: "/tasks",
    oracle: "/oracle",
    time_step: "/time-step",
    observations: "/observations",
    client_certificate: "/client-certificate",
    client_key: "/client-key",
    article_answers: "/article-answers",
};

/// What `GET /article-answers/<size>` serves: the right answer of each page
/// of the article site of `<size>` pages (`article.localhost/sites/<size>/`),
/// which a comparison checks each tool's title and Main content against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArticleAnswers {
    /// Each page's answer, in the pages' order.
    pub pages: Vec<ArticleAnswer>,
    /// The markers of each page's header, nav, aside and footer: text no
    /// Main content holds.
    pub boilerplate: Vec<String>,
    /// The marker of each page's ad, which a Clean-up rule removes.
    pub ad: String,
}

/// One page's right answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArticleAnswer {
    /// The page's path on the site's host.
    pub path: String,
    /// Its title: character references decoded, white space collapsed.
    pub title: String,
    /// The paragraphs of its article, each as its text reads.
    pub paragraphs: Vec<String>,
}

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
    /// record lists.
    pub udp: String,
}

/// A forward proxy's Basic credentials, derived from the seed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// One request a page host received, as `GET /observations` lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservedRequest {
    /// Its place among every request, from 0.
    pub sequence: u64,
    /// The connection it came on.
    pub connection: u64,
    /// The task of the host it came to, if the host has one.
    pub task: Option<String>,
    /// The host name it named, without the port.
    pub host: String,
    /// `http/1.1` or `h2`.
    pub protocol: String,
    /// What it opens: `websocket`, `event_stream` or `request`.
    pub kind: String,
    /// Its method.
    pub method: String,
    /// Its target: the path and query.
    pub target: String,
    /// HTTP/2's pseudo-header names in the order sent.
    pub pseudo_header_order: Option<Vec<String>>,
    /// Every other header, in the order sent, names as sent.
    pub headers: Vec<(String, String)>,
    /// Its cookies, in the order sent.
    pub cookies: Vec<(String, String)>,
    /// Its HTTP Message Signatures headers (RFC 9421), when signed.
    pub message_signature: Option<Value>,
    /// The TLS `ClientHello` of its connection, over TLS.
    pub tls: Option<Value>,
    /// Its HTTP/2 connection's settings and its stream, over HTTP/2.
    pub http2: Option<Value>,
    /// Its answer's status, once the answer's head left.
    pub status: Option<u16>,
    /// When it arrived and was answered.
    pub timing: RequestTiming,
    /// The bytes received and sent for it.
    pub bytes: Value,
    /// The bytes of its bodies alone, received and sent: the same for
    /// every client that sends and receives the same bodies.
    pub body: Value,
    /// The latest earlier request of the same host, method and target
    /// this one repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeats: Option<u64>,
    /// The next request the same host received, and how long after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_request: Option<Value>,
    /// What the raw server sent of the answer, for the hosts it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Value>,
}

impl ObservedRequest {
    /// The value of its first header named `name`, compared without case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// When a request's head arrived and its answer's head left, in
/// microseconds since `serve` started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestTiming {
    /// When its head arrived.
    pub arrived_microseconds: u64,
    /// When its answer's head left; none while it has not.
    pub answered_microseconds: Option<u64>,
}

/// The targets the engine and the browser ask a site for on their own: the
/// robots.txt read of a document's origin (ADR 0086) and Chromium's icon.
pub const OWN_TARGETS: [&str; 2] = ["/robots.txt", "/favicon.ico"];

/// Each request the Proving ground's page hosts received, in arrival order,
/// as `GET /observations` lists them: what a test or a conformance case
/// counts through. A site sees requests no case asked for, which land
/// before or after any count a case takes (ADR 0064): the robots.txt the
/// engine reads of each document's origin, which nothing a case calls
/// waits for (ADR 0086), and the icon Chromium asks for on its own.
/// [`of_the_case`](Self::of_the_case) leaves them out; a claim that covers
/// them reads [`every`](Self::every). `xtask received-requests` refuses a
/// read of the observations' requests anywhere else. A test whose record is
/// too large to read whole counts one host's requests of the case through
/// `GET /requests-of-the-case` instead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReceivedRequests(Vec<ObservedRequest>);

impl ReceivedRequests {
    /// The requests of a Proving ground's `observations`.
    ///
    /// # Errors
    ///
    /// When the observations hold no list of requests, or a request this
    /// format does not describe.
    pub fn read(observations: &Value) -> Result<Self, serde_json::Error> {
        /// The part of the observations that lists the requests.
        #[derive(Deserialize)]
        struct Listed {
            requests: Vec<ObservedRequest>,
        }
        Listed::deserialize(observations).map(|listed| Self(listed.requests))
    }

    /// The requests the case is about: every one but those for
    /// [`OWN_TARGETS`], which the engine and the browser send on their own,
    /// whenever they like.
    pub fn of_the_case(&self) -> impl Iterator<Item = &ObservedRequest> {
        self.0
            .iter()
            .filter(|request| !OWN_TARGETS.contains(&request.target.as_str()))
    }

    /// Every request, the robots.txt reads and icons included, for a claim
    /// about them or about all a site saw, taken at a named state such as
    /// after the robots.txt read.
    pub fn every(&self) -> impl Iterator<Item = &ObservedRequest> {
        self.0.iter()
    }
}

/// What starts every Secret canary value but a TOTP seed's (ADR 0067), the
/// keyword of test support's `gitleaks` rule.
pub const CANARY_MARK: &str = "gfcanary";

/// What starts a TOTP seed canary: a seed is base32, whose alphabet holds
/// `GFCANARY`.
pub const TOTP_CANARY_MARK: &str = "GFCANARYTOTP";

/// The base32 characters a TOTP seed canary draws after its mark: with the
/// mark's 12, 32 characters, 20 bytes, RFC 4226's recommended length for an
/// HMAC-SHA-1 key.
pub const TOTP_CANARY_DRAWN: u64 = 20;

/// RFC 4648's base32 alphabet, from which a TOTP seed canary draws.
pub const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// How a kind of Secret canary spells its value after [`CANARY_MARK`]:
/// `<mark>-<tag>-<hex digits>`. The Proving ground's `canaries` login flows
/// plant the kinds below, and test support plants and searches every kind
/// by the same shapes, so a leak the Proving ground plants matches the
/// search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanaryShape {
    /// The kind's tag, such as `password`.
    pub tag: &'static str,
    /// How many lowercase hex digits follow the tag.
    pub hex_digits: usize,
}

/// A typed password's canary.
pub const TYPED_PASSWORD_CANARY: CanaryShape = CanaryShape {
    tag: "password",
    hex_digits: 16,
};

/// A Session cookie's canary.
pub const SESSION_COOKIE_CANARY: CanaryShape = CanaryShape {
    tag: "session",
    hex_digits: 32,
};

/// An endpoint token's canary.
pub const ENDPOINT_TOKEN_CANARY: CanaryShape = CanaryShape {
    tag: "token",
    hex_digits: 32,
};
