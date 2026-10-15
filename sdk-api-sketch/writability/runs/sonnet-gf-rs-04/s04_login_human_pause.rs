//! Logs in to quotes.toscrape.com once with a person at the keyboard, keeps the login as a Saved
//! identity, and later reuses it headless.
//!
//! Usage: `s04_login_human_pause login` (visible browser, human pause) or
//! `s04_login_human_pause check` (headless, reuses the Saved identity).
//!
//! The password is never held by this program: the engine reads it from the environment variable
//! `QUOTES_PASSWORD` at the moment it types it, and the program only names the reference.

use std::process::ExitCode;

use goodfetch_collie::settings::{browser, session};
use goodfetch_collie::{
    Collie, ConnectOptions, Error, ErrorCode, PageCheck, PageMatch, SecretBinding, SecretReference,
    SessionSource, Settings, secret,
};

const IDENTITY_NAME: &str = "quotes-toscrape";
const USERNAME: &str = "admin";
const LOGIN_URL: &str = "https://quotes.toscrape.com/login";
const HOME_URL: &str = "https://quotes.toscrape.com/";
const PASSWORD_REFERENCE: &str = "env:QUOTES_PASSWORD";
const PASSWORD_PLACEHOLDER: &str = "quotes_password";
const LOGOUT_LINK: &str = "a[href='/logout']";
const LOGIN_LINK: &str = "a[href='/login']";

/// How the interactive login ended.
enum LoginOutcome {
    Saved,
    Cancelled,
}

/// What the headless check found.
enum CheckOutcome {
    LoggedIn,
    Expired,
}

#[tokio::main]
async fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "login" => match Collie::scope(ConnectOptions::default(), async |collie: &Collie| {
            login(collie).await
        })
        .await
        {
            Ok(LoginOutcome::Saved) => {
                println!("Logged in; the login is kept as the Saved identity {IDENTITY_NAME:?}.");
                ExitCode::SUCCESS
            }
            Ok(LoginOutcome::Cancelled) => {
                println!("Login cancelled; nothing was saved.");
                ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("Login failed: {}", describe(&error));
                ExitCode::FAILURE
            }
        },
        "check" => match Collie::scope(ConnectOptions::default(), async |collie: &Collie| {
            check(collie).await
        })
        .await
        {
            Ok(CheckOutcome::LoggedIn) => {
                println!("The page shows a Logout link: the kept login still works.");
                ExitCode::SUCCESS
            }
            Ok(CheckOutcome::Expired) => {
                println!(
                    "The kept login {IDENTITY_NAME:?} has expired: the page shows no Logout link. \
                     Run the login step again."
                );
                ExitCode::FAILURE
            }
            Err(error) if error.code() == ErrorCode::LOGGED_OUT => {
                println!(
                    "The kept login {IDENTITY_NAME:?} has expired (logged out). Run the login step again."
                );
                ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("Check failed: {}", describe(&error));
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: s04_login_human_pause <login|check>");
            ExitCode::from(2)
        }
    }
}

/// An error line that names the code and message only; no secret value ever reaches an error.
fn describe(error: &Error) -> String {
    format!("{}: {}", error.code().as_str(), error.message())
}

/// First run: a visible browser, the username typed by the program, the password typed by the
/// engine from the secret, then a human pause until the person has pressed Login.
async fn login(collie: &Collie) -> Result<LoginOutcome, Error> {
    let reference: SecretReference = PASSWORD_REFERENCE.parse()?;
    let settings = Settings::new()
        .set(browser::HEADLESS, false)
        .secret(
            PASSWORD_PLACEHOLDER,
            SecretBinding::reference(reference, ["toscrape.com"]),
        );

    let run = collie.start().await?;
    let session = run.new_session().settings(settings).await?;
    let page = session.new_page().await?;

    page.goto(LOGIN_URL).await?;
    page.locator("#username").fill(USERNAME).await?;
    page.locator("#password")
        .fill(secret(PASSWORD_PLACEHOLDER))
        .await?;

    // No time limit; ends when the Logout link shows (the person pressed Login), and Ctrl-C cancels.
    let logout = page.locator(LOGOUT_LINK);
    let pause = page
        .pause()
        .reason("Check the login form and press Login")
        .until(&logout)
        .no_timeout();
    let outcome = tokio::select! {
        result = async { pause.await } => {
            result?;
            true
        }
        _ = tokio::signal::ctrl_c() => false,
    };

    if !outcome {
        let _report = run.stop().await?;
        return Ok(LoginOutcome::Cancelled);
    }

    session.save_identity(IDENTITY_NAME).await?;
    let _report = run.finish().await?;
    Ok(LoginOutcome::Saved)
}

/// Second run: headless, the Saved identity as the Session source.
async fn check(collie: &Collie) -> Result<CheckOutcome, Error> {
    let page_check = PageCheck::new().login_page(PageMatch::selector("form[action='/login']"));
    let settings = Settings::new()
        .set(browser::HEADLESS, true)
        .set(session::PAGE_CHECK, page_check);

    let run = collie.start().await?;
    let session = run
        .new_session()
        .source(SessionSource::saved_identity(IDENTITY_NAME))
        .settings(settings)
        .await?;
    let page = session.new_page().await?;

    page.goto(HOME_URL).await?;
    let logout_links = page.locator(LOGOUT_LINK).count().await?;
    let login_links = page.locator(LOGIN_LINK).count().await?;
    let _report = run.finish().await?;

    if logout_links > 0 {
        Ok(CheckOutcome::LoggedIn)
    } else if login_links > 0 {
        Ok(CheckOutcome::Expired)
    } else {
        // Neither link: the page is not the site's usual home page.
        Ok(CheckOutcome::Expired)
    }
}
