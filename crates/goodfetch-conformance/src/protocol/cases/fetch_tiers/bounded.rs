//! The Response cache's bounds cases of the `http` tier (ADRs 0046 and
//! 0122), crawls of `cache.test` as `cached`'s are, each setting a bound in
//! the target's user configuration, which the next Run's start reads, and
//! putting it back after. Past `cache.max_size` the least recently used
//! entries leave, and the body files the cache keeps weigh no more than
//! the cap; past `cache.max_age_ceiling` an entry is served in no mode; and
//! while the cache's filesystem is at the `low` storage level nothing new
//! is stored, while what the cache held is still served. The `critical`
//! level is the engine's unit tests': a Run cannot start there (ADR 0122).

use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::home::RESPONSE_CACHE;
use goodfetch_protocol::needs_you::{NEEDS_YOU_LIST, STORAGE_LEVEL};
use serde_json::json;

use super::cached::{crawl, is_answer, post, requests_for, settings};
use super::{Checks, Connection, Gone, Site};
use crate::protocol::cases::consent::walls::{configure, major_of};
use crate::protocol::cases::{poll, user_configuration};

/// The size cap the size case sets, in bytes: more than two of the cache
/// host's large pages and the index together, less than three pages.
const SIZE_CAP: u64 = 3_000_000;

/// The weight of a large page's comment: one mebibyte.
const LARGE: u64 = 1 << 20;

/// The age ceiling the ceiling case sets, in milliseconds.
const CEILING_MS: u64 = 1_000;

/// How long the ceiling case lets its entries age: past the ceiling.
const AGED: Duration = Duration::from_millis(1_500);

/// The free space below which the low-storage case puts every filesystem
/// at `low`: 2⁶⁰ bytes, more than any disk holds. `critical` is off.
const LOW: &str = "[settings.storage]\nlow_free = 1152921504606846976\ncritical_free = 0\n";

/// How long "Needs you" may take to list a level: the next Run's start
/// reads user configuration, then the governor's next storage sample.
const LEVEL_WAIT: Duration = Duration::from_secs(20);

/// Runs `cases` with `settings`, TOML tables, as user configuration in the
/// per-major folder of `home` that holds the Run `run_id`, handed that
/// folder, and puts user configuration back after.
fn configured(
    home: Option<&str>,
    run_id: &str,
    settings: &str,
    checks: &mut Checks,
    cases: impl FnOnce(&Path, &mut Checks) -> Result<(), Gone>,
) -> Result<(), Gone> {
    let Some(major) = home.and_then(|home| major_of(home, run_id)) else {
        checks.check("the engine's per-major folder is found", false);
        return Ok(());
    };
    let Some(before) = checks.needs(
        "user configuration sets the case's bound",
        configure(&major, Some(&user_configuration(settings))),
    ) else {
        return Ok(());
    };
    let outcome = cases(&major, checks);
    checks.needs(
        "user configuration is put back",
        configure(&major, before.as_deref()),
    );
    outcome
}

/// The bytes of the body files the Response cache in `major` keeps, each
/// named by its body's SHA-256.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn body_bytes(major: &Path) -> std::io::Result<u64> {
    let mut bytes = 0;
    let mut folders = vec![major.join(RESPONSE_CACHE)];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file()
                && entry.file_name().to_str().is_some_and(|name| {
                    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            {
                bytes += entry.metadata()?.len();
            }
        }
    }
    Ok(bytes)
}

