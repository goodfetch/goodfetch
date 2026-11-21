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
    #[arg(long, default_value = API_URL)]
    pub(crate) api_url: String,
}

/// GitHub's API.
pub(crate) const API_URL: &str = "https://api.github.com";

impl Arguments {
    /// The API, read and written with `token` when there is one.
    pub(crate) fn api<'a>(&'a self, token: Option<&'a str>) -> Api<'a> {
        Api {
            url: &self.api_url,
            token,
        }
    }

    /// The API with `token`, which reading the issues needs: without one, a
    /// private repository answers 404 for every issue.
    pub(crate) fn authenticated<'a>(&'a self, token: Option<&'a str>) -> Result<Api<'a>> {
        if token.is_none() {
            anyhow::bail!(
                "no GitHub token to read the issues of {} with: set GITHUB_TOKEN, or log the GitHub CLI in (`gh auth login`)",
                self.repository
            );
        }
        Ok(self.api(token))
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
        match self.headers(ureq::get(&url)).call() {
            Ok(mut response) => response
                .body_mut()
                .read_json()
                .map(Some)
                .with_context(|| format!("reading {url}")),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {url}")),
        }
    }

    /// The bytes at `path`, at most `maximum`, asked for as
    /// `application/octet-stream`: a release asset's content. GitHub answers
    /// with a redirect to its storage, which ureq follows without the token.
    pub(crate) fn download(&self, path: &str, maximum: u64) -> Result<Vec<u8>> {
        let url = self.url(path);
        self.headers_accepting(ureq::get(&url), "application/octet-stream")
            .call()
            .with_context(|| format!("downloading {url}"))?
            .body_mut()
            .with_config()
            .limit(maximum)
            .read_to_vec()
            .with_context(|| format!("reading {url}"))
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

    /// Deletes the document at `path`, which needs a token allowed to write
    /// there; GitHub answers with no body.
    pub(crate) fn delete(&self, path: &str) -> Result<()> {
        let url = self.url(path);
        self.headers(ureq::delete(&url))
            .call()
            .with_context(|| format!("deleting {url}"))?;
        Ok(())
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
        self.headers(request)
            .send_json(body)
            .with_context(|| format!("writing {url}"))
    }

    /// Every item of the list at `path`, read a page of [`PAGE_SIZE`] at a
    /// time until a short page or the list's `total_count`. A list GitHub
    /// wraps in an object, such as `{"total_count": 2, "jobs": [...]}`, is
    /// read from its `key`.
    pub(crate) fn get_all<T: DeserializeOwned>(&self, path: &str, key: List) -> Result<Vec<T>> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        for page in 1.. {
            let page: serde_json::Value = self.get(&format!(
                "{path}{separator}per_page={PAGE_SIZE}&page={page}"
            ))?;
            let (listed, total) = match key {
                List::Array => (page, None),
                List::In(key) => {
                    let total = page.get("total_count").and_then(serde_json::Value::as_u64);
                    (page.get(key).cloned().unwrap_or_default(), total)
                }
            };
            let listed: Vec<T> = serde_json::from_value(listed)
                .with_context(|| format!("reading the list at {path}"))?;
            let short = listed.len() < PAGE_SIZE;
            items.extend(listed);
            let complete = total
                .is_some_and(|total| u64::try_from(items.len()).is_ok_and(|read| read >= total));
            if short || complete {
                break;
            }
        }
        Ok(items)
    }

    /// The headers of every request: GitHub's media type and API version,
    /// the user agent, and the token when there is one.
    fn headers<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        self.headers_accepting(request, "application/vnd.github+json")
    }

    /// As [`Api::headers`], asking for the media type `accept`.
    fn headers_accepting<B>(
        &self,
        request: ureq::RequestBuilder<B>,
        accept: &str,
    ) -> ureq::RequestBuilder<B> {
        let request = request
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "goodfetch-xtask");
        match self.token {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url.trim_end_matches('/'))
    }
}

/// GitHub's largest page.
const PAGE_SIZE: usize = 100;

/// Where a page of a list holds its items.
#[derive(Clone, Copy)]
pub(crate) enum List {
    /// The page is the array.
    Array,
    /// The page is an object holding the array under this key, beside the
    /// list's `total_count`.
    In(&'static str),
}

/// The HTTP methods that write through GitHub's interface.
#[derive(Clone, Copy)]
pub(crate) enum Write {
    Post,
    Patch,
}
