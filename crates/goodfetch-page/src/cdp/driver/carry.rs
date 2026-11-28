//! What a Session carries from a browser being recycled to the one that
//! replaces it (ADR 0040: a Session is not its browser): the cookies of its
//! browser context and the `localStorage` of every origin its pages'
//! documents came from. Each origin's storage is read, and written again,
//! in a page of the Session's context that commits an empty document of
//! that origin, answered by the engine itself through `Fetch`, so no request
//! reaches the site; the script runs in the guard's isolated world, never
//! through `Runtime.enable` (ADR 0019), and the page closes once done.
//!
//! Size: one concern — all CDP cookie and Web Storage access on a Session
//! browser context and its pages (carry-over capture/restore plus the
//! SDK-only cookie and `localStorage`/`sessionStorage` operations of ADR
//! 0092).
#![expect(
    clippy::disallowed_types,
    reason = "a Session's origins are added and listed one at a time, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use data_encoding::BASE64;
use goodfetch_cdp::generated::{browser, fetch, network, page, storage, target};
use goodfetch_cdp::{Failure, SessionId as TargetSession};
use goodfetch_cdp::{RawEvent, Subscription};
use goodfetch_engine::CallError;
use goodfetch_protocol::page::storage::{
    Cookie, CookieSameSite, NewCookie, PageStorageResult, WebStorageAction,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::{Browser, Connect, GUARD_WORLD, PageDriver, SessionDriver, world};
use crate::errors::Call;

/// The origins whose documents a Session's pages loaded, which its pages'
/// watchers add to.
#[derive(Clone, Debug, Default)]
pub(crate) struct Origins(Arc<Mutex<Watched>>);

#[derive(Debug, Default)]
struct Watched {
    origins: BTreeSet<String>,
    /// How to ask each watcher still going whether its page's reader read
    /// what the connection handed it: [`Origins::caught_up`].
    watchers: Vec<mpsc::Sender<oneshot::Sender<()>>>,
}

/// What a page's reader adds the origins of its documents with, and answers
/// a capture's ask with once it read every event the connection handed it
/// (`page_reader`): a page's origins watcher.
#[derive(Debug)]
pub(crate) struct Watcher {
    origins: Origins,
    asked: mpsc::Receiver<oneshot::Sender<()>>,
}

impl Watcher {
    /// Adds the origin of the document `event` shows a frame of the page
    /// navigated to, when it does.
    pub(crate) fn read(&self, event: &RawEvent) {
        let Some(navigated) = event.read::<page::FrameNavigatedEvent>() else {
            return;
        };
        let origin = navigated.frame.security_origin;
        if origin.starts_with("http://") || origin.starts_with("https://") {
            self.origins.watched().origins.insert(origin);
        }
    }

    /// Waits for a capture's ask, and answers it: its reader asks only
    /// once it holds no event unread. Never answers once no capture can ask.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no ask.
    pub(crate) async fn asked(&mut self) {
        match self.asked.recv().await {
            Some(answer) => {
                let _ = answer.send(());
            }
            None => std::future::pending().await,
        }
    }
}

/// How many asks a watcher holds unread: its Session captures one at a time.
const WATCHER_ASKS: usize = 1;

impl Origins {
    fn watched(&self) -> MutexGuard<'_, Watched> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn list(&self) -> Vec<String> {
        self.watched().origins.iter().cloned().collect()
    }

    /// Adds `origins` that a Session already carried from run state, so a
    /// later browser recycle still captures them even before a page visits
    /// them again.
    pub(crate) fn seed(&self, origins: impl IntoIterator<Item = String>) {
        let mut watched = self.watched();
        for origin in origins {
            if origin.starts_with("http://") || origin.starts_with("https://") {
                watched.origins.insert(origin);
            }
        }
    }

    /// Waits until each page's reader read every event the connection
    /// handed it so far, so an origin the browser reported before an answer
    /// the caller already holds is listed, however far the reader's task
    /// lagged. A watcher that went has nothing more to add.
    async fn caught_up(&self) {
        let watchers = self.watched().watchers.clone();
        for watcher in watchers {
            let (answer, answered) = oneshot::channel();
            if watcher.send(answer).await.is_ok() {
                let _ = answered.await;
            }
        }
    }

    /// The watcher a page's reader adds the origin of every document of the
    /// page's session with, until it ends: its main frame's and its
    /// frames'. A capture waits for it from now on, whether or not its
    /// reader started reading.
    pub(crate) fn watcher(self) -> Watcher {
        let (asks, asked) = mpsc::channel(WATCHER_ASKS);
        {
            let mut watched = self.watched();
            watched.watchers.retain(|watcher| !watcher.is_closed());
            watched.watchers.push(asks);
        }
        Watcher {
            origins: self,
            asked,
        }
    }
}

