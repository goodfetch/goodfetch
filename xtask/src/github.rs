//! GitHub's REST interface, read with the token `main` was given: one JSON
//! document per request, and a missing one told apart from a failed request.

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;

pub(crate) struct Api<'a> {
    /// `https://api.github.com`, or a local stand-in in tests.
    pub(crate) url: &'a str,
    pub(crate) token: Option<&'a str>,
}

impl Api<'_> {
    /// The document at `path` (from the API's root, query included), or
    /// `None` when GitHub answers 404: it does not exist, or the token
    /// cannot see it.
    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
        let url = format!("{}{path}", self.url.trim_end_matches('/'));
        let mut request = ureq::get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "goodfetch-xtask");
        if let Some(token) = self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        match request.call() {
            Ok(mut response) => response
                .body_mut()
                .read_json()
                .map(Some)
                .with_context(|| format!("reading {url}")),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(error) => Err(error).with_context(|| format!("requesting {url}")),
        }
    }
}
