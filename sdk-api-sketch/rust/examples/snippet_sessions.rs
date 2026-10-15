//! Snippet for Sessions, logins, the Human pause and secrets in code (#476), on golden scenario 04.

use goodfetch_sdk_sketch::settings::{browser, proxy, session};
use goodfetch_sdk_sketch::{
    AriaRole, ConnectOptions, CookieInput, Error, ErrorCode, ErrorDetails, Goodfetch, PageCheck,
    PageMatch, SecretBinding, SecretReference, SessionSource, Settings, secret,
};

/// First run: a person at the keyboard logs in once, and the login is kept under a name.
async fn log_in_once() -> Result<(), Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    // A Secret placeholder: the program names it, and the engine types its value, only into a field
    // on toscrape.com.
    let password: SecretReference = "env:QUOTES_PASSWORD".parse()?;
    let secrets = Settings::new().secret(
        "password",
        SecretBinding::reference(password, ["toscrape.com"]),
    );
    let run = gf.collie().start().settings(secrets).await?;
    // A visible window for the person; headless, they would act in the Live view instead.
    let session = run
        .new_session()
        .settings(Settings::new().set(browser::HEADLESS, false))
        .await?;
    let page = session.new_page().await?;
    page.goto("https://quotes.toscrape.com/login").await?;
    page.get_by_label("Username").fill("alice").await?;
    page.get_by_label("Password")
        .fill(secret("password"))
        .await?;
    // The Human pause: no clock runs while it waits, and it ends once the Logout link shows.
    page.pause()
        .reason("Check the form, then press Login")
        .until(&page.get_by_role(AriaRole::Link.name("Logout")))
        .no_timeout()
        .await?;
    // Kept encrypted per install, never as a clear-text file (ADRs 0035, 0056).
    session.save_identity("quotes").await?;
    gf.close().await
}

/// Second run: headless and unattended, it reuses the login and says when it has expired.
async fn reuse_login() -> Result<bool, Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    let run = gf.collie().start().await?;
    // A page showing the Login link is a lost login; nobody is there to log in again.
    let check = Settings::new()
        .set(
            session::PAGE_CHECK,
            Some(PageCheck::new().login_page(PageMatch::selector("a[href='/login']"))),
        )
        .set(session::recovery::HUMAN_PAUSE, false);
    let session = run
        .new_session()
        .source(SessionSource::saved_identity("quotes"))
        .settings(check)
        .await?;
    let page = session.new_page().await?;
    match page.goto("https://quotes.toscrape.com/").await {
        Ok(_) => {
            page.get_by_role(AriaRole::Link.name("Logout"))
                .is_visible()
                .await
        }
        Err(error) if error.code() == &ErrorCode::LoggedOut => {
            if let Some(ErrorDetails::LoggedOut(details)) = error.details() {
                let _ = details.steps.len(); // each recovery step and why it did not log back in
            }
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// The scripted login step: the program logs the Session back in when recovery asks.
async fn answer_logins(session: goodfetch_sdk_sketch::Session) -> Result<(), Error> {
    let mut logins = session.logins();
    while let Some(request) = logins.next().await {
        let request = request?;
        request
            .page()
            .get_by_label("Username")
            .fill("alice")
            .await?;
        request
            .page()
            .get_by_label("Password")
            .fill(secret("password"))
            .await?;
        request
            .page()
            .get_by_role(AriaRole::Button.name("Login"))
            .click()
            .await?;
        // Dropping the request hands the page to the Page check.
    }
    Ok(())
}

/// `caller:` references, answered from the program's own store at the moment of use.
async fn answer_secrets(gf: Goodfetch) -> Result<(), Error> {
    let mut requests = gf.secret_requests();
    while let Some(request) = requests.next().await {
        let request = request?;
        let value = read_vault(request.name());
        request.answer(value);
    }
    Ok(())
}

/// Cookies and a proxy on the Run's default Session.
async fn cookies_and_proxy(gf: &Goodfetch) -> Result<usize, Error> {
    let proxy_settings = Settings::new()
        .set(
            proxy::SERVER,
            Some(String::from("http://proxy.example:3128")),
        )
        .set(proxy::USERNAME, String::from("crawler"))
        .set(
            proxy::PASSWORD,
            "caller:proxy_password".parse::<SecretReference>()?,
        )
        .secret(
            "token",
            SecretBinding::reference("caller:api_token".parse()?, ["toscrape.com"]),
        );
    let run = gf.collie().start().settings(proxy_settings).await?;
    // The Run's default Session, which every `run.new_page()` shares.
    let page = run.new_page().await?;
    let session = page.session();
    session
        .add_cookies([CookieInput::for_url(
            "theme",
            "dark",
            "https://quotes.toscrape.com",
        )])
        .await?;
    page.goto("https://quotes.toscrape.com/").await?;
    page.request()
        .get("https://quotes.toscrape.com/api/quotes")
        .header("authorization", secret("token"))
        .await?;
    let cookies = session.cookies(&["https://quotes.toscrape.com"]).await?;
    for identity in gf.identities().list().await? {
        let _ = (identity.name, identity.cookies_expire);
    }
    Ok(cookies.len())
}

/// Stands in for the program's own secret store.
fn read_vault(name: &str) -> String {
    let _ = name;
    unimplemented!()
}

fn main() {
    let _ = (
        log_in_once,
        reuse_login,
        answer_logins,
        answer_secrets,
        cookies_and_proxy,
    );
}