/// A Session's cookies and storage, taken from one browser for the next,
/// imported from a `state_file` / `cookie_header` (ADR 0056), or kept sealed
/// in run state for a resumed Run (ADRs 0040, 0092, 0102).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Carried {
    cookies: Vec<network::CookieParam>,
    /// Each origin's `localStorage` items, in their order.
    storage: Vec<(String, Vec<(String, String)>)>,
    /// Each origin's `IndexedDB` databases (Playwright 1.64 `storageState`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    indexed_db: Vec<(String, Value)>,
    /// Each origin's OPFS files (Playwright 1.64 `storageState`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    opfs: Vec<(String, Value)>,
}

impl Carried {
    /// Builds [`Carried`] from an [`ImportedState`](crate::state_import::ImportedState).
    pub(crate) fn from_imported(imported: crate::state_import::ImportedState) -> Self {
        use goodfetch_protocol::session_export::SameSite;
        let cookies = imported
            .cookies
            .into_iter()
            .map(|cookie| network::CookieParam {
                name: cookie.name,
                value: cookie.value,
                url: None,
                domain: Some(cookie.domain),
                path: Some(cookie.path),
                secure: Some(cookie.secure),
                http_only: Some(cookie.http_only),
                same_site: cookie.same_site.map(|same_site| match same_site {
                    SameSite::Strict => network::CookieSameSite::Strict,
                    SameSite::Lax => network::CookieSameSite::Lax,
                    SameSite::None => network::CookieSameSite::None,
                }),
                expires: cookie.expires,
                priority: None,
                source_scheme: None,
                source_port: None,
                partition_key: None,
            })
            .collect();
        let mut storage = Vec::new();
        let mut indexed_db = Vec::new();
        let mut opfs = Vec::new();
        for origin in imported.origins {
            if !origin.local_storage.is_empty() {
                storage.push((origin.origin.clone(), origin.local_storage));
            }
            if let Some(idb) = origin.indexed_db {
                indexed_db.push((origin.origin.clone(), idb));
            }
            if let Some(files) = origin.opfs {
                opfs.push((origin.origin, files));
            }
        }
        Self {
            cookies,
            storage,
            indexed_db,
            opfs,
        }
    }

    /// Whether no cookies, `localStorage`, `IndexedDB` or OPFS entries are carried.
    pub(crate) fn is_empty(&self) -> bool {
        self.cookies.is_empty()
            && self.storage.is_empty()
            && self.indexed_db.is_empty()
            && self.opfs.is_empty()
    }

