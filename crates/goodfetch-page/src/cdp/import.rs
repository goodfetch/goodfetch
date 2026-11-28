//! Chromium-family browser import over CDP (`installed_browser` and
//! `profile_directory` imports, ADR 0056; ticket #738).
//!
//! Reads cookies (`Storage.getCookies`), `localStorage` (`DOMStorage`), and
//! `IndexedDB` (`IndexedDB`) by launching the person's own Chromium-family
//! browser (Chrome, Edge, Brave, or Chromium) on a closed, non-default Profile
//! directory over its debugging pipe (`--remote-debugging-pipe`), never
//! decrypting browser cookie files directly and never loading any site page.
//!
//! Probes `Network.enableDeviceBoundSessions` to detect and exclude refreshed
//! cookies belonging to Device Bound Session Credentials (DBSC), warning that
//! such a login works only in its own browser.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use goodfetch_cdp::generated::{
    browser, dom_storage, indexed_db, network, runtime, storage, target,
};
use goodfetch_cdp::{Connection, Failure, SessionId as TargetSession, Subscription};
use goodfetch_counted_work::CountedWork;
use goodfetch_engine::CallError;
use goodfetch_engine::secrets;
use goodfetch_engine::tasks::TaskStarter;
use goodfetch_protocol::browser_cache::{
    BROWSER_NOT_INSTALLED, BrowserNotInstalledDetails, MissingBrowser,
};
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::page::{BROWSER_LAUNCH_FAILED, BrowserLaunchFailedDetails};
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::{
    DEVICE_BOUND_SESSION_EXCLUDED, DEVICE_BOUND_SESSION_PROBE_UNAVAILABLE,
    IMPORTED_IDENTITY_FAMILY_DIFFERS,
};
use goodfetch_protocol::session_export::SameSite;
use goodfetch_protocol::settings::session_source::BrowserName;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SESSION_SOURCE, SettingsLayer,
};
use serde_json::{Value, json};
use tokio::process::Command;

use super::launch::{self, Launcher, Process};
use crate::profile_directory;
use crate::state_import::{ImportedCookie, ImportedOrigin, ImportedState, filter_domains};

/// Timeout for individual CDP calls while reading an installed browser profile.
const CDP_TIMEOUT: Duration = Duration::from_secs(15);
/// Timeout when closing the short-lived reader browser cleanly.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Page size for `IndexedDB.requestData` pagination.
const IDB_PAGE_SIZE: i64 = 100;
/// Maximum records read per `IndexedDB` object store to bound memory.
const IDB_MAX_RECORDS: usize = 10_000;

/// Request to read login state from an installed Chromium-family browser or
/// closed Profile directory over CDP.
#[derive(Clone, Debug)]
pub(crate) struct ChromiumImportRequest {
    pub(crate) browser: BrowserName,
    pub(crate) path: Option<PathBuf>,
    pub(crate) chromium_profile: Option<String>,
    pub(crate) executable: Option<PathBuf>,
    pub(crate) domains: Vec<String>,
    pub(crate) layer: SettingsLayer,
}

/// A structured non-fatal warning raised during a browser import (ADR 0056).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportWarning {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ImportWarning {
    /// Converts this import warning into a Run report warning bound to `session`.
    pub(crate) fn to_run_warning(&self, session: &SessionId) -> RunWarning {
        RunWarning {
            code: self.code.to_owned(),
            message: self.message.clone(),
            sessions: vec![session.0.clone()],
            folder: None,
        }
    }
}

/// Everything learned from reading a Chromium-family browser over CDP.
#[derive(Clone, Debug)]
pub(crate) struct ChromiumImportOutcome {
    pub(crate) state: ImportedState,
    pub(crate) source_family: &'static str,
    pub(crate) locale: Option<String>,
    pub(crate) timezone_id: Option<String>,
    pub(crate) warnings: Vec<ImportWarning>,
}

/// Returns the canonical browser family label for `browser`.
pub(crate) const fn browser_family_label(browser: BrowserName) -> &'static str {
    match browser {
        BrowserName::Chrome => "chrome",
        BrowserName::Edge => "edge",
        BrowserName::Brave => "brave",
        BrowserName::Chromium => "chromium",
        BrowserName::Firefox => "firefox",
        BrowserName::Safari => "safari",
    }
}

