//! What a site's preferences say of an attempt's request (ADRs 0086 and
//! 0093): its origin's robots.txt, read from the copy the user's runs
//! share and its Crawl-delay recorded and waited; the refusal of a request
//! that robots.txt disallows or whose Crawl-delay exceeds the crawl's
//! bound; the AI-use signals its host declares before it is sent and those
//! its document's response and markup carry; and a handler's escaped
//! `site_preferences_refused`.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::lease::LeaseCheck;
use goodfetch_engine::politeness::Politeness;
use goodfetch_engine::robots_txt::{self, Fetch, Fetched};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{
    FetchTiers, HttpTier, Purpose, SignalFiles, TDMREP_FILE, content_signals_refusing,
    rsl_refusing, tdmrep_file_refusing,
};
use goodfetch_frontier::SharedRobotsTxt;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_DOMAIN_DELAY, SITE_PREFERENCES_MAX_CRAWL_DELAY, SITE_PREFERENCES_RESPECT_ROBOTS_TXT,
    SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT,
};
use goodfetch_protocol::page::{PageId, SITE_PREFERENCES_REFUSED, SitePreference};
use goodfetch_protocol::site_preferences::{
    SITE_PREFERENCES_PURPOSE, SITE_PREFERENCES_RESPECT_USE_SIGNALS,
};
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use super::Answer;
use crate::work_items::Leased;

/// The Politeness the crawl's Run shares, once it opened: from then on
/// the Run reads robots.txt from the copy its user's runs share there
/// (ADR 0086). A Politeness file that cannot open leaves the Run its own
/// copy.
pub(super) async fn share_robots_txt(run: &Run) -> Option<Politeness> {
    let settings = run.settings()?.resolved();
    match run.politeness(settings).await {
        Ok((politeness, _)) => {
            let shared = politeness.clone();
            run.share_robots_txt(|| Arc::new(SharedRobotsTxt::new(shared, run.clock())));
            Some(politeness)
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %run.id(), "the shared robots.txt copy is out of reach; the crawl reads its own");
            None
        }
    }
}

/// Reads the robots.txt of `leased`'s origin, as the crawl's Run reads it:
/// from the copy its user's runs share, fetched on `http` only when the
/// copy has none to use. The Crawl-delay it asks for is recorded for the
/// request's domain key, for every run sharing the domain. A robots.txt
/// fetch is a request of the domain: while the one the attempt made, or
/// the one whose copy it read, is less than the delay before, the document
/// starts `politeness.domain.delay` after that fetch's response arrived,
/// or the Crawl-delay when the crawl waits it out and it is longer, and no
/// sooner than that after the start another sharing run took meanwhile;
/// that start is taken in the Politeness file for every run sharing the
/// domain. Counted from the response's arrival, the site never receives
/// the document sooner than the delay after the robots.txt request,
/// whatever that request's connection cost. That wait is a permit's wait
/// for the domain's delay: `lease` is held through it, so it counts
/// against no timeout and the Run's live status names it (ADR 0093).
/// Answers what applies, or `None` for a URL with no robots.txt.
pub(super) async fn read_robots_txt(
    run: &Run,
    http: &HttpTier,
    leased: &Leased,
    lease: &dyn LeaseCheck,
) -> Option<Arc<Fetched>> {
    let location = goodfetch_frontier::Url::parse(&leased.crawl_request.url)
        .ok()
        .and_then(|url| robots_txt::location(&url))?;
    let politeness = share_robots_txt(run).await;
    let (arriving, arrived) = oneshot::channel::<SystemTime>();
    let fetch: Fetch<'_> = Box::pin(async {
        let answer = http.robots_txt(run, &location).await;
        let _ = arriving.send(run.clock().wall());
        answer
    });
    let read = run.robots_txt().copy(&location, fetch).await;
    let Some(settings) = run.settings().map(|settings| settings.resolved().clone()) else {
        return Some(read);
    };
    let asked = crawl_delay_asked(&settings, &read);
    if let Some(politeness) = &politeness
        && let Err(error) = goodfetch_frontier::crawl_delay_read(
            politeness,
            &leased.waiting.key,
            location.as_str(),
            asked,
        )
        .await
    {
        tracing::warn!(%error, run_id = %run.id(), "a host's Crawl-delay was not recorded for its domain");
    }
    let delay = settings
        .duration(POLITENESS_DOMAIN_DELAY.name)
        .max(asked.filter(|asked| waits_out(&settings, *asked)))
        .filter(|delay| !delay.is_zero());
    let Some(delay) = delay else {
        return Some(read);
    };
    let fetched = arrived.await.ok();
    let start = match &politeness {
        Some(politeness) => goodfetch_frontier::start_after_robots_txt(
            politeness,
            &leased.waiting.key,
            location.as_str(),
            fetched,
            delay,
            run.clock().wall(),
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, run_id = %run.id(), "a document's start after its robots.txt was not recorded");
            fetched.map(|fetched| fetched + delay)
        }),
        None => fetched.map(|fetched| fetched + delay),
    };
    if let Some(wait) = start.and_then(|start| start.duration_since(run.clock().wall()).ok()) {
        let _waiting = lease.permit_wait(&leased.crawl_request.url);
        run.clock().sleep(wait).await;
    }
    Some(read)
}

