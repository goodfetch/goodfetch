//! A Proving ground started for one test: `serve` on a seed, its first output
//! line, and a plain HTTP client that maps the Proving ground's host names to
//! the reported address, as the `resolve` setting does (ADR 0059). The client
//! follows no redirect and keeps no cookie, so a test sees each one.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use goodfetch_test_support::{
    LOOPBACK_PORT_0, Observations, ROUTES, ServeLine, UNSHAPED, serve_arguments,
};
use hmac::{Hmac, Mac as _};
use serde_json::{Value, json};
use sha1::Sha1;

/// The TOTP code (RFC 6238: HMAC-SHA-1, 30-second steps, 6 digits) of a
/// base32 secret at `unix_seconds`, as a tool's authenticator computes it,
/// apart from the Proving ground's own and held to the RFC's vectors
/// (`login`'s `the_test_authenticator_gives_rfc_6238s_codes`).
pub(crate) fn totp(secret: &str, unix_seconds: u64) -> String {
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.as_bytes())
        .expect("the secret is base32");
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).expect("HMAC takes any key");
    mac.update(&(unix_seconds / 30).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 0x0f);
    let code =
        u32::from_be_bytes(hash[offset..offset + 4].try_into().expect("four bytes")) & 0x7fff_ffff;
    format!("{:06}", code % 1_000_000)
}

/// [`totp`] of a base32 secret now: the Proving ground checks a code by the
/// system's clock, as a login page does (RFC 6238's section 5.2), and
/// accepts the steps next to the current one, so a step ending between the
/// two reads fails nothing.
pub(crate) fn totp_now(secret: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970")
        .as_secs();
    totp(secret, now)
}

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

