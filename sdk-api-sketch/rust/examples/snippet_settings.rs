//! Snippet for Settings in code (#470), on golden scenarios 01, 10 and 12.

use std::time::Duration;

use goodfetch_sdk_sketch::settings::{Viewport, browser, timeout};
use goodfetch_sdk_sketch::{
    ConnectOptions, Error, ErrorCode, ExplainedSetting, Goodfetch, Settings,
};

async fn search_quotes() -> Result<Vec<ExplainedSetting>, Error> {
    // One `Settings` value per scope, inherited beneath it. `engine` is left at `Auto`, so a
    // `GOODFETCH_URL` in the environment may fill it; a value written here would win.
    let options = ConnectOptions::default()
        .settings(Settings::new().set(timeout::NAVIGATION, Duration::from_secs(60)));
    let gf = Goodfetch::connect(options).await?;

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
    let run = gf.collie().start().settings(identity).await?;
    let page = run
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
    gf.close().await?;
    Ok(explained)
}

/// A service reads no project file and passes its own environment map (golden scenario 12).
async fn start_in_service(env: Vec<(String, String)>) -> Result<String, Error> {
    let gf = Goodfetch::connect(ConnectOptions::default().no_project().env(env)).await?;
    match gf
        .collie()
        .start()
        .settings(Settings::new().set(timeout::RUN, None))
        .await
    {
        Ok(run) => Ok(run.id().to_owned()),
        // An Operator policy locked the Run's timeout: refused, never silently ignored.
        Err(error) if error.code() == &ErrorCode::SettingLocked => {
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
