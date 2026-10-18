//! GitHub's public REST interface, read and written with the job's token when
//! it has one; a test points `url` at a local stand-in (TESTING.md, true
//! externals).

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;

#[derive(Clone, Copy)]
pub(crate) struct Api<'a> {
    /// `https://api.github.com`, or a stand-in's address.
    pub(crate) url: &'a str,
    pub(crate) token: Option<&'a str>,
}

impl Api<'_> {
    /// GETs `path` (from `/`, with its query string) and parses its JSON body.
    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.url.trim_end_matches('/'));
        let mut request = ureq::get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "goodfetch-xtask");
        if let Some(token) = self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        request
            .call()
            .and_then(|mut response| response.body_mut().read_json())
            .with_context(|| format!("reading {url}"))
    }

    /// POSTs `body` as JSON to `path`; the token needs the matching write
    /// permission.
    pub(crate) fn post(&self, path: &str, body: &serde_json::Value) -> Result<()> {
        let url = format!("{}{path}", self.url.trim_end_matches('/'));
        let mut request = ureq::post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "goodfetch-xtask");
        if let Some(token) = self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        request
            .send_json(body)
            .map(drop)
            .with_context(|| format!("writing {url}"))
    }
}