/// Returns the running browser family label for a Goodfetch-launched Chromium
/// build (Chrome for Testing on Linux x64 / macOS / Windows is `"chrome"`;
/// distribution Chromium on Linux musl / arm64 is `"chromium"`).
pub(crate) const fn default_running_browser_family() -> &'static str {
    if cfg!(all(
        target_os = "linux",
        any(target_env = "musl", target_arch = "aarch64")
    )) {
        "chromium"
    } else {
        "chrome"
    }
}

/// Returns an [`ImportWarning`] when the running browser family differs from
/// the source browser family that minted the cookies (ADR 0056).
pub(crate) fn family_mismatch_warning(
    source_family: &str,
    running_family: &str,
) -> Option<ImportWarning> {
    if source_family.eq_ignore_ascii_case(running_family)
        || source_family.eq_ignore_ascii_case("chromium")
    {
        return None;
    }
    Some(ImportWarning {
        code: IMPORTED_IDENTITY_FAMILY_DIFFERS,
        message: format!(
            "The running browser family (`{running_family}`) differs from the source browser (`{source_family}`) that minted these cookies; the Session presents `{running_family}`'s identity."
        ),
    })
}

/// Launches the person's own Chromium-family browser on a closed, non-default
/// Profile directory over its debugging pipe, reads cookies, `localStorage`,
/// and `IndexedDB` over CDP (excluding device-bound session cookies), and
/// closes the browser cleanly.
pub(crate) async fn read_chromium_browser(
    request: ChromiumImportRequest,
    launcher: &Launcher,
    owner: &TaskStarter,
    work: CountedWork,
    host_zone: &str,
) -> Result<ChromiumImportOutcome, CallError> {
    if !matches!(
        request.browser,
        BrowserName::Chrome | BrowserName::Edge | BrowserName::Brave | BrowserName::Chromium
    ) {
        let name = browser_family_label(request.browser);
        return Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            format!(
                "This engine imports Chromium-family browsers (`chrome`, `edge`, `brave`, `chromium`) so far, not `{name}`."
            ),
            &CapabilityUnavailableDetails {
                key: SESSION_SOURCE.name.to_owned(),
                layer: request.layer,
                kind: "installed_browser".to_owned(),
                fix: "`state_file`".to_owned(),
            },
        ));
    }

    let prepared = prepare_source(&request, launcher, owner).await?;
    let reader_owner = owner.child();

    let (connection, process): (Connection, Option<Process>) =
        match (launcher, &prepared.executable) {
            (Launcher::Scripted(scripted), None) => {
                let (connection, driver) = launch::start_scripted_pipe(scripted, work).await?;
                reader_owner.spawn(async move {
                    let _ = driver.await;
                });
                (connection, None)
            }
            (_, Some(executable)) => {
                let command = build_reader_command(
                    executable,
                    &prepared.data_dir,
                    request.chromium_profile.as_deref(),
                );
                let (connection, driver, process) =
                    launch::start_pipe(command, work).await.map_err(|error| {
                        CallError::new(
                            &BROWSER_LAUNCH_FAILED,
                            format!(
                                "could not launch `{}` to read profile `{}` over a pipe: {error}",
                                executable.display(),
                                prepared.data_dir.display()
                            ),
                            &BrowserLaunchFailedDetails {
                                executable: executable.display().to_string(),
                                exit: None,
                            },
                        )
                    })?;
                reader_owner.spawn(async move {
                    let _ = driver.await;
                });
                (connection, Some(process))
            }
            (Launcher::Executable, None) => {
                return Err(missing_browser_error(request.browser, &[]));
            }
        };

    let read_result = read_over_cdp(&connection, &request.domains).await;

    if let Some(process) = process {
        process.close_gracefully(&connection, CLOSE_TIMEOUT).await;
    } else {
        let _ = connection
            .send_raw("Browser.close", None, json!({}), CLOSE_TIMEOUT)
            .await;
    }
    drop(connection);
    reader_owner.stop().await;

    let (state, warnings) = read_result?;
    let source_family = browser_family_label(request.browser);

    Ok(ChromiumImportOutcome {
        state,
        source_family,
        locale: prepared.locale,
        timezone_id: Some(host_zone.to_owned()),
        warnings,
    })
}

struct PreparedSource {
    data_dir: PathBuf,
    executable: Option<PathBuf>,
    locale: Option<String>,
}

