//! Golden scenario 04, "Log in once with a person, reuse the login headless" (Collie, Rust).
//!
//! `cargo run --example s04_login_human_pause` logs in with a person; `-- --reuse` confirms the
//! kept login headless.

use std::future::IntoFuture;

use goodfetch_collie::settings::{browser, session};
use goodfetch_collie::{
    AriaRole, Collie, ConnectOptions, ElementState, Error, ErrorCode, PageCheck, PageMatch,
    SecretBinding, SessionSource, Settings, secret,
};

/// The name the login is kept under.
const IDENTITY: &str = "quotes";

/// What the second run found.
#[derive(Debug, PartialEq, Eq)]
enum Login {
    /// The page shows the Logout link.
    LoggedIn,
    /// The kept login no longer logs in.
    Expired,
}

/// First run: a person checks the form and presses Login; the login is kept as a Saved identity.
/// Returns `false` when the person's wait was cancelled with Ctrl+C, and nothing was kept.
async fn log_in_once(username: &str) -> Result<bool, Error> {
    Collie::scope(ConnectOptions::default(), async |collie| {
        // The Secret placeholder `password`: the engine reads QUOTES_PASSWORD and types it only
        // into a field on toscrape.com. The program never holds it, and every log, trace,
        // screenshot and returned value shows `%password%` instead (ADR 0092).
        let password = SecretBinding::reference("env:QUOTES_PASSWORD".parse()?, ["toscrape.com"]);
        let run = collie
            .start()
            .settings(Settings::new().secret("password", password))
            .await?;
        let session = run
            .new_session()
            .settings(Settings::new().set(browser::HEADLESS, false))
            .await?;
        let page = session.new_page().await?;
        page.goto("https://quotes.toscrape.com/login").await?;
        page.get_by_label("Username").fill(username).await?;
        // The page's "Password" label points at the username field (`for="username"`): the field
        // is found by its id.
        page.locator("#password").fill(secret("password")).await?;
        // The Human pause: no time limit and no other clock runs; it ends once the Logout link
        // shows, and a person's "Decline" fails with `human_pause_declined`. Closing the window
        // fails with `call_cancelled`, whose `CallCancelled` details say `closed: Page` (ADR 0113).
        let logout = page.get_by_role(AriaRole::Link.name("Logout"));
        let pause = page
            .pause()
            .reason("Check the form, then press Login")
            .until(&logout)
            .no_timeout();
        // Ctrl+C drops the pause's future, which cancels it in the engine (ADR 0080).
        tokio::select! {
            ended = pause.into_future() => {
                ended?;
            }
            _ = tokio::signal::ctrl_c() => return Ok(false),
        }
        // Kept encrypted in the install's store under that name, never as a clear-text file.
        session.save_identity(IDENTITY).await?;
        let _ = run.finish().await?;
        Ok(true)
    })
    .await
}

/// Second run, headless by default: `Expired` when the kept login no longer logs in.
async fn confirm_login() -> Result<Login, Error> {
    Collie::scope(ConnectOptions::default(), async |collie| {
        let run = collie.start().await?;
        // A page showing the Login link is a lost login, and no one is there to log in again.
        let login_page = PageCheck::new().login_page(PageMatch::selector("a[href='/login']"));
        let unattended = Settings::new()
            .set(session::PAGE_CHECK, Some(login_page))
            .set(session::recovery::HUMAN_PAUSE, false)
            .set(session::recovery::AGENT, false);
        let session = run
            .new_session()
            .source(SessionSource::saved_identity(IDENTITY))
            .settings(unattended)
            .await?;
        let page = session.new_page().await?;
        let logout = page.get_by_role(AriaRole::Link.name("Logout"));
        let shown = async {
            page.goto("https://quotes.toscrape.com/").await?;
            logout.wait_for(ElementState::Visible).await
        };
        let state = match shown.await {
            Ok(()) => Login::LoggedIn,
            // An outcome, named by the Page check, not a missing element.
            Err(error) if error.code() == ErrorCode::LOGGED_OUT => Login::Expired,
            Err(error) => return Err(error),
        };
        let _ = run.finish().await?;
        Ok(state)
    })
    .await
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    if std::env::args().any(|arg| arg == "--reuse") {
        match confirm_login().await? {
            Login::LoggedIn => println!("Logged in: the page shows Logout."),
            Login::Expired => println!("The kept login has expired: log in again."),
        }
    } else if !log_in_once("alice").await? {
        println!("Cancelled: no login was kept.");
    }
    Ok(())
}