/// The Crawl-delay the robots.txt `read` asks of a crawl under `settings`,
/// its operator token's group merged with Foxhound's (ADR 0086).
fn crawl_delay_asked(settings: &Resolved, read: &Fetched) -> Option<Duration> {
    let operator_token = settings
        .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
        .as_str();
    goodfetch_fetch_tier::crawl_delay(read, operator_token)
}

/// Whether a crawl under `settings` waits out the Crawl-delay `asked`:
/// while `site_preferences.respect_robots_txt` is on, up to
/// `site_preferences.max_crawl_delay` (ADR 0086).
fn waits_out(settings: &Resolved, asked: Duration) -> bool {
    settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) != Some(false)
        && settings
            .duration(SITE_PREFERENCES_MAX_CRAWL_DELAY.name)
            .is_some_and(|max| asked <= max)
}

/// The `site_preferences.purpose` a crawl under `settings` judges the
/// AI-use signals against: none while `site_preferences.respect_use_signals`
/// is off, or when the Purpose is `search`, which is no declaration (ADR
/// 0086).
pub(super) fn honoured_purpose(settings: &Resolved) -> Option<Purpose> {
    if settings.boolean(SITE_PREFERENCES_RESPECT_USE_SIGNALS.name) != Some(true) {
        return None;
    }
    settings
        .value(SITE_PREFERENCES_PURPOSE.name)
        .as_str()
        .and_then(Purpose::named)
        .filter(|purpose| *purpose != Purpose::Search)
}

/// What the AI-use signals `leased`'s host declared before its request is
/// sent say of it while `site_preferences.respect_use_signals` is on (ADRs
/// 0016 and 0086): the `Content-Signal` lines of the robots.txt `read`'s
/// groups that apply, and `TDMRep`'s well-known file, fetched on `http`
/// once per origin for the Run through `files`. Nothing when none refuses
/// the Run's `site_preferences.purpose`, else the attempt's answer, sent
/// nothing.
pub(super) async fn use_signals_declared(
    run: &Run,
    http: &HttpTier,
    leased: &Leased,
    read: Option<&Fetched>,
    files: &SignalFiles,
) -> Option<Answer> {
    let settings = run.settings()?.resolved();
    let purpose = honoured_purpose(settings)?;
    let refused = Some(Answer::Refused(SkipReason::UseSignals));
    if let Some(read) = read {
        let operator_token = settings
            .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
            .as_str();
        let signals = goodfetch_fetch_tier::content_signals(read, operator_token);
        if content_signals_refusing(&signals, purpose).is_some() {
            return refused;
        }
    }
    let url = goodfetch_frontier::Url::parse(&leased.crawl_request.url).ok()?;
    let location = url.join(TDMREP_FILE).ok()?;
    let file = files
        .read(&location, http.signal_file(run, &location))
        .await?;
    tdmrep_file_refusing(&file, &url, purpose).and(refused)
}