async fn prepare_source(
    request: &ChromiumImportRequest,
    launcher: &Launcher,
    owner: &TaskStarter,
) -> Result<PreparedSource, CallError> {
    let browser = request.browser;
    let explicit_dir = request.path.clone();
    let chromium_profile = request.chromium_profile.clone();
    let explicit_exe = request.executable.clone();
    let layer = request.layer;
    let is_scripted = matches!(launcher, Launcher::Scripted(_));

    owner
        .blocking(move || {
            let data_dir = match explicit_dir {
                Some(path) => path,
                None => profile_directory::default_user_data_dir(browser).ok_or_else(|| {
                    CallError::new(
                        &BROWSER_NOT_INSTALLED,
                        format!(
                            "Could not determine a default user data directory for `{}`.",
                            browser_family_label(browser)
                        ),
                        &BrowserNotInstalledDetails {
                            missing: MissingBrowser::Path {
                                path: String::new(),
                            },
                            fix: "Pass a non-default Profile directory `path` or use `running_browser`."
                                .to_owned(),
                        },
                    )
                })?,
            };
            let _validated = profile_directory::inspect(
                &data_dir,
                browser,
                chromium_profile.as_deref(),
                None,
                None,
                layer,
            )?;
            let locale =
                profile_directory::read_profile_locale(&data_dir, chromium_profile.as_deref());

            let executable = match explicit_exe {
                Some(path) => {
                    if !path.is_file() {
                        return Err(missing_browser_error(browser, &[path]));
                    }
                    Some(path)
                }
                None if is_scripted => None,
                None => {
                    let candidates = candidate_executables(browser);
                    let found = candidates.iter().find(|path| path.is_file()).cloned();
                    match found {
                        Some(path) => Some(path),
                        None => return Err(missing_browser_error(browser, &candidates)),
                    }
                }
            };

            Ok(PreparedSource {
                data_dir,
                executable,
                locale,
            })
        })
        .await
}

fn build_reader_command(
    executable: &Path,
    data_dir: &Path,
    chromium_profile: Option<&str>,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--remote-debugging-pipe")
        .arg(format!("--user-data-dir={}", data_dir.display()))
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-component-update")
        .arg("--disable-sync")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(profile) = chromium_profile {
        command.arg(format!("--profile-directory={profile}"));
    }
    command
}

async fn read_over_cdp(
    connection: &Connection,
    domains: &[String],
) -> Result<(ImportedState, Vec<ImportWarning>), CallError> {
    let mut warnings = Vec::new();
    let _ = connection
        .send(None, &browser::GetVersionParams {}, CDP_TIMEOUT)
        .await;

    let (target_session, created_target) = attach_reader_target(connection).await?;

    let dbsc_sessions = if let Some(session) = &target_session {
        probe_device_bound_sessions(connection, session, &mut warnings).await
    } else {
        Vec::new()
    };

    let raw_cookies = fetch_cookies(connection, target_session.as_ref()).await?;
    let (cookies, cookie_ports, excluded_count) =
        filter_device_bound_cookies(raw_cookies, &dbsc_sessions, domains);

    if !dbsc_sessions.is_empty() || excluded_count > 0 {
        let sites: BTreeSet<&str> = dbsc_sessions
            .iter()
            .map(|session| session.key.site.as_str())
            .collect();
        let sites_text = if sites.is_empty() {
            "device-bound sessions".to_owned()
        } else {
            sites.into_iter().collect::<Vec<_>>().join(", ")
        };
        warnings.push(ImportWarning {
            code: DEVICE_BOUND_SESSION_EXCLUDED,
            message: format!(
                "Left out {excluded_count} cookie(s) refreshed by device-bound sessions ({sites_text}); such a login works only in its own browser (borrow it with `running_browser` or use a non-default `profile_directory` in place)."
            ),
        });
    }

    let candidate_origins = collect_candidate_origins(&cookies, &cookie_ports, domains);
    let origins = if let Some(session) = &target_session {
        read_origin_storage(connection, session, &candidate_origins).await
    } else {
        Vec::new()
    };

    if let Some(target_id) = created_target {
        let _ = connection
            .send(None, &target::CloseTargetParams { target_id }, CDP_TIMEOUT)
            .await;
    }

    let state = filter_domains(ImportedState { cookies, origins }, domains);
    Ok((state, warnings))
}

