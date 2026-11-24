//! The Firefox profile seed (#1535): a profile folder whose `cookies.sqlite`
//! holds one Session cookie, in the `moz_cookies` table of Firefox's current
//! cookie schema, version 17 (`CookiePersistentStorage.cpp`), so a Journey
//! prepares a Firefox logged in on the Proving ground's own login and
//! `identity import` reads it as it reads a person's (ADR 0056). Every time
//! it writes is fixed, never read from a clock, so the same Session writes
//! the same file.
#![expect(
    clippy::disallowed_methods,
    reason = "`firefox-profile` writes the profile on its main thread and exits, with no runtime"
)]

use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};

use crate::login::SessionCookie;

/// Firefox's `COOKIES_SCHEMA_VERSION`, kept in `PRAGMA user_version`.
const SCHEMA_VERSION: u32 = 17;

/// The table as Firefox creates it at that version.
const CREATE_TABLE: &str = "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER, inBrowserElement INTEGER DEFAULT 0, sameSite INTEGER DEFAULT 0, schemeMap INTEGER DEFAULT 0, isPartitionedAttributeSet INTEGER DEFAULT 0, updateTime INTEGER, CONSTRAINT moz_uniqueid UNIQUE (name, host, path, originAttributes))";

/// The one cookie, host-only on path `/`, `HttpOnly` and not `Secure`, as the
/// login's `Set-Cookie` sets it, set, last used and last updated at once.
const INSERT_COOKIE: &str = "INSERT INTO moz_cookies (name, value, host, path, expiry, lastAccessed, creationTime, isSecure, isHttpOnly, sameSite, schemeMap, updateTime) VALUES (?1, ?2, ?3, '/', ?4, ?5, ?5, 0, 1, ?6, ?7, ?5)";

/// The cookie's expiry, 2100-01-01, in milliseconds since the Unix epoch
/// (Firefox's unit since schema 16): far enough that Firefox never purges
/// it, since the Proving ground, not the cookie, ends the Session.
const EXPIRY_MS: i64 = 4_102_444_800_000;
/// When the cookie was set, last used and last updated, 2026-01-01, in
/// microseconds since the Unix epoch.
const SET_AT_US: i64 = 1_767_225_600_000_000;
/// `nsICookie::SAMESITE_LAX`, as the login's `Set-Cookie` says.
const SAME_SITE_LAX: i64 = 1;
/// `nsICookie::SCHEME_HTTP`: the Proving ground's login is served over HTTP.
const SCHEME_HTTP: i64 = 1;

/// The cookie database and the files SQLite and Firefox keep beside it.
const FILES: [&str; 4] = [
    "cookies.sqlite",
    "cookies.sqlite-wal",
    "cookies.sqlite-shm",
    "cookies.sqlite-journal",
];

/// Writes `folder/cookies.sqlite` holding `cookie` alone, creating the folder
/// when missing and replacing any cookie database already in it.
pub(crate) fn write(folder: &Path, cookie: &SessionCookie) -> Result<()> {
    std::fs::create_dir_all(folder)
        .with_context(|| format!("creating the profile folder {}", folder.display()))?;
    for file in FILES {
        let path = folder.join(file);
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(error).with_context(|| format!("replacing {}", path.display()));
            }
            _ => {}
        }
    }
    let path = folder.join(FILES[0]);
    let database =
        Connection::open(&path).with_context(|| format!("creating {}", path.display()))?;
    database
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .and_then(|()| database.execute_batch(CREATE_TABLE))
        .and_then(|()| database.prepare_cached(INSERT_COOKIE))
        .and_then(|mut insert| {
            insert.execute(params![
                cookie.name,
                cookie.value,
                cookie.host,
                EXPIRY_MS,
                SET_AT_US,
                SAME_SITE_LAX,
                SCHEME_HTTP
            ])
        })
        .with_context(|| format!("writing {}", path.display()))?;
    database
        .close()
        .map_err(|(_, error)| error)
        .with_context(|| format!("closing {}", path.display()))
}
