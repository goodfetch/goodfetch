//! GitHub's public REST interface, read with the job's token when it has one;
//! a test points `url` at a local stand-in (TESTING.md, true externals).

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;

pub(crate) struct Api<'a> {
    /// `https://api.github.com`, or a stand-in's address.
    pub(crate) url: &'a str,
    pub(crate) token: Option<&'a str>,
}

impl Api<'_> {
    /// GETs `path` (from `/`, with its query string) and parses its JSON body.
    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.find(path)?
            .with_context(|| format!("reading {path}: GitHub answered 404"))
    }

    /// As [`Api::get`], but `None` when GitHub answers 404: the document does
    /// not exist, or the token cannot see it.
    pub(crate) fn find<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
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
            Err(error) => Err(error).with_context(|| format!("reading {url}")),
        }
    }
}