async fn attach_reader_target(
    connection: &Connection,
) -> Result<(Option<TargetSession>, Option<target::TargetID>), CallError> {
    let created = connection
        .send(
            None,
            &target::CreateTargetParams {
                url: "about:blank".to_owned(),
                browser_context_id: None,
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
            CDP_TIMEOUT,
        )
        .await;
    let Ok(created) = created else {
        return Ok((None, None));
    };
    let attached = connection
        .send(
            None,
            &target::AttachToTargetParams {
                target_id: created.target_id.clone(),
                flatten: Some(true),
            },
            CDP_TIMEOUT,
        )
        .await;
    if let Ok(attached) = attached {
        Ok((
            Some(TargetSession(attached.session_id)),
            Some(created.target_id),
        ))
    } else {
        let _ = connection
            .send(
                None,
                &target::CloseTargetParams {
                    target_id: created.target_id,
                },
                CDP_TIMEOUT,
            )
            .await;
        Ok((None, None))
    }
}

async fn probe_device_bound_sessions(
    connection: &Connection,
    session: &TargetSession,
    warnings: &mut Vec<ImportWarning>,
) -> Vec<network::DeviceBoundSession> {
    let mut events = connection.subscribe(Some(session));
    let enabled = connection
        .send(
            Some(session),
            &network::EnableDeviceBoundSessionsParams { enable: true },
            CDP_TIMEOUT,
        )
        .await;
    match enabled {
        Ok(_) => {
            // Round-trip barrier so any events emitted synchronously with
            // `Network.enableDeviceBoundSessions` are queued on `events`.
            let _ = connection
                .send(None, &browser::GetVersionParams {}, CDP_TIMEOUT)
                .await;
            drain_device_bound_sessions(&mut events).await
        }
        Err(Failure::Refused { .. }) => {
            warnings.push(ImportWarning {
                code: DEVICE_BOUND_SESSION_PROBE_UNAVAILABLE,
                message: "The source browser does not support `Network.enableDeviceBoundSessions`, so device-bound sessions could not be checked; the import continued.".to_owned(),
            });
            Vec::new()
        }
        Err(_) => Vec::new(),
    }
}

async fn drain_device_bound_sessions(
    events: &mut Subscription,
) -> Vec<network::DeviceBoundSession> {
    let mut sessions = Vec::new();
    loop {
        let next = tokio::select! {
            biased;
            event = events.next() => event,
            () = std::future::ready(()) => break,
        };
        let Some(event) = next else {
            break;
        };
        if let Some(added) = event.read::<network::DeviceBoundSessionsAddedEvent>() {
            sessions.extend(added.sessions);
        }
    }
    sessions
}

async fn fetch_cookies(
    connection: &Connection,
    target_session: Option<&TargetSession>,
) -> Result<Vec<network::Cookie>, CallError> {
    let browser_res = connection
        .send(
            None,
            &storage::GetCookiesParams {
                browser_context_id: None,
            },
            CDP_TIMEOUT,
        )
        .await;
    match browser_res {
        Ok(res) => Ok(res.cookies),
        Err(first_err) => {
            if let Some(session) = target_session
                && let Ok(res) = connection
                    .send(
                        Some(session),
                        &storage::GetCookiesParams {
                            browser_context_id: None,
                        },
                        CDP_TIMEOUT,
                    )
                    .await
            {
                return Ok(res.cookies);
            }
            Err(CallError::new(
                &BROWSER_LAUNCH_FAILED,
                format!("could not read cookies over CDP from source browser: {first_err}"),
                &BrowserLaunchFailedDetails {
                    executable: String::new(),
                    exit: None,
                },
            ))
        }
    }
}

fn filter_device_bound_cookies(
    cookies: Vec<network::Cookie>,
    sessions: &[network::DeviceBoundSession],
    domains: &[String],
) -> (Vec<ImportedCookie>, Vec<(String, i64, bool)>, usize) {
    let mut kept = Vec::new();
    let mut ports = Vec::new();
    let mut excluded = 0_usize;
    for cookie in cookies {
        let host = cookie.domain.trim_start_matches('.');
        if !domains.is_empty() && !secrets::allowed(host, domains) {
            continue;
        }
        if is_device_bound_cookie(&cookie, sessions) {
            excluded += 1;
            continue;
        }
        if cookie.source_port > 0 && cookie.source_port != 80 && cookie.source_port != 443 {
            ports.push((
                host.to_ascii_lowercase(),
                cookie.source_port,
                cookie.secure
                    || matches!(cookie.source_scheme, network::CookieSourceScheme::Secure),
            ));
        }
        kept.push(cdp_cookie_to_imported(cookie));
    }
    (kept, ports, excluded)
}

fn is_device_bound_cookie(
    cookie: &network::Cookie,
    sessions: &[network::DeviceBoundSession],
) -> bool {
    let cookie_domain = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
    for session in sessions {
        for craving in &session.cookie_cravings {
            if craving.name != cookie.name {
                continue;
            }
            let craving_domain = craving.domain.trim_start_matches('.').to_ascii_lowercase();
            let domain_matches = if craving_domain.is_empty() {
                let site_host = url::Url::parse(&session.key.site)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
                    .unwrap_or_else(|| session.key.site.to_ascii_lowercase());
                cookie_domain == site_host || cookie_domain.ends_with(&format!(".{site_host}"))
            } else {
                cookie_domain == craving_domain
                    || cookie_domain.ends_with(&format!(".{craving_domain}"))
            };
            let path_matches = craving.path.is_empty()
                || craving.path == "/"
                || cookie.path == craving.path
                || cookie.path.starts_with(&craving.path);
            if domain_matches && path_matches {
                return true;
            }
        }
    }
    false
}

fn cdp_cookie_to_imported(cookie: network::Cookie) -> ImportedCookie {
    let expires = if cookie.session || !cookie.expires.is_finite() || cookie.expires <= 0.0 {
        None
    } else {
        Some(cookie.expires)
    };
    let same_site = cookie.same_site.and_then(|s| match s {
        network::CookieSameSite::Strict => Some(SameSite::Strict),
        network::CookieSameSite::Lax => Some(SameSite::Lax),
        network::CookieSameSite::None => Some(SameSite::None),
        network::CookieSameSite::Unlisted => None,
    });
    ImportedCookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        path: if cookie.path.is_empty() {
            "/".to_owned()
        } else {
            cookie.path
        },
        expires,
        http_only: cookie.http_only,
        secure: cookie.secure,
        same_site,
    }
}

