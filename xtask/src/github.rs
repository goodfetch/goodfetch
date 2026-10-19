//! GitHub's public REST interface, read and written with the job's token when
//! it has one; a test points `url` at a local stand-in (TESTING.md, true
//! externals).

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;

/// The arguments of every command that reads or writes GitHub.
#[derive(clap::Args)]
#[group(skip)]
pub(crate) struct Arguments {
    /// The repository, as `owner/name`.
    #[arg(long, default_value = "goodfetch/goodfetch")]
    pub(crate) repository: String,
    /// GitHub's API, or a stand-in's address.
    #[arg(long, default_value = "https://api.github.com")]
    pub(crate) api_url: String,
}

impl Arguments {
    /// The API, read and written with `token` when there is one.
    pub(crate) fn api<'a>(&'a self, token: Option<&'a str>) -> Api<'a> {
        Api {
            url: &self.api_url,
            token,
        }
    }
}

#[derive(Clone, Copy)]
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
        let url = self.url(path);
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

    /// Sends `body` as JSON with `method` to `path`, which needs a token
    /// allowed to write there, and parses GitHub's JSON answer.
    pub(crate) fn send<T: DeserializeOwned>(
        &self,
        method: Write,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        let url = self.url(path);
        self.write(method, &url, body)?
            .body_mut()
            .read_json()
            .with_context(|| format!("reading GitHub's answer from {url}"))
    }

    /// POSTs `body` as JSON to `path`; the token needs the matching write
    /// permission.
    pub(crate) fn post(&self, path: &str, body: &serde_json::Value) -> Result<()> {
        self.write(Write::Post, &self.url(path), body).map(drop)
    }

    fn write(
        &self,
        method: Write,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let request = match method {
            Write::Post => ureq::post(url),
            Write::Patch => ureq::patch(url),
        };
        let mut request = request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "goodfetch-xtask");
        if let Some(token) = self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        request
            .send_json(body)
            .with_context(|| format!("writing {url}"))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url.trim_end_matches('/'))
    }
}

/// The HTTP methods that write through GitHub's interface.
#[derive(Clone, Copy)]
pub(crate) enum Write {
    Post,
    Patch,
}
