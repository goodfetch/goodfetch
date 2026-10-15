//! Snippet for Settings in code (#470), on golden scenarios 01, 10 and 12; the last two are
//! Foxhound's, shown on Collie until Crawl definition and handler context (#477) gives crawls
//! their start verb.

use std::time::Duration;

use goodfetch_foxhound::settings::{Viewport, browser, timeout};
use goodfetch_foxhound::{
    ConnectOptions, Error, ErrorCode, ExplainedSetting, Foxhound, Settings,
};

async fn search_quotes() -> Result<Vec<ExplainedSetting>, Error> {
    // One `Settings` value per scope, inherited beneath it. `engine` is left at `Auto`, so a
    // `GOODFETCH_URL` in the environment may fill it; a value written here would win.
    let options = ConnectOptions::default()
        .settings(Settings::new().set(timeout::NAVIGATION, Duration::from_secs(60)));
    let foxhound = Foxhound::connect(options).await?;

    // A Session's identity is set where it opens, or above it, never on a page (ADR 0021).
    let identity = Settings::new()
        .set(
            browser::VIEWPORT,
            Viewport {
                width: 1280,
                height: 720,
            },
        )
        .set(browser::LOCALE, String::from("en-GB"));
    let run = foxhound.start().await?;
    let session = run.new_session().settings(identity).await?;
    let page = session
        .new_page()
        .settings(Settings::new().set(timeout::ACTION, Duration::from_secs(10)))
        .await?;
    // A per-call timeout applies to that call alone; `None` would clear a scope's setting instead.
    page.goto("https://quotes.toscrape.com/search.aspx")
        .timeout(Duration::from_secs(20))
        .await?;

    // What the page actually runs with, and which layer set each value (golden scenario 10 asserts on it).
    let explained = page
        .explain_settings(&[
            timeout::ACTION.key(),
            timeout::NAVIGATION.key(),
            browser::LOCALE.key(),
        ])
        .await?;
    foxhound.close().await?;
    Ok(explained)
}

/// A service reads no project file and passes its own environment map (golden scenario 12).
async fn start_in_service(env: Vec<(String, String)>) -> Result<String, Error> {
    // Every Run on this connection may last four hours unless it says otherwise.
    let defaults = Settings::new().set(timeout::RUN, Duration::from_secs(4 * 60 * 60));
    let options = ConnectOptions::default()
        .no_project()
        .env(env)
        .settings(defaults);
    let foxhound = Foxhound::connect(options).await?;
    // A Run's own timeout is its start's, one spelling (S2).
    match foxhound.start().no_timeout().await {
        Ok(run) => Ok(run.id().to_owned()),
        // An Operator policy locked the Run's timeout: refused, never silently ignored.
        Err(error) if error.code() == ErrorCode::SETTING_LOCKED => {
            tracing_like_error(&error);
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// Stands in for `tracing::error!`, which the sketch does not depend on.
fn tracing_like_error(error: &Error) {
    let _ = error.limit();
}

fn main() {
    let _ = (search_quotes, start_in_service);
}