fn collect_candidate_origins(
    cookies: &[ImportedCookie],
    cookie_ports: &[(String, i64, bool)],
    domains: &[String],
) -> Vec<String> {
    let mut origins = BTreeSet::new();
    for cookie in cookies {
        let host = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
        if !host.is_empty() {
            origins.insert(format!("https://{host}"));
            if !cookie.secure {
                origins.insert(format!("http://{host}"));
            }
        }
    }
    for (host, port, secure) in cookie_ports {
        if !host.is_empty() {
            if *secure {
                origins.insert(format!("https://{host}:{port}"));
            } else {
                origins.insert(format!("http://{host}:{port}"));
            }
        }
    }
    for domain in domains {
        let trimmed = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if !trimmed.is_empty() && trimmed != "*" {
            origins.insert(format!("https://{trimmed}"));
        }
    }
    origins.into_iter().collect()
}

async fn read_origin_storage(
    connection: &Connection,
    session: &TargetSession,
    candidate_origins: &[String],
) -> Vec<ImportedOrigin> {
    if candidate_origins.is_empty() {
        return Vec::new();
    }
    let _ = connection
        .send(Some(session), &dom_storage::EnableParams {}, CDP_TIMEOUT)
        .await;
    let _ = connection
        .send(Some(session), &indexed_db::EnableParams {}, CDP_TIMEOUT)
        .await;

    let mut result = Vec::new();
    for origin in candidate_origins {
        let local_storage = read_dom_storage(connection, session, origin).await;
        let indexed_db = read_indexed_db(connection, session, origin).await;
        if !local_storage.is_empty() || indexed_db.is_some() {
            result.push(ImportedOrigin {
                origin: origin.clone(),
                local_storage,
                indexed_db,
                opfs: None,
            });
        }
    }
    result
}

async fn read_dom_storage(
    connection: &Connection,
    session: &TargetSession,
    origin: &str,
) -> Vec<(String, String)> {
    let Ok(items) = connection
        .send(
            Some(session),
            &dom_storage::GetDOMStorageItemsParams {
                storage_id: dom_storage::StorageId {
                    security_origin: Some(origin.to_owned()),
                    storage_key: None,
                    is_local_storage: true,
                },
            },
            CDP_TIMEOUT,
        )
        .await
    else {
        return Vec::new();
    };
    items
        .entries
        .into_iter()
        .filter_map(|mut pair| {
            if pair.len() >= 2 {
                let value = pair.remove(1);
                let key = pair.remove(0);
                Some((key, value))
            } else {
                None
            }
        })
        .collect()
}

