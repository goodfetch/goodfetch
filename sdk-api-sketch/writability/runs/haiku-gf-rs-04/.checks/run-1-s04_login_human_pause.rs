//! Logs in to quotes.toscrape.com once with a person at the keyboard, keeps the login as a Saved
//! identity, then reuses it in a headless run to check that the page shows a Logout link.
//!
//! The password is a secret reference (`env:QUOTES_PASSWORD`): the engine resolves it when it types
//! the value, so this program never reads or holds it. Ctrl-C cancels the wait for the person.

use std::future::IntoFuture;
use std::process::ExitCode;

use goodfetch_collie::settings::{browser, session as session_settings};
use goodfetch_collie::{
    Collie, ConnectOptions, Error, ErrorCode, PageCheck, PageMatch, Run, SecretBinding,
    SecretReference, SessionSource, Settings, secret,
};

const LOGIN_URL: &str = "https://quotes.toscrape.com/login";
const HOME_URL: &str = "https://quotes.toscrape.com/";
const USERNAME: &str = "admin";
const USERNAME_FIELD: &str = "input[name=\"username\"]";
const PASSWORD_FIELD: &str = "input[name=\"password\"]";
const LOGOUT_LINK: &str = "a[href=\"/logout\"]";
const LOGIN_LINK: &str = "a[href=\"/login\"]";
const PASSWORD_REFERENCE: &str = "env:QUOTES_PASSWORD";
const PASSWORD_SECRET: &str = "quotes_password";
const SECRET_DOMAIN: &str = "toscrape.com";
const IDENTITY: &str = "quotes-toscrape";

/// A failure already rendered for the person: the error code, the SDK call and the message. The
/// engine never puts a secret's value in any of them.
#[derive(Debug)]
struct Failure(String);

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self(format!(
            "{} in {}: {}",
            error.code().as_str(),
            error.operation(),
            error.message()
        ))
    }
}

/// How the login step with the person ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Completed,
    Cancelled,
}

#[tokio::main]
async fn main() -> ExitCode {
    let collie = match Collie::connect(ConnectOptions::default()).await {
        Ok(collie) => collie,
        Err(error) => return report(error.into()),
    };
    let outcome = run_all(&collie).await;
    let closed = collie.close().await;
    match outcome.and(closed.map_err(Failure::from)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => report(failure),
    }
}

fn report(failure: Failure) -> ExitCode {
    eprintln!("{}", failure.0);
    ExitCode::FAILURE
}

async fn run_all(collie: &Collie) -> Result<(), Failure> {
    match login_with_person(collie).await? {
        Step::Completed => verify_saved_login(collie).await,
        Step::Cancelled => {
            println!("Login cancelled; nothing was saved.");
            Ok(())
        }
    }
}

/// The visible run: the program fills the form, the person checks it and presses Login.
async fn login_with_person(collie: &Collie) -> Result<Step, Failure> {
    let run = collie.start().await?;
    let outcome = fill_and_wait_for_person(&run).await;
    let closed = run.close().await;
    let step = outcome?;
    closed?;
    Ok(step)
}

async fn fill_and_wait_for_person(run: &Run) -> Result<Step, Failure> {
    let reference = PASSWORD_REFERENCE.parse::<SecretReference>()?;
    let login_settings = Settings::new()
        .set(browser::HEADLESS, false)
        .secret(
            PASSWORD_SECRET,
            SecretBinding::reference(reference, [SECRET_DOMAIN]),
        );
    let session = run.new_session().settings(login_settings).await?;
    let page = session.new_page().await?;
    page.goto(LOGIN_URL).await?;
    page.locator(USERNAME_FIELD).fill(USERNAME).await?;
    page.locator(PASSWORD_FIELD)
        .fill(secret(PASSWORD_SECRET))
        .await?;

    let logout = page.locator(LOGOUT_LINK);
    println!(
        "Check the login form in the browser window and press Login. Press Ctrl-C to cancel."
    );
    let pause = page
        .pause()
        .reason("Check the login form and press Login")
        .until(&logout)
        .no_timeout();
    let paused = tokio::select! {
        result = pause.into_future() => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };
    let Some(result) = paused else {
        return Ok(Step::Cancelled);
    };
    let finished = result?;
    println!("The pause ended: {:?}.", finished.resolved_by);

    if !logout.is_visible().await? {
        return Err(Failure(
            "The login did not complete: no Logout link is shown, so nothing was saved."
                .to_owned(),
        ));
    }
    session.save_identity(IDENTITY).await?;
    println!("Saved the login as \"{IDENTITY}\".");
    Ok(Step::Completed)
}

/// The headless run: it reuses the Saved identity and reports an expired login instead of failing
/// on a missing element.
async fn verify_saved_login(collie: &Collie) -> Result<(), Failure> {
    let run = collie.start().await?;
    let outcome = check_saved_login(&run).await;
    let closed = run.close().await;
    outcome?;
    closed?;
    Ok(())
}

async fn check_saved_login(run: &Run) -> Result<(), Failure> {
    let headless = Settings::new()
        .set(browser::HEADLESS, true)
        .set(session_settings::recovery::HUMAN_PAUSE, false)
        .set(session_settings::recovery::AGENT, false)
        .set(
            session_settings::PAGE_CHECK,
            Some(PageCheck::new().login_page(PageMatch::selector(LOGIN_LINK))),
        );
    let session = run
        .new_session()
        .source(SessionSource::saved_identity(IDENTITY))
        .settings(headless)
        .await?;
    let page = session.new_page().await?;
    match page.goto(HOME_URL).await {
        Ok(_) => {}
        Err(error) if error.code() == ErrorCode::LOGGED_OUT => return Err(expired()),
        Err(error) => return Err(error.into()),
    }
    if !page.locator(LOGOUT_LINK).is_visible().await? {
        return Err(expired());
    }
    println!("The kept login works: the page shows a Logout link.");
    Ok(())
}

fn expired() -> Failure {
    Failure(format!(
        "The login kept as \"{IDENTITY}\" has expired: run the login step again."
    ))
}
