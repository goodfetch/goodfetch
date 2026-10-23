//! The Filter lists from their authors' own URLs (ADR 0047's `upstream`):
//! easylist.to's EasyList and EasyPrivacy, fanboy.co.nz's Fanboy Annoyance
//! and Brave's unbreak list, each fetched again once the shortest expiry
//! they declare (`! Expires: 4 days`) has passed; the cookie sections of
//! the three lists dropped before they are kept, since the Consent prompt
//! runner handles those. The version they make is cached by the digest the
//! engine computes of its files, and that digest is what a Run holds to.

use std::time::Duration;

use goodfetch_clean_up_rules::rules::without_cookie_sections;
use goodfetch_engine::rule_sources;
use goodfetch_protocol::clean_up_rules::{
    CleanUpRulesManifest, MANIFEST_FILE, RulesLists, RulesSource,
};
use goodfetch_protocol::formats::CLEAN_UP_RULES;
use serde_json::json;

use super::{FILE_LIMIT, Sources, Version};

/// Each list: the setting that turns it on, its file in the version, and
/// its author's URL.
pub(crate) const LISTS: [(&str, &str, &str); 4] = [
    (
        "ads",
        "easylist.txt",
        "https://easylist.to/easylist/easylist.txt",
    ),
    (
        "trackers",
        "easyprivacy.txt",
        "https://easylist.to/easylist/easyprivacy.txt",
    ),
    (
        "annoyances",
        "fanboy-annoyance.txt",
        "https://secure.fanboy.co.nz/fanboy-annoyance.txt",
    ),
    (
        "unbreak",
        "brave-unbreak.txt",
        "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-unbreak.txt",
    ),
];

/// A list's expiry when it declares none.
const DEFAULT_EXPIRY: Duration = Duration::from_secs(24 * 60 * 60);

/// The newest version from the lists' authors: the last one fetched while
/// none of its lists expired, otherwise a new one; the last one when the
/// fetch fails.
pub(super) async fn newest(sources: &Sources, now: jiff::Timestamp) -> Result<Version, String> {
    let state = sources.read_state().await;
    let last = match state["upstream"]["version"].as_str() {
        Some(digest) => sources
            .kept
            .cached(digest, now)
            .await
            .map(|(version, _)| version),
        None => None,
    };
    if let Some(last) = &last
        && !last.expired(now)
    {
        return Ok(last.clone());
    }
    let version = match fetched(sources, now).await {
        Ok(version) => version,
        Err(why) => {
            return match last {
                Some(last) => {
                    tracing::warn!(%why, "the Filter lists were not fetched again: the Run holds to the last version fetched");
                    Ok(last)
                }
                None => Err(why),
            };
        }
    };
    let mut state = state.clone();
    state["upstream"] = json!({ "version": version.digest });
    sources.write_state(state).await;
    Ok(version)
}

/// A new version of the lists, fetched now.
async fn fetched(sources: &Sources, now: jiff::Timestamp) -> Result<Version, String> {
    let mut files = Vec::new();
    let mut expiry = None::<Duration>;
    for (list, file, url) in LISTS {
        let answer = rule_sources::get(url, &[], &sources.fetching, FILE_LIMIT)
            .await
            .map_err(|failure| format!("the `{list}` Filter list cannot be fetched: {failure}"))?;
        if answer.status != 200 {
            return Err(format!(
                "the `{list}` Filter list's author answered {}",
                answer.status
            ));
        }
        let text = String::from_utf8(answer.body)
            .map_err(|_| format!("the `{list}` Filter list is not UTF-8 text"))?;
        let declared = expires_in(&text).unwrap_or(DEFAULT_EXPIRY);
        expiry = Some(expiry.map_or(declared, |shortest| shortest.min(declared)));
        let kept = if list == "unbreak" {
            text
        } else {
            without_cookie_sections(&text)
        };
        files.push((file.to_owned(), kept.into_bytes()));
    }
    let manifest = CleanUpRulesManifest {
        format_version: CLEAN_UP_RULES.version,
        source: RulesSource::Upstream,
        published_at: now,
        expires_at: now.checked_add(expiry.unwrap_or(DEFAULT_EXPIRY)).ok(),
        lists: RulesLists {
            ads: LISTS[0].1.to_owned(),
            trackers: LISTS[1].1.to_owned(),
            annoyances: LISTS[2].1.to_owned(),
            unbreak: LISTS[3].1.to_owned(),
        },
    };
    files.push((MANIFEST_FILE.to_owned(), super::manifest_file(&manifest)));
    sources
        .kept
        .keep(RulesSource::Upstream, &files, now)
        .await
        .map_err(|why| format!("the Filter lists fetched from their authors {why}"))
}

/// The expiry `text`, a Filter list, declares in its header's
/// `! Expires: <n> days` or `hours`.
fn expires_in(text: &str) -> Option<Duration> {
    let line = text
        .lines()
        .take_while(|line| line.starts_with('!') || line.starts_with('['))
        .find_map(|line| line.strip_prefix("! Expires:"))?;
    let mut words = line.split_whitespace();
    let count: u64 = words.next()?.parse().ok()?;
    let hours = match words.next()? {
        "day" | "days" => count.checked_mul(24)?,
        "hour" | "hours" => count,
        _ => return None,
    };
    Some(Duration::from_secs(hours.checked_mul(60 * 60)?))
}
