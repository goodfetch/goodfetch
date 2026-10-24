//! The Filter lists from their authors' own URLs (ADR 0047's `upstream`),
//! or a mirror's, as `clean_up.lists` names them: easylist.to's EasyList
//! and EasyPrivacy, fanboy.co.nz's Fanboy Annoyance and Brave's unbreak
//! list by default, each fetched again once the shortest expiry
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

use super::failures::Unobtained;
use super::{FILE_LIMIT, Sources, Version};

/// Each list: its key in `clean_up.lists` and in the manifest, and its file
/// in the version.
const LISTS: [(&str, &str); 4] = [
    ("ads", "easylist.txt"),
    ("trackers", "easyprivacy.txt"),
    ("annoyances", "fanboy-annoyance.txt"),
    ("unbreak", "brave-unbreak.txt"),
];

/// A list's expiry when it declares none.
const DEFAULT_EXPIRY: Duration = Duration::from_hours(24);

/// The newest version from the lists' URLs: the last one fetched from the
/// same URLs while none of its lists expired, otherwise a new one; the
/// last one when the fetch fails.
pub(super) async fn newest(sources: &Sources, now: jiff::Timestamp) -> Result<Version, Unobtained> {
    let state = sources.read_state().await;
    let last = match state["upstream"]["version"].as_str() {
        Some(digest) => sources
            .kept
            .cached(digest, now)
            .await
            .map(|(version, _)| version),
        None => None,
    };
    let same_urls = state["upstream"]["lists"] == json!(sources.lists);
    if let Some(last) = &last
        && same_urls
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
    state["upstream"] = json!({ "version": version.digest, "lists": sources.lists });
    sources.write_state(state).await;
    Ok(version)
}

/// A new version of the lists, fetched now.
async fn fetched(sources: &Sources, now: jiff::Timestamp) -> Result<Version, Unobtained> {
    let mut files = Vec::new();
    let mut expiry = None::<Duration>;
    for (list, file) in LISTS {
        let url = sources.lists.get(list).ok_or_else(|| {
            Unobtained::Unreadable(format!(
                "`clean_up.lists` names no URL for the `{list}` Filter list"
            ))
        })?;
        let answer = rule_sources::get(url, &[], &sources.fetching, FILE_LIMIT)
            .await
            .map_err(|failure| Unobtained::of(failure, &sources.fetching))?;
        if answer.status != 200 {
            return Err(Unobtained::Refused {
                host: url::Url::parse(url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_default(),
                status: answer.status,
            });
        }
        let text = String::from_utf8(answer.body).map_err(|_| {
            Unobtained::Unreadable(format!("the `{list}` Filter list is not UTF-8 text"))
        })?;
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
        .keep(RulesSource::Upstream, files, now)
        .await
        .map(|(version, _)| version)
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
