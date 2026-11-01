//! A listener's sign-in routes (spec #592), as the Console's page uses
//! them: `POST` the sign-in link's token for a cookie, `GET` the session
//! for the sign-in's CSRF token, and `POST` with that token to sign out.
//! Each request names the listener's own `Host` and `Origin`, so only the
//! rule an attempt breaks is broken.

use std::net::SocketAddr;

use serde_json::Value;

use super::runner::Observed;
use super::table::SignInRoutes;

/// A listener's sign-in routes at `address`, whose cookie is `cookie`.
pub(super) struct SignIns<'a> {
    pub(super) address: SocketAddr,
    pub(super) routes: SignInRoutes,
    pub(super) cookie: &'a str,
}

impl SignIns<'_> {
    /// Swaps `token` for a sign-in: the answer, and the cookie's value
    /// when it set one.
    pub(super) fn sign_in(&self, token: &str) -> (Observed, Option<String>) {
        let request = agent()
            .post(self.url(self.routes.sign_in))
            .header("Origin", self.origin())
            .content_type("text/plain;charset=UTF-8");
        match request.send(token) {
            Ok(response) => {
                let cookie = response
                    .headers()
                    .get_all("set-cookie")
                    .iter()
                    .filter_map(|value| value.to_str().ok())
                    .find_map(|set| self.value_in(set));
                (observed(response), cookie)
            }
            Err(error) => (unreachable(&error), None),
        }
    }

    /// The CSRF token of the sign-in `value`, if the session shows one.
    pub(super) fn csrf_token(&self, value: &str) -> Option<String> {
        let mut response = agent()
            .get(self.url(self.routes.session))
            .header("Cookie", format!("{}={value}", self.cookie))
            .call()
            .ok()?;
        let body: Value = serde_json::from_str(&response.body_mut().read_to_string().ok()?).ok()?;
        body["csrf_token"].as_str().map(str::to_owned)
    }

    /// Signs the sign-in `value` out, with the CSRF token `csrf` if any.
    pub(super) fn sign_out(&self, value: &str, csrf: Option<&str>) -> Observed {
        let mut request = agent()
            .post(self.url(self.routes.sign_out))
            .header("Origin", self.origin())
            .header("Cookie", format!("{}={value}", self.cookie));
        if let Some(csrf) = csrf {
            request = request.header("X-CSRF-Token", csrf);
        }
        match request.send_empty() {
            Ok(response) => observed(response),
            Err(error) => unreachable(&error),
        }
    }

    /// A new sign-in through the link's `token`, signed out at once: its
    /// cookie's value, if both steps were accepted.
    pub(super) fn signed_out(&self, token: &str) -> Option<String> {
        let (_, Some(value)) = self.sign_in(token) else {
            return None;
        };
        let csrf = self.csrf_token(&value);
        matches!(
            self.sign_out(&value, csrf.as_deref()),
            Observed::Answered {
                status: 200..=299,
                ..
            }
        )
        .then_some(value)
    }

    /// `attempts` sign-ins with `token`: the last one's answer.
    pub(super) fn too_often(&self, token: &str, attempts: usize) -> Observed {
        (0..attempts).fold(Observed::Malformed, |_, _| self.sign_in(token).0)
    }

    fn origin(&self) -> String {
        format!("http://{}", self.address)
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// The cookie's value a `Set-Cookie` header sets, if it sets this one.
    fn value_in(&self, set: &str) -> Option<String> {
        let pair = set.split(';').next()?;
        let (name, value) = pair.trim().split_once('=')?;
        (name == self.cookie && !value.is_empty()).then(|| value.to_owned())
    }
}

/// An HTTP agent that reads every status as an answer.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn observed(mut response: ureq::http::Response<ureq::Body>) -> Observed {
    let status = response.status().as_u16();
    let body_length = response
        .body_mut()
        .read_to_vec()
        .map_or(0, |body| body.len());
    Observed::Answered {
        status,
        body_length,
    }
}

fn unreachable(error: &ureq::Error) -> Observed {
    let error = match error {
        ureq::Error::Io(error) => error.kind(),
        _ => std::io::ErrorKind::Other,
    };
    Observed::Unreachable { error }
}