/// A response's status, `Content-Type` and body as bytes.
pub(crate) struct RawAnswer {
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Vec<u8>,
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
        Self::start_on(seed, LOOPBACK_PORT_0)
    }

    /// Starts `serve` on the given address.
    pub(crate) fn start_on(seed: u64, address: &str) -> Self {
        Self::start_with(&serve_arguments(seed, address, UNSHAPED))
    }

    /// Starts the Proving ground with `arguments`, `serve`'s from
    /// [`serve_arguments`] and any after them, in an empty environment.
    pub(crate) fn start_with(arguments: &[String]) -> Self {
        Self::start_with_env(arguments, &[])
    }

    /// Starts the Proving ground as [`Self::start_with`] does, in an
    /// environment of `variables` alone, such as the online copy's vendor
    /// secrets.
    pub(crate) fn start_with_env(arguments: &[String], variables: &[(&str, &str)]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
            .args(arguments)
            .env_clear()
            .envs(variables.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the Proving ground starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = String::new();
        stdout.read_line(&mut line).expect("stdout is readable");
        assert!(line.ends_with('\n'), "serve printed no line: {line:?}");
        let address = serde_json::from_str::<ServeLine>(&line)
            .unwrap_or_else(|error| {
                panic!("the line is not the described serve line ({error}): {line}")
            })
            .address
            .to_string();
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

    /// The password the authenticated proxies accept.
    pub(crate) fn proxy_password(&self) -> String {
        self.line_json()["proxy_credentials"]["password"]
            .as_str()
            .expect("the line names the proxy password")
            .to_owned()
    }

    /// The address of the DNS fixture `serve` reported.
    pub(crate) fn dns(&self) -> String {
        self.line_json()["dns"]
            .as_str()
            .expect("the line names the DNS fixture")
            .to_owned()
    }

    /// The address of the UDP fixture `serve` reported.
    pub(crate) fn udp(&self) -> String {
        self.line_json()["udp"]
            .as_str()
            .expect("the line names the UDP fixture")
            .to_owned()
    }

    /// The address of the watched target `serve` reported.
    pub(crate) fn target(&self) -> String {
        self.line_json()["target"]
            .as_str()
            .expect("the line names the target")
            .to_owned()
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

    /// Fetches a URL as [`Self::get`] does, sending `headers` after `Host`.
    pub(crate) fn get_with_headers(&self, url: &str, headers: &[(&str, &str)]) -> Answer {
        let (host, path) = split(url);
        let mut request = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .header("host", host);
        for (name, value) in headers {
            request = request.header(*name, *value);
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

    /// Fetches a URL on one of the Proving ground's host names as `get`
    /// does, keeping the body's bytes as sent and its `Content-Type`, if
    /// any.
    pub(crate) fn get_raw(&self, url: &str) -> RawAnswer {
        let rest = url.strip_prefix("http://").expect("an http URL");
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let mut response = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .header("host", host)
            .call()
            .expect("the Proving ground answers");
        let content_type = response
            .headers()
            .get("content-type")
            .map(|value| value.to_str().expect("an ASCII media type").to_owned());
        RawAnswer {
            status: response.status().as_u16(),
            content_type,
            body: response
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_vec()
                .expect("the body is readable"),
        }
    }

    /// The task list, from the Proving ground's own address.
    pub(crate) fn tasks(&self) -> Value {
        let response = self
            .agent
            .get(format!("http://{}{}", self.address, ROUTES.tasks))
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

    /// The certificate authority the Proving ground made from its seed, as
    /// PEM.
    pub(crate) fn certificate_authority(&self) -> String {
        self.control("/certificate-authority").body
    }

    /// The client certificate the hosts that require mutual TLS accept, and
    /// its private key, each as PEM.
    pub(crate) fn client_identity(&self) -> (String, String) {
        (
            self.control(ROUTES.client_certificate).body,
            self.control(ROUTES.client_key).body,
        )
    }

    /// Every request the Proving ground recorded, with its sums.
    pub(crate) fn observations(&self) -> Value {
        let answer = self.control(ROUTES.observations);
        assert_eq!(answer.status, 200, "{}", answer.body);
        let document = answer.json();
        if let Err(error) = serde_json::from_value::<Observations>(document.clone()) {
            panic!("the observations are not the described format ({error}): {document}");
        }
        document
    }

    /// Puts a JSON body to a path on the Proving ground's own address.
    pub(crate) fn put_control(&self, path: &str, body: &str) -> Answer {
        let response = self
            .agent
            .put(format!("http://{}{path}", self.address))
            .content_type("application/json")
            .send(body)
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// Posts a JSON body to a path on the Proving ground's own address.
    pub(crate) fn post_control(&self, path: &str, body: &Value) -> Answer {
        let response = self
            .agent
            .post(format!("http://{}{path}", self.address))
            .send_json(body)
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// Fetches a path on the Proving ground's own address.
    pub(crate) fn control(&self, path: &str) -> Answer {
        let response = self
            .agent
            .get(format!("http://{}{path}", self.address))
            .call()
            .expect("the Proving ground answers");
        Self::answer(response)
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

    /// Posts JSON to a URL on one of the Proving ground's host names, as a
    /// page's script does.
    pub(crate) fn post_json(&self, url: &str, value: &Value) -> Answer {
        let rest = url.strip_prefix("http://").expect("an http URL");
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let response = self
            .agent
            .post(format!("http://{}{path}", self.address))
            .header("host", host)
            .send_json(value)
            .expect("the Proving ground answers");
        Self::answer(response)
    }

    /// The oracle's verdict on a submitted result, which it must answer
    /// with 200: an error page fails here, naming its status and body,
    /// never later as a wrong outcome.
    pub(crate) fn judge(&self, submission: &Value) -> Value {
        let answer = self.submit(submission);
        assert_eq!(
            answer.status, 200,
            "the oracle judges the submission: {}",
            answer.body
        );
        answer.json()
    }

    /// Submits a result to the oracle, answering whatever it answers, for a
    /// test of a refused submission.
    pub(crate) fn submit(&self, submission: &Value) -> Answer {
        let response = self
            .agent
            .post(format!("http://{}{}", self.address, ROUTES.oracle))
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

/// HTTP's Basic credentials for `username` and `password` (RFC 7617), as a
/// `Proxy-Authorization` value.
pub(crate) fn basic(username: &str, password: &str) -> String {
    format!(
        "Basic {}",
        data_encoding::BASE64.encode(format!("{username}:{password}").as_bytes())
    )
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
