//! Snippet for Crawl politeness defaults (#480), on golden scenario 05.

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_sdk_sketch::settings::{politeness, site_preferences};
use goodfetch_sdk_sketch::{
    ConnectOptions, Error, ErrorCode, ExplainedSetting, Goodfetch, CrawlRequestOutcome, CrawlRequestOutcomeKind, Settings,
    SkipReason,
};

/// With nothing set, a crawl obeys robots.txt and keeps one request in flight per domain, one
/// second between starts: scenario 05 sets nothing and is polite to books.toscrape.com (G5).
async fn polite_by_default() -> Result<Vec<ExplainedSetting>, Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    // Each comes from the `Default` layer unless configuration or an Operator policy set it.
    let explained = gf
        .explain_settings(&[
            politeness::domain::CONCURRENCY.key(),
            politeness::domain::DELAY.key(),
            site_preferences::RESPECT_ROBOTS_TXT.key(),
            site_preferences::MAX_CRAWL_DELAY.key(),
        ])
        .await?;
    gf.close().await?;
    Ok(explained)
}

/// The program's own staging site, which it may press harder: an explicit value, recorded in the
/// Audit log and listed in the run's result, and logged once as a warning through `tracing`.
async fn own_staging_site() -> Result<(), Error> {
    let harder = Settings::new()
        .set(politeness::domain::CONCURRENCY, 8_u32)
        .set(politeness::domain::DELAY, Duration::ZERO)
        .set(site_preferences::RESPECT_ROBOTS_TXT, false);
    let gf = Goodfetch::connect(ConnectOptions::default().settings(harder)).await?;

    // A Collie Run's program directs its own navigation, outside Politeness: a limit set on it is
    // refused when the Run opens, never ignored (G3). Rust's one `Settings` type takes every key, so the
    // refusal comes at run time, where TypeScript and Python fail to type-check.
    let refused = gf
        .collie()
        .start()
        .settings(Settings::new().set(politeness::domain::CONCURRENCY, 2_u32))
        .await;
    if let Err(error) = &refused {
        let _ = error.code() == &ErrorCode::SettingNotApplicable;
    }

    // A Collie Run still takes the AI-use signals, which concern any run's results.
    let run = gf
        .collie()
        .start()
        .settings(Settings::new().set(site_preferences::RESPECT_USE_SIGNALS, true))
        .await?;
    let _ = run.id();
    gf.close().await
}

/// An operator crawling under its own bot name, which its user agent carries: a site's robots.txt
/// group for `AcmeBot` binds it too, merged with Foxhound's. Setting the token is recorded.
async fn as_own_bot() -> Result<Vec<ExplainedSetting>, Error> {
    let token = Settings::new().set(site_preferences::ROBOTS_TXT_USER_AGENT, Some(String::from("AcmeBot")));
    let gf = Goodfetch::connect(ConnectOptions::default().settings(token)).await?;
    let explained = gf
        .explain_settings(&[site_preferences::ROBOTS_TXT_USER_AGENT.key()])
        .await?;
    gf.close().await?;
    Ok(explained)
}

/// A page the site asked crawlers to leave is skipped, a value with its reason, never a failure
/// (G1). A program reads outcomes with `CrawlRun::outcomes` (ADR 0102).
fn why_skipped(outcomes: Vec<CrawlRequestOutcome>) -> BTreeMap<String, u32> {
    let mut skipped = BTreeMap::new();
    for outcome in outcomes {
        let CrawlRequestOutcomeKind::Skipped { reason, .. } = outcome.kind else {
            continue;
        };
        let why = match reason {
            SkipReason::RobotsTxt => "robots_txt: disallowed by the host's robots.txt".to_owned(),
            SkipReason::CrawlDelay => {
                "crawl_delay: above site_preferences.max_crawl_delay".to_owned()
            }
            SkipReason::UseSignals => "use_signals: refuses this run's Purpose".to_owned(),
            SkipReason::Unknown(raw) => raw,
            other => format!("{other:?}"),
        };
        *skipped.entry(why).or_insert(0) += 1;
    }
    skipped
}

fn main() {
    let _ = (polite_by_default, own_staging_site, as_own_bot, why_skipped);
}
