//! A Proving ground started for one test: `serve` on a seed, its first output
//! line, and a plain HTTP client that maps the Proving ground's host names to
//! the reported address, as the `resolve` setting does (ADR 0059).

use std::io::{BufRead as _, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

use base64::Engine as _;
use serde_json::Value;

/// A running `serve`, stopped when dropped.
pub(crate) struct ProvingGround {
    child: Child,
    /// Kept open so a later write to stdout never fails.
    _stdout: BufReader<ChildStdout>,
    line: String,
    address: String,
    agent: ureq::Agent,
}

/// A response's status and body.
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) body: String,
}

impl Answer {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("the body is JSON")
    }
}

impl ProvingGround {
    /// Starts `serve` on port 0 of the loopback address.
    pub(crate) fn start(seed: u64) -> Self {
        Self::start_on(seed, "127.0.0.1:0")
    }

    /// Starts `serve` on the given address.
    pub(crate) fn start_on(seed: u64, address: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
            .args(["serve", "--seed", &seed.to_string(), "--copy", "local"])
            .args(["--address", address, "--network-profile", "none"])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the Proving ground starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = String::new();
        stdout.read_line(&mut line).expect("stdout is readable");
        assert!(line.ends_with('\n'), "serve printed no line: {line:?}");
        let address = serde_json::from_str::<Value>(&line).expect("the line is JSON")["address"]
            .as_str()
            .expect("the line names an address")
            .to_owned();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            child,
            _stdout: stdout,
            line,
            address,
            agent,
        }
    }

    /// `serve`'s first output line, as printed.
    pub(crate) fn line(&self) -> &str {
        &self.line
    }

    /// The address `serve` reported.
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// `serve`'s first output line, as JSON.
    fn line_json(&self) -> Value {
        serde_json::from_str(&self.line).expect("the line is JSON")
    }

    /// The address of the forward proxy `serve` reported under `name`.
    pub(crate) fn proxy(&self, name: &str) -> String {
        self.line_json()["proxies"][name]
            .as_str()
            .unwrap_or_else(|| panic!("the line names the {name} proxy: {}", self.line))
            .to_owned()
    }

    /// The `Proxy-Authorization` header value an authenticated proxy accepts
    /// for `username` (the reported one, or a sticky form built on it) and
    /// the reported password.
    pub(crate) fn proxy_authorization(&self, username: &str) -> String {
        let password = self.line_json()["proxy_credentials"]["password"]
            .as_str()
            .expect("the line names the proxy password")
            .to_owned();
        basic(username, &password)
    }

    /// The username the authenticated proxies accept.
    pub(crate) fn proxy_username(&self) -> String {
        self.line_json()["proxy_credentials"]["username"]
            .as_str()
            .expect("the line names the proxy username")
            .to_owned()
    }

    /// The address of the DNS fixture `serve` reported.
    pub(crate) fn dns(&self) -> String {
        self.line_json()["dns"]
            .as_str()
            .expect("the line names the DNS fixture")
            .to_owned()
    }

    /// The address of the watched target `serve` reported.
    pub(crate) fn target(&self) -> String {
        self.line_json()["target"]
            .as_str()
            .expect("the line names the target")
            .to_owned()
    }

    /// Sets the Proving ground's time step.
    pub(crate) fn set_time_step(&self, step: u64) -> Answer {
        self.put_json("/time-step", &serde_json::json!({ "step": step }))
    }

    /// Puts a JSON body to a path on the Proving ground's own address.
    pub(crate) fn put_json(&self, path: &str, body: &Value) -> Answer {
        let response = self
            .agent
            .put(format!("http://{}{path}", self.address))
            .send_json(body)
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// The port `serve` bound.
    pub(crate) fn port(&self) -> &str {
        self.address
            .rsplit_once(':')
            .expect("the address has a port")
            .1
    }

    /// Fetches a URL on one of the Proving ground's host names, connecting to
    /// the reported address with the URL's host in the `Host` header.
    pub(crate) fn get(&self, url: &str) -> Answer {
        let rest = url.strip_prefix("http://").expect("an http URL");
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let response = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .header("host", host)
            .call()
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// The task list, from the Proving ground's own address.
    pub(crate) fn tasks(&self) -> Value {
        let response = self
            .agent
            .get(format!("http://{}/tasks", self.address))
            .call()
            .expect("the Proving ground answers");
        let answer = Self::answer(response);
        assert_eq!(answer.status, 200, "{}", answer.body);
        answer.json()
    }

    /// The certificate authority the Proving ground made from its seed, as
    /// PEM.
    pub(crate) fn certificate_authority(&self) -> String {
        self.control("/certificate-authority").body
    }

    /// Every request the Proving ground recorded, with its sums.
    pub(crate) fn observations(&self) -> Value {
        let answer = self.control("/observations");
        assert_eq!(answer.status, 200, "{}", answer.body);
        answer.json()
    }

    /// Fetches a path on the Proving ground's own address.
    fn control(&self, path: &str) -> Answer {
        let response = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .call()
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// Posts a form to a URL on one of the Proving ground's host names.
    pub(crate) fn post_form(&self, url: &str, form: &str) -> Answer {
        let rest = url.strip_prefix("http://").expect("an http URL");
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let response = self
            .agent
            .post(format!("http://{}{path}", self.address))
            .header("host", host)
            .content_type("application/x-www-form-urlencoded")
            .send(form)
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// Submits a result to the oracle.
    pub(crate) fn judge(&self, submission: &Value) -> Answer {
        let response = self
            .agent
            .post(format!("http://{}/oracle", self.address))
            .send_json(submission)
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    fn answer(mut response: ureq::http::Response<ureq::Body>) -> Answer {
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .expect("the body is UTF-8");
        Answer { status, body }
    }
}

/// HTTP's Basic credentials for `username` and `password` (RFC 7617), as a
/// `Proxy-Authorization` value.
pub(crate) fn basic(username: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
    )
}

impl Drop for ProvingGround {
    fn drop(&mut self) {
        // The process may already have exited; either way it is reaped.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