    /// Encodes the carried state as JSON bytes for sealing in run state.
    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes carried state opened from run state.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// The origins whose storage entries this state carries.
    pub(crate) fn origins(&self) -> impl Iterator<Item = String> + '_ {
        self.storage
            .iter()
            .map(|(origin, _)| origin.clone())
            .chain(self.indexed_db.iter().map(|(origin, _)| origin.clone()))
            .chain(self.opfs.iter().map(|(origin, _)| origin.clone()))
    }

    /// The carried `localStorage` entries keyed by origin, for merging
    /// updates from a resumed Session's pages.
    pub(crate) fn storage_map(&self) -> BTreeMap<String, Vec<(String, String)>> {
        self.storage.iter().cloned().collect()
    }

    /// Preserves any `indexedDB` or `opfs` entries from `prior` when saving a
    /// Session that was seeded from an import carrying them.
    pub(crate) fn inherit_extra_storage_from(&mut self, prior: &Self) {
        if self.indexed_db.is_empty() && !prior.indexed_db.is_empty() {
            self.indexed_db.clone_from(&prior.indexed_db);
        }
        if self.opfs.is_empty() && !prior.opfs.is_empty() {
            self.opfs.clone_from(&prior.opfs);
        }
    }

    /// Number of stored cookies and origins.
    pub(crate) fn counts(&self) -> (u64, u64) {
        let mut origins: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (origin, _) in &self.storage {
            origins.insert(origin.as_str());
        }
        for (origin, _) in &self.indexed_db {
            origins.insert(origin.as_str());
        }
        for (origin, _) in &self.opfs {
            origins.insert(origin.as_str());
        }
        (self.cookies.len() as u64, origins.len() as u64)
    }

    /// Distinct non-secret domains held by cookies and storage origins.
    pub(crate) fn domains(&self) -> std::collections::BTreeSet<String> {
        let mut domains = std::collections::BTreeSet::new();
        for cookie in &self.cookies {
            if let Some(domain) = &cookie.domain {
                let normalized = domain.trim_start_matches('.').to_ascii_lowercase();
                if !normalized.is_empty() {
                    domains.insert(normalized);
                }
            }
        }
        for origin in self.origins() {
            if let Ok(parsed) = url::Url::parse(&origin)
                && let Some(host) = parsed.host_str()
            {
                domains.insert(host.to_ascii_lowercase());
            }
        }
        domains
    }

    /// Converts this carried state into a `session-export` document (ADR 0015)
    /// carrying optional Saved identity metadata.
    pub(crate) fn to_session_export(
        &self,
        identity: Option<goodfetch_protocol::session_export::ExportedIdentity>,
    ) -> goodfetch_protocol::session_export::SessionExport {
        use goodfetch_protocol::formats::SESSION_EXPORT;
        use goodfetch_protocol::session_export::{
            ExportedCookie, ExportedOrigin, SameSite, SessionExport, SessionValue, StorageItem,
        };
        let cookies = self
            .cookies
            .iter()
            .map(|cookie| ExportedCookie {
                name: cookie.name.clone(),
                value: SessionValue::new(cookie.value.clone()),
                domain: cookie.domain.clone().unwrap_or_default(),
                path: cookie.path.clone().unwrap_or_else(|| "/".to_owned()),
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Unix epoch seconds fit comfortably in i64"
                )]
                expires: cookie.expires.map(|value| value as i64),
                http_only: cookie.http_only.unwrap_or(false),
                secure: cookie.secure.unwrap_or(false),
                same_site: cookie.same_site.and_then(|same_site| match same_site {
                    network::CookieSameSite::Strict => Some(SameSite::Strict),
                    network::CookieSameSite::Lax => Some(SameSite::Lax),
                    network::CookieSameSite::None => Some(SameSite::None),
                    network::CookieSameSite::Unlisted => None,
                }),
            })
            .collect();
        let origins = self
            .storage
            .iter()
            .map(|(origin, items)| ExportedOrigin {
                origin: origin.clone(),
                local_storage: items
                    .iter()
                    .map(|(name, value)| StorageItem {
                        name: name.clone(),
                        value: SessionValue::new(value.clone()),
                    })
                    .collect(),
                session_storage: Vec::new(),
            })
            .collect();
        SessionExport {
            format_version: SESSION_EXPORT.version,
            cookies,
            origins,
            identity,
        }
    }

    /// Serializes this carried state as clear-text Playwright 1.64 `storageState`
    /// JSON (cookies, `localStorage`, `indexedDB`, `opfs`, ADR 0056).
    pub(crate) fn to_storage_state_json(&self) -> Result<String, serde_json::Error> {
        let cookies: Vec<Value> = self
            .cookies
            .iter()
            .map(|cookie| {
                let same_site = match cookie.same_site {
                    Some(network::CookieSameSite::Strict) => "Strict",
                    Some(network::CookieSameSite::None) => "None",
                    Some(network::CookieSameSite::Lax | network::CookieSameSite::Unlisted)
                    | None => "Lax",
                };
                json!({
                    "name": cookie.name,
                    "value": cookie.value,
                    "domain": cookie.domain.as_deref().unwrap_or(""),
                    "path": cookie.path.as_deref().unwrap_or("/"),
                    "expires": cookie.expires.unwrap_or(-1.0),
                    "httpOnly": cookie.http_only.unwrap_or(false),
                    "secure": cookie.secure.unwrap_or(false),
                    "sameSite": same_site,
                })
            })
            .collect();
        let mut by_origin: BTreeMap<String, serde_json::Map<String, Value>> = BTreeMap::new();
        for (origin, items) in &self.storage {
            let entry = by_origin.entry(origin.clone()).or_default();
            entry.insert("origin".to_owned(), json!(origin));
            entry.insert(
                "localStorage".to_owned(),
                Value::Array(
                    items
                        .iter()
                        .map(|(name, value)| json!({ "name": name, "value": value }))
                        .collect(),
                ),
            );
        }
        for (origin, idb) in &self.indexed_db {
            let entry = by_origin.entry(origin.clone()).or_default();
            entry
                .entry("origin".to_owned())
                .or_insert_with(|| json!(origin));
            entry
                .entry("localStorage".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            entry.insert("indexedDB".to_owned(), idb.clone());
        }
        for (origin, opfs) in &self.opfs {
            let entry = by_origin.entry(origin.clone()).or_default();
            entry
                .entry("origin".to_owned())
                .or_insert_with(|| json!(origin));
            entry
                .entry("localStorage".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            entry.insert("opfs".to_owned(), opfs.clone());
        }
        let origins: Vec<Value> = by_origin.into_values().map(Value::Object).collect();
        serde_json::to_string_pretty(&json!({
            "cookies": cookies,
            "origins": origins,
        }))
    }
}

/// The function that answers an origin's `localStorage` as its entries.
const READ_STORAGE: &str = "() => Object.entries(localStorage)";

/// The function that answers the current document's origin and its
/// `localStorage` entries, or `null` when the document has no web origin or
/// denies storage access.
const READ_PAGE_STORAGE: &str = "() => { try { const o = location.origin; if (!o || o === 'null') return null; return { origin: o, items: Object.entries(localStorage) }; } catch { return null; } }";

/// The function that sets each of its entries in an origin's
/// `localStorage`.
const WRITE_STORAGE: &str = "(items) => { for (const [key, value] of items) localStorage.setItem(key, value); return true; }";

/// The function that restores Playwright 1.64 `indexedDB` databases in an
/// origin's isolated world.
const WRITE_INDEXED_DB: &str = "async (dbs) => { if (!Array.isArray(dbs)) return true; for (const db of dbs) { await new Promise((resolve) => { const req = indexedDB.open(db.name, db.version || 1); req.onupgradeneeded = () => { const conn = req.result; for (const store of db.stores || []) { if (!conn.objectStoreNames.contains(store.name)) { const opts = {}; if (store.keyPath !== undefined && store.keyPath !== null) opts.keyPath = store.keyPath; if (store.autoIncrement) opts.autoIncrement = true; const os = conn.createObjectStore(store.name, opts); for (const idx of store.indexes || []) { os.createIndex(idx.name, idx.keyPath, { unique: !!idx.unique, multiEntry: !!idx.multiEntry }); } } } }; req.onsuccess = () => { const conn = req.result; const names = (db.stores || []).map((s) => s.name).filter((n) => conn.objectStoreNames.contains(n)); if (!names.length) { conn.close(); resolve(true); return; } const tx = conn.transaction(names, 'readwrite'); for (const store of db.stores || []) { if (!conn.objectStoreNames.contains(store.name)) continue; const os = tx.objectStore(store.name); for (const rec of store.records || []) { if (rec.key !== undefined && os.keyPath === null) os.put(rec.value, rec.key); else os.put(rec.value); } } tx.oncomplete = () => { conn.close(); resolve(true); }; tx.onerror = () => { conn.close(); resolve(false); }; }; req.onerror = () => resolve(false); }); } return true; }";

/// The function that restores Playwright 1.64 `opfs` files in an origin's
/// isolated world.
const WRITE_OPFS: &str = "async (files) => { if (!Array.isArray(files) || !navigator.storage?.getDirectory) return true; const root = await navigator.storage.getDirectory(); for (const file of files) { const parts = String(file.path || '').split('/').filter(Boolean); if (!parts.length) continue; let dir = root; for (let i = 0; i < parts.length - 1; i++) dir = await dir.getDirectoryHandle(parts[i], { create: true }); const fh = await dir.getFileHandle(parts[parts.length - 1], { create: true }); const writable = await fh.createWritable(); const bin = atob(file.contentBase64 || ''); const bytes = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i); await writable.write(bytes); await writable.close(); } return true; }";

/// The empty document an origin's page commits.
const EMPTY_DOCUMENT: &str = "<!doctype html><title></title>";

/// The command that sets `cookie` again where it was.
fn cookie_param(cookie: network::Cookie) -> network::CookieParam {
    network::CookieParam {
        name: cookie.name,
        value: cookie.value,
        url: None,
        domain: Some(cookie.domain),
        path: Some(cookie.path),
        secure: Some(cookie.secure),
        http_only: Some(cookie.http_only),
        same_site: cookie.same_site,
        expires: (!cookie.session).then_some(cookie.expires),
        priority: Some(cookie.priority),
        source_scheme: Some(cookie.source_scheme),
        source_port: Some(cookie.source_port),
        partition_key: cookie.partition_key,
    }
}

impl<B: Connect> SessionDriver<B> {
    /// The Session's cookies and the `localStorage` of each of `origins`,
    /// as its browser context holds them now; nothing when it has none.
    pub(crate) async fn capture(&self, origins: &Origins) -> Result<Carried, CallError> {
        let Some(context) = self.context.get() else {
            return Ok(Carried::default());
        };
        let browser = self.browser.connect().await?;
        let call = browser.opening();
        let cookies = browser
            .send(
                None,
                &storage::GetCookiesParams {
                    browser_context_id: Some(context.clone()),
                },
                &call,
            )
            .await?
            .cookies
            .into_iter()
            .map(cookie_param)
            .collect();
        // The browser's answer came after every commit it reported before it,
        // on the one connection: its watchers have those events to read.
        origins.caught_up().await;
        let mut stored = Vec::new();
        for origin in origins.list() {
            let read = on_origin(&browser, context, &origin, READ_STORAGE, Value::Null).await?;
            let items: Vec<(String, String)> = serde_json::from_value(read).unwrap_or_default();
            if !items.is_empty() {
                stored.push((origin, items));
            }
        }
        Ok(Carried {
            cookies,
            storage: stored,
            indexed_db: Vec::new(),
            opfs: Vec::new(),
        })
    }

    /// Puts `carried` into the Session's browser context, made first when
    /// it has none.
    pub(crate) async fn restore(&self, carried: &Carried) -> Result<(), CallError> {
        let browser = self.browser.connect().await?;
        let context = self.context_in(&browser).await?;
        let call = browser.opening();
        if !carried.cookies.is_empty() {
            browser
                .send(
                    None,
                    &storage::SetCookiesParams {
                        cookies: carried.cookies.clone(),
                        browser_context_id: Some(context.clone()),
                    },
                    &call,
                )
                .await?;
        }
        for (origin, items) in &carried.storage {
            on_origin(&browser, &context, origin, WRITE_STORAGE, json!(items)).await?;
        }
        for (origin, dbs) in &carried.indexed_db {
            on_origin(&browser, &context, origin, WRITE_INDEXED_DB, dbs.clone()).await?;
        }
        for (origin, files) in &carried.opfs {
            on_origin(&browser, &context, origin, WRITE_OPFS, files.clone()).await?;
        }
        Ok(())
    }

    /// Reads every cookie currently held by the Session's browser context
    /// (ADR 0092; ticket #746), returning an empty list if no context exists
    /// yet.
    pub(crate) async fn get_cookies(&self) -> Result<Vec<Cookie>, CallError> {
        let Some(context) = self.context.get() else {
            return Ok(Vec::new());
        };
        let browser = self.browser.connect().await?;
        let call = browser.opening();
        let cookies = browser
            .send(
                None,
                &storage::GetCookiesParams {
                    browser_context_id: Some(context.clone()),
                },
                &call,
            )
            .await?
            .cookies;
        Ok(cookies.into_iter().map(protocol_cookie).collect())
    }

    /// Adds `cookies` into the Session's browser context, creating the context
    /// first if needed (ADR 0092; ticket #746).
    pub(crate) async fn add_cookies(&self, cookies: Vec<NewCookie>) -> Result<(), CallError> {
        if cookies.is_empty() {
            return Ok(());
        }
        let browser = self.browser.connect().await?;
        let context = self.context_in(&browser).await?;
        let call = browser.opening();
        let params: Vec<network::CookieParam> = cookies.into_iter().map(new_cookie_param).collect();
        browser
            .send(
                None,
                &storage::SetCookiesParams {
                    cookies: params,
                    browser_context_id: Some(context),
                },
                &call,
            )
            .await?;
        Ok(())
    }

    /// Clears cookies from the Session's browser context (ADR 0092; #746).
    ///
    /// When `clear_all` is `true`, calls `Storage.clearCookies` on the whole
    /// context. Otherwise, deletes only the cookies for which `should_delete`
    /// returns `true` using `Network.deleteCookies`, leaving other cookies
    /// untouched (essential for domain-bound borrowed browsers).
    pub(crate) async fn clear_cookies<F>(
        &self,
        clear_all: bool,
        should_delete: F,
    ) -> Result<(), CallError>
    where
        F: Fn(&Cookie) -> bool,
    {
        let Some(context) = self.context.get() else {
            return Ok(());
        };
        let browser = self.browser.connect().await?;
        let call = browser.opening();
        if clear_all {
            browser
                .send(
                    None,
                    &storage::ClearCookiesParams {
                        browser_context_id: Some(context.clone()),
                    },
                    &call,
                )
                .await?;
            return Ok(());
        }
        let matching: Vec<network::Cookie> = browser
            .send(
                None,
                &storage::GetCookiesParams {
                    browser_context_id: Some(context.clone()),
                },
                &call,
            )
            .await?
            .cookies
            .into_iter()
            .filter(|raw| should_delete(&protocol_cookie(raw.clone())))
            .collect();
        if matching.is_empty() {
            return Ok(());
        }
        let created = browser
            .send(
                None,
                &target::CreateTargetParams {
                    url: "about:blank".to_owned(),
                    browser_context_id: Some(context.clone()),
                    left: None,
                    top: None,
                    width: None,
                    height: None,
                    window_state: None,
                    enable_begin_frame_control: None,
                    new_window: None,
                    background: Some(true),
                    for_tab: None,
                    hidden: None,
                    focus: None,
                },
                &call,
            )
            .await?;
        let delete_result = async {
            let session = browser.attach_own(&created.target_id, &call).await?;
            for cookie in matching {
                browser
                    .send(
                        Some(&session),
                        &network::DeleteCookiesParams {
                            name: cookie.name,
                            url: None,
                            domain: Some(cookie.domain),
                            path: Some(cookie.path),
                            partition_key: cookie.partition_key,
                        },
                        &call,
                    )
                    .await?;
            }
            Ok::<(), CallError>(())
        }
        .await;
        let _ = browser
            .try_send(
                None,
                &target::CloseTargetParams {
                    target_id: created.target_id,
                },
            )
            .await;
        delete_result
    }
}

/// Converts a CDP `network::Cookie` into the protocol's [`Cookie`] carrying
/// Playwright's fields with an RFC 3339 UTC expiry (`None` for session cookies).
fn protocol_cookie(cookie: network::Cookie) -> Cookie {
    let expires = if cookie.session || cookie.expires <= 0.0 || !cookie.expires.is_finite() {
        None
    } else {
        Duration::try_from_secs_f64(cookie.expires)
            .ok()
            .and_then(|duration| UNIX_EPOCH.checked_add(duration))
    };
    let same_site = match cookie.same_site {
        Some(network::CookieSameSite::Strict) => CookieSameSite::Strict,
        Some(network::CookieSameSite::None) => CookieSameSite::None,
        Some(network::CookieSameSite::Lax | network::CookieSameSite::Unlisted) | None => {
            CookieSameSite::Lax
        }
    };
    Cookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        path: cookie.path,
        expires,
        http_only: cookie.http_only,
        secure: cookie.secure,
        same_site,
    }
}

