//! `browser-builds.test` (#3100): the prepared download host of the browser
//! builds a test hands `serve --browser-builds <folder>`, so an engine's
//! first use downloads a pinned build from the Proving ground, never the
//! internet, its `browser.mirror` set to `https://browser-builds.test`.
//! `GET /<path>` answers the file at `<path>` below the folder, read once
//! when `serve` starts; any other request, and every one when `serve` is
//! given no folder, is `404`, so a mirror path the folder lacks is the
//! download host that answers `404`. It is the one host whose bytes no
//! seed generates: the files are the test's own, such as Chrome for
//! Testing's archives as `ci/chrome-for-testing-fetch.sh` lays them out.
#![expect(
    clippy::disallowed_methods,
    reason = "`serve` reads the builds' folder once, on its main thread, before its runtime starts"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use hyper::body::Bytes;
use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};

/// The host name the builds are served on.
pub(crate) const HOST: &str = "browser-builds.test";

/// The files `serve` was given, by the path each is served at.
#[derive(Default)]
pub(crate) struct BrowserBuilds {
    files: BTreeMap<String, Bytes>,
}

impl BrowserBuilds {
    /// Every file below `folder`, each served at its path relative to it,
    /// its components joined by `/`.
    ///
    /// # Errors
    ///
    /// When the folder or one of its files cannot be read.
    pub(crate) fn read(folder: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        let mut folders = vec![(folder.to_path_buf(), String::new())];
        while let Some((current, path)) = folders.pop() {
            let entries = fs::read_dir(&current)
                .with_context(|| format!("reading the builds' folder {}", current.display()))?;
            for entry in entries {
                let entry = entry
                    .with_context(|| format!("reading the builds' folder {}", current.display()))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let served = format!("{path}/{name}");
                if entry.file_type()?.is_dir() {
                    folders.push((entry.path(), served));
                } else {
                    let bytes = fs::read(entry.path())
                        .with_context(|| format!("reading {}", entry.path().display()))?;
                    files.insert(served, Bytes::from(bytes));
                }
            }
        }
        Ok(Self { files })
    }

    /// The reply to `request` on `name`, a host name without its port;
    /// `None` when `name` is another host's.
    pub(crate) fn respond(&self, name: &str, request: &Request<'_>) -> Option<Reply> {
        if name != HOST {
            return None;
        }
        Some(match (request.method, self.files.get(request.path)) {
            (&Method::GET, Some(bytes)) => Reply {
                status: StatusCode::OK,
                content_type: if Path::new(request.path)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                {
                    "application/zip"
                } else {
                    "application/octet-stream"
                },
                headers: Vec::new(),
                body: bytes.clone(),
            },
            _ => Reply::not_found(),
        })
    }
}
