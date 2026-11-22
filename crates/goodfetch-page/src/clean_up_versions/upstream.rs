//! The Filter lists from their authors' own URLs (ADR 0047's `upstream`),
//! or a mirror's, as `clean_up.lists` names them: easylist.to's EasyList
//! and EasyPrivacy, fanboy.co.nz's Fanboy Annoyance and Brave's unbreak
//! list by default, each fetched again once the shortest expiry
//! they declare (`! Expires: 4 days`) has passed; the cookie sections of
//! the three lists dropped before they are kept, since the Consent prompt
//! runner handles those. The version they make is cached by the digest the
//! engine computes of its files, and that digest is what a Run holds to.
//! Runs that start together share one fetch, and a fetch that fails or
//! outlasts `clean_up.fetch_timeout` leaves a Run the last version fetched.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use goodfetch_clean_up_rules::rules::{EMBEDDED_MANIFEST, without_cookie_sections};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::rule_sources;
use goodfetch_protocol::clean_up_rules::{
    CleanUpRulesManifest, MANIFEST_FILE, RulesLists, RulesSource,
};
use goodfetch_protocol::formats::CLEAN_UP_RULES;
use serde_json::json;
use tokio::sync::OnceCell;

use super::failures::Unobtained;
use super::{FILE_LIMIT, Sources, Version, lock, within};

/// The file of each list in a version the engine fetches: the embedded
/// part's, as its manifest names them.
fn embedded_lists() -> RulesLists {
    serde_json::from_str::<CleanUpRulesManifest>(EMBEDDED_MANIFEST)
        .expect("invariant: the clean-up crate carries a manifest of this format")
        .lists
}

/// Each of `lists` by its key in `clean_up.lists` and in the manifest, with
/// its file.
fn by_key(lists: &RulesLists) -> [(&'static str, &str); 4] {
    [
        ("ads", &lists.ads),
        ("trackers", &lists.trackers),
        ("annoyances", &lists.annoyances),
        ("unbreak", &lists.unbreak),
    ]
}

/// A list's expiry when it declares none.
const DEFAULT_EXPIRY: Duration = Duration::from_hours(24);

/// The engine's fetches of the Filter lists: at most one in flight per set
/// of sources, which every Run starting meanwhile waits on, so Runs that
/// start together fetch each list once and hold to one version.
#[derive(Default)]
pub(super) struct Fetches {
    /// The fetch in flight, by its sources' set; its outcome stays until
    /// the first Run that waited on it takes it.
    flights: Mutex<BTreeMap<String, Arc<Flight>>>,
}

/// One fetch of the lists, and its outcome once it landed.
type Flight = OnceCell<Result<Version, Unobtained>>;

impl Fetches {
    /// The newest version from the lists' URLs: the last one fetched from
    /// the same URLs while none of its lists expired, otherwise a new one,
    /// fetched within `limit` on `clock`; the last one when that fetch
    /// fails or takes longer, so `limit` bounds the fetch, never the reading
    /// of the last one.
    pub(super) async fn newest(
        &self,
        sources: &Sources,
        now: jiff::Timestamp,
        clock: Clock,
        limit: Duration,
    ) -> Result<Version, Unobtained> {
        let last = match last(sources, now).await {
            Last::Fresh(last) => return Ok(last),
            Last::Stale(last) => last,
        };
        let fetched = within(
            clock,
            limit,
            self.shared(&set_of(sources), || fresh_or_fetched(sources, now)),
        )
        .await;
        match (fetched, last) {
            (Ok(version), _) => Ok(version),
            (Err(why), Some(last)) => {
                tracing::warn!(%why, "the Filter lists were not fetched again: the Run holds to the last version fetched");
                Ok(last)
            }
            (Err(why), None) => Err(why),
        }
    }

    /// The outcome of the fetch of `set` in flight, or of `fetch` when none
    /// is. A fetch stops when the caller that began it stops waiting, and a
    /// caller still waiting, or the next one, fetches again.
    async fn shared<Fetch, Fetching>(&self, set: &str, fetch: Fetch) -> Result<Version, Unobtained>
    where
        Fetch: FnOnce() -> Fetching,
        Fetching: Future<Output = Result<Version, Unobtained>>,
    {
        let flight = Arc::clone(lock(&self.flights).entry(set.to_owned()).or_default());
        let landed = flight.get_or_init(fetch).await.clone();
        let mut flights = lock(&self.flights);
        if flights
            .get(set)
            .is_some_and(|newest| Arc::ptr_eq(newest, &flight))
        {
            flights.remove(set);
        }
        landed
    }
}

/// The set `sources` fetch from: the lists' URLs, and what a fetch of
/// them may do, as it prints, so that Runs whose fetches could end
/// otherwise, offline or trusting other authorities, never share one.
fn set_of(sources: &Sources) -> String {
    json!({ "lists": sources.lists, "fetching": format!("{:?}", sources.fetching) }).to_string()
}

/// The last version fetched from the lists' URLs, as the sources' state
/// names it.
enum Last {
    /// Fetched from the same URLs, and none of its lists expired.
    Fresh(Version),
    /// Expired or fetched from other URLs, if any.
    Stale(Option<Version>),
}

/// The last version fetched from `sources`' lists, at `now`.
async fn last(sources: &Sources, now: jiff::Timestamp) -> Last {
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
    match last {
        Some(last) if same_urls && !last.expired(now) => Last::Fresh(last),
        last => Last::Stale(last),
    }
}

/// The last version fetched while it is fresh, as a fetch that landed
/// while this one waited to begin left it; otherwise a new one, fetched
/// now, which the sources' state then names.
async fn fresh_or_fetched(sources: &Sources, now: jiff::Timestamp) -> Result<Version, Unobtained> {
    if let Last::Fresh(last) = last(sources, now).await {
        return Ok(last);
    }
    let version = fetched(sources, now).await?;
    let mut state = sources.read_state().await;
    state["upstream"] = json!({ "version": version.digest, "lists": sources.lists });
    sources.write_state(state).await;
    Ok(version)
}

/// A new version of the lists, fetched now.
async fn fetched(sources: &Sources, now: jiff::Timestamp) -> Result<Version, Unobtained> {
    let lists = embedded_lists();
    let mut files = Vec::new();
    let mut expiry = None::<Duration>;
    for (list, file) in by_key(&lists) {
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
        published_at: SystemTime::from(now),
        expires_at: now
            .checked_add(expiry.unwrap_or(DEFAULT_EXPIRY))
            .ok()
            .map(SystemTime::from),
        lists,
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