/// The storage levels "Needs you" lists, once `done` holds of them or the
/// wait ends.
fn listed_levels(
    connection: &mut Connection,
    done: impl Fn(&Vec<String>) -> bool,
) -> Result<Vec<String>, Gone> {
    poll::try_until(
        LEVEL_WAIT,
        Duration::from_millis(250),
        || {
            let id =
                connection.request(NEEDS_YOU_LIST.name, &json!({ "kinds": [STORAGE_LEVEL] }))?;
            let answer = connection.answer(id)?.message;
            Ok(answer["result"]["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item["details"]["level"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default())
        },
        done,
    )
}

/// Under `cache.mode: revalidate` with `cache.max_size` below three of the
/// cache host's large pages, a page stored before the two others and used
/// since stays, while the page used least recently leaves: the last page
/// stored and the page used are handed their stored first answers, the
/// one left reaches the site again, and the body files kept weigh no more
/// than the cap.
pub(super) fn size_cap(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let revalidate = settings(Some("revalidate"));
    let (used, left, last) = ("/large/used", "/large/left", "/large/last");
    let first = "conformance-cache-size-1";
    crawl(connection, site, first, used, &revalidate, checks)?;
    let cap = format!("[settings.cache]\nmax_size = {SIZE_CAP}\n");
    configured(home, first, &cap, checks, |major, checks| {
        crawl(
            connection,
            site,
            "conformance-cache-size-2",
            left,
            &revalidate,
            checks,
        )?;
        let hit = crawl(
            connection,
            site,
            "conformance-cache-size-3",
            used,
            &revalidate,
            checks,
        )?;
        crawl(
            connection,
            site,
            "conformance-cache-size-4",
            last,
            &revalidate,
            checks,
        )?;
        let kept = checks.needs("the cache's body files are read", body_bytes(major));
        let last_again = crawl(
            connection,
            site,
            "conformance-cache-size-5",
            last,
            &revalidate,
            checks,
        )?;
        let used_again = crawl(
            connection,
            site,
            "conformance-cache-size-6",
            used,
            &revalidate,
            checks,
        )?;
        let left_again = crawl(
            connection,
            site,
            "conformance-cache-size-7",
            left,
            &revalidate,
            checks,
        )?;
        checks.check(
            "the page stored first is served, which uses it again",
            is_answer(&hit, 1, used),
        );
        checks.check(
            "past the cap, the body files kept weigh no more than `cache.max_size`, two large pages",
            kept.is_some_and(|kept| (2 * LARGE..=SIZE_CAP).contains(&kept)),
        );
        checks.check(
            "the page stored last and the page used since are still served",
            is_answer(&last_again, 1, last)
                && is_answer(&used_again, 1, used)
                && requests_for(site, last).len() == 1
                && requests_for(site, used).len() == 1,
        );
        checks.check(
            "the page used least recently left the cache: the site sees it again",
            is_answer(&left_again, 2, left) && requests_for(site, left).len() == 2,
        );
        Ok(())
    })
}

/// With `cache.max_age_ceiling` at a second, a page the site lets be kept
/// an hour and a `POST` `replay` kept, both stored before, reach the site
/// again once older than the ceiling, under `revalidate` and `replay`.
pub(super) fn age_ceiling(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (revalidate, replay) = (settings(Some("revalidate")), settings(Some("replay")));
    let (page, api, body) = ("/fresh/aged", "/api/aged", r#"{"q":"aged"}"#);
    let first = "conformance-cache-ceiling-1";
    crawl(connection, site, first, page, &revalidate, checks)?;
    post(
        connection,
        site,
        "conformance-cache-ceiling-2",
        (api, body),
        &replay,
        checks,
    )?;
    let ceiling = format!("[settings.cache]\nmax_age_ceiling = {CEILING_MS}\n");
    configured(home, first, &ceiling, checks, |_, checks| {
        outlive();
        let fresh = crawl(
            connection,
            site,
            "conformance-cache-ceiling-3",
            page,
            &revalidate,
            checks,
        )?;
        let replayed = post(
            connection,
            site,
            "conformance-cache-ceiling-4",
            (api, body),
            &replay,
            checks,
        )?;
        checks.check(
            "past `cache.max_age_ceiling`, a page fresh for an hour is not served under `revalidate`",
            is_answer(&fresh, 2, page) && requests_for(site, page).len() == 2,
        );
        checks.check(
            "past `cache.max_age_ceiling`, a `POST` kept by `replay` is not served",
            is_answer(&replayed, 2, api) && requests_for(site, api).len() == 2,
        );
        Ok(())
    })
}

/// Lets the ceiling case's entries outlive the ceiling.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness waits on its own thread, outside any async worker (ADR 0060's note for the harness)"
)]
fn outlive() {
    std::thread::sleep(AGED);
}

/// Under `cache.mode: revalidate`, with `storage.low_free` above any
/// filesystem's free space, "Needs you" lists the cache's filesystem at
/// `low`; a page the site lets be kept an hour is then not stored, its
/// second crawl reaching the site, while the page the cache held before is
/// still served. Once user configuration is put back, the next Run's start
/// brings the level back to `normal`, for the cases after.
pub(super) fn low_storage(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let revalidate = settings(Some("revalidate"));
    let (held, shed) = ("/fresh/held", "/fresh/shed");
    let first = "conformance-cache-low-1";
    crawl(connection, site, first, held, &revalidate, checks)?;
    configured(home, first, LOW, checks, |_, checks| {
        crawl(
            connection,
            site,
            "conformance-cache-low-2",
            held,
            &revalidate,
            checks,
        )?;
        let levels = listed_levels(connection, |levels| {
            levels.iter().any(|level| level == "low")
        })?;
        checks.check(
            "\"Needs you\" lists a filesystem at the `low` storage level",
            levels.iter().any(|level| level == "low"),
        );
        crawl(
            connection,
            site,
            "conformance-cache-low-3",
            shed,
            &revalidate,
            checks,
        )?;
        let second = crawl(
            connection,
            site,
            "conformance-cache-low-4",
            shed,
            &revalidate,
            checks,
        )?;
        let still = crawl(
            connection,
            site,
            "conformance-cache-low-5",
            held,
            &revalidate,
            checks,
        )?;
        checks.check(
            "at `low`, a page fresh for an hour is not stored: its second crawl reaches the site",
            is_answer(&second, 2, shed) && requests_for(site, shed).len() == 2,
        );
        checks.check(
            "at `low`, the page the cache held is still served",
            is_answer(&still, 1, held) && requests_for(site, held).len() == 1,
        );
        Ok(())
    })?;
    crawl(
        connection,
        site,
        "conformance-cache-low-6",
        held,
        &revalidate,
        checks,
    )?;
    let levels = listed_levels(connection, |levels| {
        levels.iter().all(|level| level != "low")
    })?;
    checks.check(
        "once user configuration is put back, no filesystem is listed at `low`",
        levels.iter().all(|level| level != "low"),
    );
    Ok(())
}