async fn read_indexed_db(
    connection: &Connection,
    session: &TargetSession,
    origin: &str,
) -> Option<Value> {
    let names = connection
        .send(
            Some(session),
            &indexed_db::RequestDatabaseNamesParams {
                security_origin: Some(origin.to_owned()),
                storage_key: None,
                storage_bucket: None,
            },
            CDP_TIMEOUT,
        )
        .await
        .ok()?
        .database_names;
    if names.is_empty() {
        return None;
    }
    let mut databases = Vec::new();
    for database_name in names {
        let Ok(db) = connection
            .send(
                Some(session),
                &indexed_db::RequestDatabaseParams {
                    security_origin: Some(origin.to_owned()),
                    storage_key: None,
                    storage_bucket: None,
                    database_name: database_name.clone(),
                },
                CDP_TIMEOUT,
            )
            .await
        else {
            continue;
        };
        let mut stores = Vec::new();
        for store in db.database_with_object_stores.object_stores {
            let records =
                read_idb_store(connection, session, origin, &database_name, &store.name).await;
            let key_path = match store.key_path.r#type {
                indexed_db::KeyPathType::String => store.key_path.string.map(Value::String),
                indexed_db::KeyPathType::Array => store
                    .key_path
                    .array
                    .map(|arr| Value::Array(arr.into_iter().map(Value::String).collect())),
                indexed_db::KeyPathType::Null | indexed_db::KeyPathType::Unlisted => None,
            };
            stores.push(json!({
                "name": store.name,
                "autoIncrement": store.auto_increment,
                "keyPath": key_path,
                "records": records,
            }));
        }
        databases.push(json!({
            "name": db.database_with_object_stores.name,
            "version": db.database_with_object_stores.version,
            "stores": stores,
        }));
    }
    if databases.is_empty() {
        None
    } else {
        Some(Value::Array(databases))
    }
}

async fn read_idb_store(
    connection: &Connection,
    session: &TargetSession,
    origin: &str,
    database_name: &str,
    object_store_name: &str,
) -> Vec<Value> {
    let mut records = Vec::new();
    let mut skip_count = 0_i64;
    while records.len() < IDB_MAX_RECORDS {
        let Ok(page) = connection
            .send(
                Some(session),
                &indexed_db::RequestDataParams {
                    security_origin: Some(origin.to_owned()),
                    storage_key: None,
                    storage_bucket: None,
                    database_name: database_name.to_owned(),
                    object_store_name: object_store_name.to_owned(),
                    index_name: None,
                    skip_count,
                    page_size: IDB_PAGE_SIZE,
                    key_range: None,
                },
                CDP_TIMEOUT,
            )
            .await
        else {
            break;
        };
        let count = page.object_store_data_entries.len();
        for entry in page.object_store_data_entries {
            records.push(json!({
                "key": remote_object_value(&entry.key),
                "primaryKey": remote_object_value(&entry.primary_key),
                "value": remote_object_value(&entry.value),
            }));
        }
        if !page.has_more || count == 0 {
            break;
        }
        skip_count += i64::try_from(count).unwrap_or(IDB_PAGE_SIZE);
    }
    records
}

fn remote_object_value(object: &runtime::RemoteObject) -> Value {
    if let Some(value) = &object.value {
        return value.clone();
    }
    if let Some(unserializable) = &object.unserializable_value {
        return Value::String(unserializable.clone());
    }
    if let Some(description) = &object.description {
        return Value::String(description.clone());
    }
    Value::Null
}