/// What the AI-use signals the response of the served page `page_id`
/// carried, in its headers and its document's markup, say of its request
/// while `site_preferences.respect_use_signals` is on (ADRs 0045, 0086 and
/// 0093), the RSL license they name fetched on `http` once for the Run
/// through `files`: nothing when none refuses the Run's
/// `site_preferences.purpose`, else the attempt's answer, its handler
/// never run.
pub(super) async fn use_signals_refusal(
    run: &Run,
    (tiers, http): (&FetchTiers, &HttpTier),
    page_id: &PageId,
    files: &SignalFiles,
) -> Option<Answer> {
    let purpose = honoured_purpose(run.settings()?.resolved())?;
    let refused = Some(Answer::Refused(SkipReason::UseSignals));
    let signals = tiers.use_signals(page_id);
    if signals.refusing(purpose).is_some() {
        return refused;
    }
    let license = signals.license()?;
    let file = files
        .read(&license, http.signal_file(run, &license))
        .await?;
    rsl_refusing(&file, purpose).and(refused)
}

/// What the robots.txt `read` for `leased`'s request says of it while
/// `site_preferences.respect_robots_txt` is on (ADR 0086): nothing when
/// it allows it and asks for no Crawl-delay above
/// `site_preferences.max_crawl_delay`, else the attempt's answer, sent
/// nothing.
pub(super) fn robots_txt_refusal(
    run: &Run,
    leased: &Leased,
    read: Option<&Fetched>,
) -> Option<Answer> {
    let settings = run.settings()?.resolved();
    if settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) == Some(false) {
        return None;
    }
    let read = read?;
    if let Fetched::Unreachable(reason) = read {
        return Some(Answer::RobotsTxtUnreachable(format!(
            "The site's robots.txt could not be read ({reason}), and no earlier copy applies, so the request was not sent."
        )));
    }
    let url = goodfetch_frontier::Url::parse(&leased.crawl_request.url).ok()?;
    let operator_token = settings
        .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
        .as_str();
    if goodfetch_fetch_tier::disallowing(read, operator_token, &url).is_some() {
        return Some(Answer::Refused(SkipReason::RobotsTxt));
    }
    crawl_delay_asked(settings, read)
        .filter(|asked| !waits_out(settings, *asked))
        .map(|_| Answer::Refused(SkipReason::CrawlDelay))
}

/// Whether `leased`'s request is one the robots.txt `read` disallows,
/// fetched anyway since `site_preferences.respect_robots_txt` is off: the
/// Provenance of each Record it commits then marks it `ignored` (ADR
/// 0086). A robots.txt that could not be read disallows nothing here.
pub(super) fn robots_txt_ignored(run: &Run, leased: &Leased, read: Option<&Fetched>) -> bool {
    let Some(settings) = run
        .settings()
        .map(goodfetch_engine::settings::RunSettings::resolved)
    else {
        return false;
    };
    if settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name) != Some(false) {
        return false;
    }
    let (Some(read), Ok(url)) = (
        read,
        goodfetch_frontier::Url::parse(&leased.crawl_request.url),
    ) else {
        return false;
    };
    if matches!(read, Fetched::Unreachable(_)) {
        return false;
    }
    let operator_token = settings
        .value(SITE_PREFERENCES_ROBOTS_TXT_USER_AGENT.name)
        .as_str();
    goodfetch_fetch_tier::disallowing(read, operator_token, &url).is_some()
}

/// The skip reason of a handler's failure that is a `site_preferences_refused`
/// its own navigation raised and it let escape: the request ends `skipped`
/// with that refusal's reason, not `failed`, since the site asked not to be
/// crawled there (ADR 0093).
pub(super) fn escaped_refusal(code: &str, details: &Map<String, Value>) -> Option<SkipReason> {
    if code != SITE_PREFERENCES_REFUSED.name {
        return None;
    }
    let reason = details.get("reason").cloned()?;
    match serde_json::from_value::<SitePreference>(reason).ok()? {
        SitePreference::RobotsTxt => Some(SkipReason::RobotsTxt),
        SitePreference::CrawlDelay => Some(SkipReason::CrawlDelay),
        SitePreference::UseSignals => Some(SkipReason::UseSignals),
    }
}