/// Converts a protocol [`NewCookie`] into a CDP `network::CookieParam`.
fn new_cookie_param(cookie: NewCookie) -> network::CookieParam {
    let expires = cookie.expires.and_then(|at| {
        at.duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs_f64())
    });
    let same_site = cookie.same_site.map(|same_site| match same_site {
        CookieSameSite::Strict => network::CookieSameSite::Strict,
        CookieSameSite::Lax => network::CookieSameSite::Lax,
        CookieSameSite::None => network::CookieSameSite::None,
    });
    network::CookieParam {
        name: cookie.name,
        value: cookie.value,
        url: cookie.url,
        domain: cookie.domain,
        path: cookie.path,
        secure: cookie.secure,
        http_only: cookie.http_only,
        same_site,
        expires,
        priority: None,
        source_scheme: None,
        source_port: None,
        partition_key: None,
    }
}

impl PageDriver {
    /// Captures the Session's cookies from its browser context and refreshes
    /// this page's main-frame origin in `stored`, returning what the Session
    /// would carry on a resume. Runs in the guard's isolated world, never
    /// through `Runtime.enable` (ADR 0019), and opens no extra target.
    pub(crate) async fn capture_page_state(
        &self,
        stored: &mut BTreeMap<String, Vec<(String, String)>>,
    ) -> Result<Carried, CallError> {
        let call = self.browser.opening();
        let cookies = self
            .browser
            .send(
                None,
                &storage::GetCookiesParams {
                    browser_context_id: Some(self.context.clone()),
                },
                &call,
            )
            .await?
            .cookies
            .into_iter()
            .map(cookie_param)
            .collect();
        match self.read_origin_storage(&call).await {
            Ok(Some((origin, items))) => {
                if items.is_empty() {
                    stored.remove(&origin);
                } else {
                    stored.insert(origin, items);
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(%error, "could not read page localStorage for run state");
            }
        }
        Ok(Carried {
            cookies,
            storage: stored
                .iter()
                .map(|(origin, items)| (origin.clone(), items.clone()))
                .collect(),
            indexed_db: Vec::new(),
            opfs: Vec::new(),
        })
    }

    /// Reads the current main frame's origin and `localStorage` entries in
    /// the guard's isolated world, answering `None` for non-web origins.
    async fn read_origin_storage(
        &self,
        call: &Call,
    ) -> Result<Option<(String, Vec<(String, String)>)>, CallError> {
        #[derive(Deserialize)]
        struct OriginStorage {
            origin: String,
            items: Vec<(String, String)>,
        }
        let world = self
            .browser
            .send(
                Some(&self.session),
                &isolated_world(self.frame.clone()),
                call,
            )
            .await?;
        let called = self
            .browser
            .send(
                Some(&self.session),
                &world::call(READ_PAGE_STORAGE, Value::Null, world.execution_context_id),
                call,
            )
            .await?;
        let Some(value) = called.result.value else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let Ok(parsed) = serde_json::from_value::<OriginStorage>(value) else {
            return Ok(None);
        };
        if !(parsed.origin.starts_with("http://") || parsed.origin.starts_with("https://")) {
            return Ok(None);
        }
        Ok(Some((parsed.origin, parsed.items)))
    }

    /// Reads the current main frame's URL in the guard's isolated world,
    /// without enabling `Runtime` (ADR 0019).
    pub(crate) async fn current_location_href(&self) -> Result<String, CallError> {
        let call = self.browser.opening();
        let world = self
            .browser
            .send(
                Some(&self.session),
                &isolated_world(self.frame.clone()),
                &call,
            )
            .await?;
        let called = self
            .browser
            .send(
                Some(&self.session),
                &world::call(
                    "() => window.location.href",
                    Value::Null,
                    world.execution_context_id,
                ),
                &call,
            )
            .await?;
        Ok(called
            .result
            .value
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default())
    }

    /// Runs a `WebStorage` action on `window.localStorage` (`is_local: true`)
    /// or `window.sessionStorage` (`is_local: false`) in the guard's isolated
    /// world, without enabling `Runtime` (ADRs 0019 and 0092; ticket #746).
    pub(crate) async fn web_storage(
        &self,
        is_local: bool,
        action: &WebStorageAction,
    ) -> Result<PageStorageResult, CallError> {
        const WEB_STORAGE_OP: &str = r#"(arg) => {
            const store = arg.isLocal ? window.localStorage : window.sessionStorage;
            switch (arg.op) {
                case "items": {
                    const out = {};
                    for (let i = 0; i < store.length; i += 1) {
                        const key = store.key(i);
                        if (key !== null) out[key] = store.getItem(key) ?? "";
                    }
                    return { kind: "items", items: out };
                }
                case "get_item":
                    return { kind: "item", value: store.getItem(arg.key) };
                case "set_item":
                    store.setItem(arg.key, arg.value);
                    return { kind: "updated" };
                case "remove_item":
                    store.removeItem(arg.key);
                    return { kind: "updated" };
                case "clear":
                    store.clear();
                    return { kind: "updated" };
                default:
                    throw new Error("unknown storage op");
            }
        }"#;
        let payload = match action {
            WebStorageAction::Items => json!({ "isLocal": is_local, "op": "items" }),
            WebStorageAction::GetItem { key } => {
                json!({ "isLocal": is_local, "op": "get_item", "key": key })
            }
            WebStorageAction::SetItem { key, value } => {
                json!({ "isLocal": is_local, "op": "set_item", "key": key, "value": value })
            }
            WebStorageAction::RemoveItem { key } => {
                json!({ "isLocal": is_local, "op": "remove_item", "key": key })
            }
            WebStorageAction::Clear => json!({ "isLocal": is_local, "op": "clear" }),
        };
        let call = self.browser.opening();
        let world = self
            .browser
            .send(
                Some(&self.session),
                &isolated_world(self.frame.clone()),
                &call,
            )
            .await?;
        let called = self
            .browser
            .send(
                Some(&self.session),
                &world::call(WEB_STORAGE_OP, payload, world.execution_context_id),
                &call,
            )
            .await?;
        if let Some(exception) = called.exception_details {
            return Err(CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("web storage access failed: {}", exception.text),
            ));
        }
        let value = called.result.value.unwrap_or(Value::Null);
        serde_json::from_value(value).map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("unexpected web storage result: {error}"),
            )
        })
    }
}