#[expect(
    clippy::disallowed_methods,
    reason = "reads LOCALAPPDATA/PROGRAMFILES purely for OS install discovery inside blocking prepare_source"
)]
fn candidate_executables(browser: BrowserName) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        let paths: &[&str] = match browser {
            BrowserName::Chrome => &[
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            ],
            BrowserName::Edge => {
                &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"]
            }
            BrowserName::Brave => &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"],
            BrowserName::Chromium => &["/Applications/Chromium.app/Contents/MacOS/Chromium"],
            BrowserName::Firefox | BrowserName::Safari => &[],
        };
        paths.iter().map(PathBuf::from).collect()
    } else if cfg!(windows) {
        let mut bases = Vec::new();
        for var in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Ok(val) = std::env::var(var) {
                bases.push(PathBuf::from(val));
            }
        }
        let rel = match browser {
            BrowserName::Chrome => "Google/Chrome/Application/chrome.exe",
            BrowserName::Edge => "Microsoft/Edge/Application/msedge.exe",
            BrowserName::Brave => "BraveSoftware/Brave-Browser/Application/brave.exe",
            BrowserName::Chromium => "Chromium/Application/chrome.exe",
            BrowserName::Firefox | BrowserName::Safari => return Vec::new(),
        };
        bases.into_iter().map(|b| b.join(rel)).collect()
    } else {
        let paths: &[&str] = match browser {
            BrowserName::Chrome => &[
                "/opt/google/chrome/chrome",
                "/usr/bin/google-chrome",
                "/usr/bin/google-chrome-stable",
            ],
            BrowserName::Edge => &[
                "/opt/microsoft/msedge/msedge",
                "/usr/bin/microsoft-edge",
                "/usr/bin/microsoft-edge-stable",
            ],
            BrowserName::Brave => &[
                "/opt/brave.com/brave/brave",
                "/usr/bin/brave-browser",
                "/usr/bin/brave",
            ],
            BrowserName::Chromium => &[
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/usr/lib/chromium/chromium",
            ],
            BrowserName::Firefox | BrowserName::Safari => &[],
        };
        paths.iter().map(PathBuf::from).collect()
    }
}

fn missing_browser_error(browser: BrowserName, candidates: &[PathBuf]) -> CallError {
    let label = browser_family_label(browser);
    let looked = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    CallError::new(
        &BROWSER_NOT_INSTALLED,
        format!("No installed `{label}` browser found ({looked})."),
        &BrowserNotInstalledDetails {
            missing: MissingBrowser::Path {
                path: candidates
                    .first()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            },
            fix: format!(
                "Install `{label}` or specify its executable path in `session.source.executable`."
            ),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cookie(name: &str, domain: &str, path: &str) -> network::Cookie {
        network::Cookie {
            name: name.to_owned(),
            value: "val".to_owned(),
            domain: domain.to_owned(),
            path: path.to_owned(),
            expires: 1_900_000_000.0,
            size: 10,
            http_only: true,
            secure: true,
            session: false,
            same_site: Some(network::CookieSameSite::Lax),
            priority: network::CookiePriority::Medium,
            source_scheme: network::CookieSourceScheme::Secure,
            source_port: 443,
            partition_key: None,
            partition_key_opaque: None,
        }
    }

    #[test]
    fn device_bound_cookie_cravings_exclude_only_bound_cookies() {
        let dbsc = network::DeviceBoundSession {
            key: network::DeviceBoundSessionKey {
                site: "https://accounts.google.com".to_owned(),
                id: "dbsc-1".to_owned(),
            },
            refresh_url: "https://accounts.google.com/refresh".to_owned(),
            inclusion_rules: network::DeviceBoundSessionInclusionRules {
                origin: "https://accounts.google.com".to_owned(),
                include_site: true,
                url_rules: Vec::new(),
            },
            cookie_cravings: vec![network::DeviceBoundSessionCookieCraving {
                name: "__Secure-1PSIDTS".to_owned(),
                domain: ".google.com".to_owned(),
                path: "/".to_owned(),
                secure: true,
                http_only: true,
                same_site: None,
            }],
            expiry_date: 1_900_000_000.0,
            cached_challenge: None,
            allowed_refresh_initiators: Vec::new(),
        };
        let cookies = vec![
            sample_cookie("__Secure-1PSIDTS", ".google.com", "/"),
            sample_cookie("PREF", ".google.com", "/"),
            sample_cookie("other", "other.test", "/"),
        ];
        let (kept, _, excluded) =
            filter_device_bound_cookies(cookies, &[dbsc], &["google.com".to_owned()]);
        assert_eq!(excluded, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "PREF");
    }

    #[test]
    fn family_mismatch_warns_for_edge_and_brave_on_chrome_but_not_chrome_on_chrome() {
        assert!(family_mismatch_warning("chrome", "chrome").is_none());
        assert!(family_mismatch_warning("chromium", "chrome").is_none());
        let edge = family_mismatch_warning("edge", "chrome").expect("edge differs from chrome");
        assert_eq!(edge.code, IMPORTED_IDENTITY_FAMILY_DIFFERS);
        let brave = family_mismatch_warning("brave", "chrome").expect("brave differs from chrome");
        assert_eq!(brave.code, IMPORTED_IDENTITY_FAMILY_DIFFERS);
    }
}
