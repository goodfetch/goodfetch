//! A Proving ground started for one test: `serve` on a seed, its first output
//! line, and a plain HTTP client that maps the Proving ground's host names to
//! the reported address, as the `resolve` setting does (ADR 0059). The client
//! follows no redirect and keeps no cookie, so a test sees each one.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// A running `serve`, stopped when dropped.
pub(crate) struct ProvingGround {
    child: Child,
    /// Kept open so a later write to stdout never fails.
    _stdout: BufReader<ChildStdout>,
    line: String,
    address: String,
    agent: ureq::Agent,
}

/// A response's status, headers and body.
pub(crate) struct Answer {
    pub(crate) status: u16,
    headers: Vec<(String, String)>,
    pub(crate) body: String,
}

impl Answer {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("the body is JSON")
    }

    /// The value of the header `name`, if the response has one.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The `name=value` pair of the cookie `name` the response sets.
    pub(crate) fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .filter(|(header, _)| header.eq_ignore_ascii_case("set-cookie"))
            .filter_map(|(_, value)| value.split(';').next())
            .find(|pair| pair.split_once('=').is_some_and(|(key, _)| key == name))
            .map(str::to_owned)
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
            .max_redirects(0)
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

    /// The port `serve` bound.
    pub(crate) fn port(&self) -> &str {
        self.address
            .rsplit_once(':')
            .expect("the address has a port")
            .1
    }

    /// The URL of `path` on the host name `host`, with the bound port.
    pub(crate) fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port())
    }

    /// Fetches a URL on one of the Proving ground's host names, connecting to
    /// the reported address with the URL's host in the `Host` header.
    pub(crate) fn get(&self, url: &str) -> Answer {
        self.get_with_cookie(url, None)
    }

    /// Fetches a URL as [`Self::get`] does, sending `cookie` (`name=value`
    /// pairs) when given.
    pub(crate) fn get_with_cookie(&self, url: &str, cookie: Option<&str>) -> Answer {
        let (host, path) = split(url);
        let mut request = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .header("host", host);
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        Self::answer(request.call().expect("the Proving ground answers"))
    }

    /// Posts a form to a URL on one of the Proving ground's host names,
    /// sending `cookie` when given.
    pub(crate) fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        cookie: Option<&str>,
    ) -> Answer {
        let (host, path) = split(url);
        let mut request = self
            .agent
            .post(format!("http://{}{path}", self.address))
            .header("host", host);
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        Self::answer(
            request
                .send_form(form.iter().copied())
                .expect("the Proving ground answers"),
        )
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

    /// The task `id` of the task list.
    pub(crate) fn task(&self, id: &str) -> Value {
        let tasks = self.tasks();
        tasks["tasks"]
            .as_array()
            .expect("`tasks` is an array")
            .iter()
            .find(|task| task["id"] == id)
            .unwrap_or_else(|| panic!("the task list has no {id}: {tasks}"))
            .clone()
    }

    /// Sets the Proving ground's time step, on its own address.
    pub(crate) fn set_time_step(&self, step: &Value) -> Answer {
        let response = self
            .agent
            .put(format!("http://{}/time-step", self.address))
            .send_json(json!({ "step": step }))
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
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                let value = value.to_str().expect("a header is text");
                (name.as_str().to_owned(), value.to_owned())
            })
            .collect();
        let body = response
            .body_mut()
            .read_to_string()
            .expect("the body is UTF-8");
        Answer {
            status,
            headers,
            body,
        }
    }
}

/// A result for the oracle: the task, the tool's status and its Records.
pub(crate) fn submission(task: &str, status: &str, records: &[Value]) -> Value {
    json!({ "task": task, "status": status, "records": records })
}

/// The host and path of a URL on one of the Proving ground's host names.
fn split(url: &str) -> (&str, &str) {
    let rest = url.strip_prefix("http://").expect("an http URL");
    rest.split_at(rest.find('/').unwrap_or(rest.len()))
}

impl Drop for ProvingGround {
    fn drop(&mut self) {
        // The process may already have exited; either way it is reaped.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
