//! `firefox-profile` (#1535): a Firefox profile folder whose `cookies.sqlite`
//! holds a Session logged in on `login/3`, the seeded login, at one time
//! step. A plain SQLite client reads it here, as `identity import` reads an
//! installed Firefox's (ADR 0056), and the Proving ground accepts its cookie
//! until the time step changes.

use std::path::Path;
use std::process::Command;

use goodfetch_test_support::TestEnv;
use rusqlite::{Connection, OpenFlags};
use serde_json::json;

use crate::proving_ground::ProvingGround;

const SEED: u64 = 1535;

/// Firefox's `COOKIES_SCHEMA_VERSION` (`CookiePersistentStorage.cpp`).
const COOKIES_SCHEMA_VERSION: i64 = 17;

/// One row of `moz_cookies`, as Firefox reads it.
#[derive(Debug, PartialEq)]
struct Cookie {
    name: String,
    value: String,
    host: String,
    path: String,
    is_http_only: i64,
    scheme_map: i64,
}

fn write_profile(folder: &Path, seed: u64, step: u64) {
    let status = Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
        .args(["firefox-profile", "--seed", &seed.to_string()])
        .args(["--time-step", &step.to_string(), "--out"])
        .arg(folder)
        .env_clear()
        .status()
        .expect("the Proving ground runs");
    assert!(status.success(), "firefox-profile exited with {status}");
}

/// The schema version and every cookie of the profile's `cookies.sqlite`.
fn read_cookies(folder: &Path) -> (i64, Vec<Cookie>) {
    let database = Connection::open_with_flags(
        folder.join("cookies.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the profile holds cookies.sqlite");
    let version = database
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("a schema version");
    let mut statement = database
        .prepare(
            "SELECT name, value, host, path, isHttpOnly, schemeMap FROM moz_cookies ORDER BY id",
        )
        .expect("Firefox's cookie table");
    let cookies = statement
        .query_map([], |row| {
            Ok(Cookie {
                name: row.get(0)?,
                value: row.get(1)?,
                host: row.get(2)?,
                path: row.get(3)?,
                is_http_only: row.get(4)?,
                scheme_map: row.get(5)?,
            })
        })
        .expect("the cookies")
        .collect::<Result<_, _>>()
        .expect("each cookie");
    (version, cookies)
}

/// Covers: story:2-33
#[test]
fn the_profiles_session_is_accepted_until_the_time_step_changes() {
    let env = TestEnv::new();
    let folder = env.folder().join("profile");
    write_profile(&folder, SEED, 0);
    let (version, cookies) = read_cookies(&folder);
    let proving_ground = ProvingGround::start(SEED);
    let [cookie] = cookies.as_slice() else {
        panic!("one cookie: {cookies:?}");
    };
    let session = format!("{}={}", cookie.name, cookie.value);
    let url = proving_ground.url(&cookie.host, "/account");

    let at_its_step = proving_ground.get_with_cookie(&url, Some(&session));
    proving_ground.set_time_step(&json!(1));
    let after = proving_ground.get_with_cookie(&url, Some(&session));

    assert_eq!(version, COOKIES_SCHEMA_VERSION);
    assert_eq!(
        (
            cookie.name.as_str(),
            cookie.host.as_str(),
            cookie.path.as_str(),
            cookie.is_http_only,
            cookie.scheme_map
        ),
        ("session", "3.login.test", "/", 1, 1)
    );
    assert_eq!(at_its_step.status, 200, "{}", at_its_step.body);
    assert_eq!(after.status, 303, "{}", after.body);
    assert_eq!(after.header("location"), Some("/login"));
}

/// An imported login that expired is imported again from a profile written
/// at the new time step, over the old one.
///
/// Covers: story:2-33
#[test]
fn a_profile_written_again_at_the_new_time_step_is_accepted_again() {
    let env = TestEnv::new();
    let folder = env.folder().join("profile");
    write_profile(&folder, SEED, 0);
    let (_, first) = read_cookies(&folder);
    let proving_ground = ProvingGround::start(SEED);
    proving_ground.set_time_step(&json!(1));

    write_profile(&folder, SEED, 1);
    let (_, again) = read_cookies(&folder);
    let [cookie] = again.as_slice() else {
        panic!("one cookie: {again:?}");
    };
    let page = proving_ground.get_with_cookie(
        &proving_ground.url(&cookie.host, "/account"),
        Some(&format!("{}={}", cookie.name, cookie.value)),
    );

    assert_ne!(first, again);
    assert_eq!(page.status, 200, "{}", page.body);
}

/// Covers: story:2-33
#[test]
fn the_same_seed_and_time_step_write_the_same_profile() {
    let env = TestEnv::new();
    let (one, other) = (env.folder().join("one"), env.folder().join("other"));
    write_profile(&one, SEED, 0);
    write_profile(&other, SEED, 0);

    assert_eq!(
        std::fs::read(one.join("cookies.sqlite")).expect("the first profile"),
        std::fs::read(other.join("cookies.sqlite")).expect("the second profile")
    );
}