/// Calls `function` with `argument` in the guard's world of a page of
/// `context` that holds an empty document of `origin`, and answers its
/// value; the page closes whatever happens.
async fn on_origin(
    browser: &Browser,
    context: &browser::BrowserContextID,
    origin: &str,
    function: &str,
    argument: Value,
) -> Result<Value, CallError> {
    let call = browser.opening();
    let created = browser
        .send(
            None,
            &target::CreateTargetParams {
                url: "about:blank".to_owned(),
                browser_context_id: Some(context.clone()),
                left: None,
                top: None,
                width: None,
                height: None,
                window_state: None,
                enable_begin_frame_control: None,
                new_window: None,
                background: Some(true),
                for_tab: None,
                hidden: None,
                focus: None,
            },
            &call,
        )
        .await?;
    let answered = run_on(
        browser,
        &created.target_id,
        origin,
        (function, argument),
        &call,
    )
    .await;
    if let Err(failure) = browser
        .try_send(
            None,
            &target::CloseTargetParams {
                target_id: created.target_id,
            },
        )
        .await
    {
        tracing::debug!(%failure, "a Session's storage page was gone already");
    }
    answered
}

/// Attaches to the page `page_target`, commits an empty document of
/// `origin` in it and calls the function there.
async fn run_on(
    browser: &Browser,
    page_target: &target::TargetID,
    origin: &str,
    (function, argument): (&str, Value),
    call: &Call,
) -> Result<Value, CallError> {
    let session = browser.attach_own(page_target, call).await?;
    let mut events = browser.connection.subscribe(Some(&session));
    browser
        .send(
            Some(&session),
            &fetch::EnableParams {
                patterns: Some(vec![fetch::RequestPattern {
                    url_pattern: Some("*".to_owned()),
                    resource_type: None,
                    request_stage: Some(fetch::RequestStage::Request),
                }]),
                handle_auth_requests: None,
            },
            call,
        )
        .await?;
    browser
        .send(Some(&session), &page::EnableParams::default(), call)
        .await?;
    let committed = tokio::time::timeout(
        browser.deadlines.navigation.duration(),
        commit(browser, &session, &mut events, origin, call),
    )
    .await;
    // No later command waits on a subscription nobody reads.
    drop(events);
    match committed {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) | Err(_) => {
            tracing::warn!(%origin, "a Session's storage of this origin is not carried over");
            return Ok(Value::Null);
        }
        Ok(Err(error)) => return Err(error),
    }
    let world = browser
        .send(Some(&session), &isolated_world(page_target.clone()), call)
        .await?;
    let called = browser
        .send(
            Some(&session),
            &world::call(function, argument, world.execution_context_id),
            call,
        )
        .await?;
    Ok(called.result.value.unwrap_or(Value::Null))
}

