//! A plain HTTP client that keeps cookies and follows redirects the way a
//! browser does, without running a page's scripts: what a test needs to sign
//! in to the portal, page through it and post what a page's script posts.

use serde_json::Value;

use crate::proving_ground::ProvingGround;

/// How many redirects one request follows before it gives up.
const REDIRECTS: usize = 10;

/// A client of one Proving ground, with its own cookie jar.
pub(crate) struct Client<'a> {
    proving_ground: &'a ProvingGround,
    agent: ureq::Agent,
    jar: Vec<Cookie>,
}

/// A cookie the client keeps: its name, value and path.
#[derive(Clone, Debug)]
pub(crate) struct Cookie {
    name: String,
    value: String,
    path: String,
}

/// A response after every redirect: the URL it came from, its status,
/// headers and body.
pub(crate) struct Response {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("the body is UTF-8")
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("the body is JSON")
    }

    /// The value of the header `name`, if the response has it.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A request's method and body.
enum Body<'b> {
    Empty,
    Form(&'b str),
    Json(&'b Value),
}

impl<'a> Client<'a> {
    pub(crate) fn new(proving_ground: &'a ProvingGround) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        Self {
            proving_ground,
            agent,
            jar: Vec::new(),
        }
    }

    pub(crate) fn get(&mut self, url: &str) -> Response {
        self.send("GET", url, &Body::Empty)
    }

    /// Posts an empty body.
    pub(crate) fn post(&mut self, url: &str) -> Response {
        self.send("POST", url, &Body::Empty)
    }

    pub(crate) fn post_form(&mut self, url: &str, form: &str) -> Response {
        self.send("POST", url, &Body::Form(form))
    }

    /// Posts a form without following the redirect it answers with.
    pub(crate) fn post_form_once(&mut self, url: &str, form: &str) -> Response {
        self.once("POST", url, &Body::Form(form))
    }

    pub(crate) fn post_json(&mut self, url: &str, value: &Value) -> Response {
        self.send("POST", url, &Body::Json(value))
    }

    /// The value of the cookie `name`, if the client keeps one.
    pub(crate) fn cookie(&self, name: &str) -> Option<&str> {
        self.jar
            .iter()
            .find(|cookie| cookie.name == name)
            .map(|cookie| cookie.value.as_str())
    }

    /// Every cookie the client keeps, as a resumed client imports them.
    pub(crate) fn cookies(&self) -> Vec<Cookie> {
        self.jar.clone()
    }

    pub(crate) fn import(&mut self, cookies: Vec<Cookie>) {
        self.jar = cookies;
    }

    /// Sends a request, then follows each redirect with a `GET`, keeping the
    /// cookies every response sets.
    fn send(&mut self, method: &str, url: &str, body: &Body<'_>) -> Response {
        let mut url = url.to_owned();
        let mut method = method;
        let mut body = body;
        for _ in 0..=REDIRECTS {
            let response = self.once(method, &url, body);
            let location = response
                .header("location")
                .filter(|_| (300..400).contains(&response.status))
                .map(|location| absolute(&url, location));
            match location {
                Some(location) => {
                    url = location;
                    method = "GET";
                    body = &Body::Empty;
                }
                None => return response,
            }
        }
        panic!("more than {REDIRECTS} redirects from {url}");
    }

    fn once(&mut self, method: &str, url: &str, body: &Body<'_>) -> Response {
        let rest = url.strip_prefix("http://").expect("an http URL");
        let (host, target) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let path = target.split('?').next().unwrap_or_default();
        let address = format!("http://{}{target}", self.proving_ground.address());
        let cookies = self
            .jar
            .iter()
            .filter(|cookie| path.starts_with(&cookie.path))
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ");
        let mut response = match (method, body) {
            ("GET", _) => self
                .agent
                .get(&address)
                .header("host", host)
                .header("cookie", &cookies)
                .call(),
            (_, Body::Empty) => self
                .agent
                .post(&address)
                .header("host", host)
                .header("cookie", &cookies)
                .send_empty(),
            (_, Body::Form(form)) => self
                .agent
                .post(&address)
                .header("host", host)
                .header("cookie", &cookies)
                .content_type("application/x-www-form-urlencoded")
                .send(*form),
            (_, Body::Json(value)) => self
                .agent
                .post(&address)
                .header("host", host)
                .header("cookie", &cookies)
                .send_json(value),
        }
        .expect("the Proving ground answers");
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().expect("a visible header").to_owned(),
                )
            })
            .collect();
        for (_, set) in headers.iter().filter(|(name, _)| name == "set-cookie") {
            self.keep(set);
        }
        Response {
            url: url.to_owned(),
            status: response.status().as_u16(),
            headers,
            body: response.body_mut().read_to_vec().expect("a readable body"),
        }
    }

    /// Keeps the cookie a `Set-Cookie` header sets, replacing one of the
    /// same name and path.
    fn keep(&mut self, set: &str) {
        let mut parts = set.split(';').map(str::trim);
        let (name, value) = parts
            .next()
            .and_then(|pair| pair.split_once('='))
            .expect("a cookie pair");
        let path = parts
            .find_map(|attribute| attribute.strip_prefix("Path="))
            .unwrap_or("/")
            .to_owned();
        self.jar
            .retain(|cookie| !(cookie.name == name && cookie.path == path));
        self.jar.push(Cookie {
            name: name.to_owned(),
            value: value.to_owned(),
            path,
        });
    }
}

/// `location` resolved against `url`: a path on the same origin, or a whole
/// URL.
fn absolute(url: &str, location: &str) -> String {
    if location.starts_with("http://") {
        return location.to_owned();
    }
    let rest = url.strip_prefix("http://").expect("an http URL");
    let origin = &rest[..rest.find('/').unwrap_or(rest.len())];
    format!("http://{origin}{location}")
}