/// Navigates the page of `session` to `origin`, answering each request it
/// sends with an empty document, until that document commits: `false`
/// when the browser refuses the navigation.
async fn commit(
    browser: &Browser,
    session: &TargetSession,
    events: &mut Subscription,
    origin: &str,
    call: &Call,
) -> Result<bool, CallError> {
    let params = page::NavigateParams {
        url: format!("{origin}/"),
        referrer: None,
        transition_type: None,
        frame_id: None,
        referrer_policy: None,
    };
    let mut navigate = std::pin::pin!(browser.send(Some(session), &params, call));
    let mut navigated = false;
    let mut committed = false;
    let mut reading = true;
    while !(navigated && committed) {
        // Both the answer and the commit are waited for, and a refused
        // navigation ends the wait whatever came before it, so the order
        // the two are polled in cannot change how it ends (#1770).
        tokio::select! {
            answered = &mut navigate, if !navigated => {
                if answered?.error_text.is_some() {
                    return Ok(false);
                }
                navigated = true;
            }
            event = events.next(), if reading => {
                let Some(event) = event else {
                    // A target the browser detached sends nothing more, and
                    // the navigation in flight on it failed with it.
                    if !navigated && super::detached(events) {
                        reading = false;
                        continue;
                    }
                    return Err(browser.failed(call, &Failure::Disconnected).await);
                };
                if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
                    let fulfilled = browser
                        .try_send(
                            Some(session),
                            &fetch::FulfillRequestParams {
                                request_id: paused.request_id,
                                response_code: 200,
                                response_headers: Some(vec![fetch::HeaderEntry {
                                    name: "content-type".to_owned(),
                                    value: "text/html; charset=utf-8".to_owned(),
                                }]),
                                binary_response_headers: None,
                                body: Some(BASE64.encode(EMPTY_DOCUMENT.as_bytes())),
                                response_phrase: None,
                            },
                        )
                        .await;
                    if let Err(failure) = fulfilled {
                        tracing::debug!(%failure, "a storage page's request went before it was answered");
                    }
                } else if let Some(frame) = event.read::<page::FrameNavigatedEvent>()
                    && frame.frame.parent_id.is_none()
                    && frame.frame.security_origin == origin
                {
                    committed = true;
                }
            }
            else => return Err(browser.failed(call, &Failure::Disconnected).await),
        }
    }
    Ok(true)
}

/// The command that makes, or finds by its name, the guard's isolated
/// world in `frame`'s document.
fn isolated_world(frame: page::FrameId) -> page::CreateIsolatedWorldParams {
    page::CreateIsolatedWorldParams {
        frame_id: frame,
        world_name: Some(GUARD_WORLD.to_owned()),
        grant_univeral_access: None,
        content_security_policy: None,
    }
}

#[cfg(test)]
mod tests;
