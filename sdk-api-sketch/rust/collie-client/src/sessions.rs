//! Sessions, logins, the Human pause and secrets in code (ADR 0092, decided in #476).
//!
//! - A Session opens from its Session source (ADR 0056): `run.new_session().source(..)`.
//!   `run.new_page()` opens pages in the Run's default Session, reached as `page.session()`.
//! - A login is kept as a Saved identity ([`Session::save_identity`]), never as a clear-text file;
//!   a lost one is found by the Session's Page check and recovered by ADR 0006's ladder, or fails
//!   with `logged_out`. The ladder's scripted login step is a closure given where the Session
//!   opens, as a crawl takes its own.
//! - [`Page::pause`] is the Human pause: no clock runs while a person acts, in the real window or
//!   the Live view, and it ends when its condition holds or the person says so.
//! - A program names a secret by a [`SecretPlaceholder`], [`secret`] or `%name%` in an AI
//!   instruction, which `secrets.<name>` binds to a [`SecretReference`] and the domains allowed to
//!   receive it. The engine types or sends the value; the program never holds it (ADRs 0014, 0015),
//!   except what it answers to a `caller:` reference through the connection's `secret_requests`.

use std::fmt;
use std::future::IntoFuture;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use crate::lifecycle::{Page, Session};
use crate::page::{Locator, TextMatch};
use crate::results::Error;
use crate::settings::Settings;
use crate::time::{Call, CallFuture, UrlMatcher};

// Secrets.

/// A Secret reference, `provider:rest`: `env:QUOTES_PASSWORD`, `keychain:goodfetch/portal`,
/// `op://vault/item/field`, or `caller:portal` for the connection's `secret_requests`. It resolves in
/// the engine, at the moment of use: `env:` reads the engine's environment, which for the linked
/// engine is the program's (ADR 0083). Parsing checks only its form; the engine refuses an unknown
/// provider with `setting_invalid`, so a raw password is never sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretReference(String);

impl SecretReference {
    /// The reference as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SecretReference {
    type Err = Error;

    /// Fails with `setting_invalid` when `reference` has no `provider:` prefix.
    fn from_str(reference: &str) -> Result<Self, Self::Err> {
        let _ = reference;
        unimplemented!()
    }
}

/// A Secret placeholder: the name by which the program asks for a secret to be typed or sent. It
/// holds no value; `secrets.<name>` binds it, and the engine refuses an unbound one with
/// `secret_unbound`. It is not serializable, so no serde value carries it into a page function or
/// a fulfilled body: a JSON body takes one through [`crate::ApiRequest::json_secret`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretPlaceholder {
    name: String,
}

impl SecretPlaceholder {
    /// The placeholder's name, which `%name%` also names in an AI instruction.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The Secret placeholder `name`.
pub fn secret(name: &str) -> SecretPlaceholder {
    SecretPlaceholder {
        name: name.to_owned(),
    }
}

/// Text a call types or sends: written by the program, or a Secret placeholder the engine resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextInput {
    /// Text as written.
    Text(String),
    /// A Secret placeholder.
    Secret(SecretPlaceholder),
}

impl From<&str> for TextInput {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for TextInput {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<SecretPlaceholder> for TextInput {
    fn from(placeholder: SecretPlaceholder) -> Self {
        Self::Secret(placeholder)
    }
}

/// `secrets.<name>`: what a placeholder stands for and where it may go, set with
/// [`Settings::secret`]. `domains` are registrable domains (ADR 0004), `*` only when written, and
/// required; they are checked against the frame holding the target field, or a request's URL. Set
/// by the program's own code, configuration or an Operator policy, at the connection, a Run or a
/// Session, nearest winning; a Recipe declares the names it needs and never binds them, and a
/// Client acting for a model never sets one (`setting_locked`).
#[derive(Debug, Clone)]
pub struct SecretBinding {
    _private: (),
}

impl SecretBinding {
    /// Bound to a Secret reference.
    pub fn reference(
        reference: SecretReference,
        domains: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let _ = (
            reference,
            domains.into_iter().map(Into::into).collect::<Vec<String>>(),
        );
        unimplemented!()
    }

    /// Bound to one of ADR 0006's email or SMS code sources, configured by name; a Human pause takes
    /// over when no code arrives.
    pub fn code_source(source: &str, domains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let _ = (
            source,
            domains.into_iter().map(Into::into).collect::<Vec<String>>(),
        );
        unimplemented!()
    }

    /// The reference holds a TOTP seed: the engine types the current code. A vault that makes codes
    /// itself needs no flag.
    pub fn totp(self) -> Self {
        unimplemented!()
    }
}

impl Settings {
    /// Binds the Secret placeholder `name` (`secrets.<name>`) for this scope.
    pub fn secret(self, name: &str, binding: SecretBinding) -> Self {
        let _ = (name, binding);
        unimplemented!()
    }
}

/// A `caller:` reference the engine asks the program to resolve, at the moment of use (ADR 0015's
/// caller provider). Dropped unanswered, the call that needed it fails with `secret_unavailable`.
#[derive(Debug)]
pub struct SecretRequest {
    _private: (),
}

impl SecretRequest {
    /// The reference's name after `caller:`.
    pub fn name(&self) -> &str {
        unimplemented!()
    }

    /// The Run asking.
    pub fn run_id(&self) -> &str {
        unimplemented!()
    }

    /// The registrable domain about to receive it; `None` for a credential the engine uses itself.
    pub fn domain(&self) -> Option<&str> {
        unimplemented!()
    }

    /// Sends the value straight to the engine; the crate keeps no copy and logs nothing.
    pub fn answer(self, value: String) {
        let _ = value;
        unimplemented!()
    }
}

// Where a Session's login comes from (ADR 0056).

/// A browser family, as ADR 0056 names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BrowserName {
    /// Google Chrome.
    Chrome,
    /// Microsoft Edge.
    Edge,
    /// Brave.
    Brave,
    /// Chromium.
    Chromium,
    /// Firefox.
    Firefox,
    /// Safari.
    Safari,
}

/// The format of a state file; detected when left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StateFileFormat {
    /// Playwright's `storageState`.
    StorageState,
    /// A Netscape `cookies.txt`.
    CookiesTxt,
    /// A JSON array of CDP `Network.Cookie` objects.
    CdpCookies,
    /// A session export bundle (ADR 0015).
    SessionExport,
}

/// `session.source`: exactly one kind (ADR 0056). `client_browser` is the TypeScript and Python
/// relay's; the crate reaches a browser through `CdpUrl`. Only the program's own code,
/// configuration, an Operator policy or a person sets it; a Recipe never.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SessionSource {
    /// A Saved identity (ADR 0035).
    SavedIdentity {
        /// Its name.
        name: String,
    },
    /// A state file, read on the program's machine and sent over the connection.
    StateFile {
        /// The file.
        path: PathBuf,
        /// Its format; detected when `None`.
        format: Option<StateFileFormat>,
        /// Opens a session export bundle (ADR 0015).
        key: Option<SecretReference>,
        /// The domains kept; every one the file holds when empty.
        domains: Vec<String>,
    },
    /// A raw `Cookie:` header.
    CookieHeader {
        /// The header.
        header: String,
        /// The domain it applies to.
        domain: String,
    },
    /// A Profile directory, launched in place.
    ProfileDirectory {
        /// The directory.
        path: PathBuf,
        /// The family that wrote it.
        browser: BrowserName,
        /// The Chromium profile folder inside it.
        chromium_profile: Option<String>,
        /// The registrable domains the Session may reach.
        domains: Vec<String>,
    },
    /// A browser installed on this machine.
    InstalledBrowser {
        /// The browser.
        browser: BrowserName,
        /// The Chromium profile folder.
        chromium_profile: Option<String>,
        /// The registrable domains imported.
        domains: Vec<String>,
    },
    /// The person's running browser, through its consented remote debugging.
    RunningBrowser {
        /// The browser.
        browser: BrowserName,
        /// The registrable domains the Session may reach.
        domains: Vec<String>,
    },
    /// Any browser reachable over CDP.
    CdpUrl {
        /// The endpoint.
        url: String,
        /// Header values, each a Secret reference.
        headers: Vec<(String, SecretReference)>,
        /// The registrable domains the Session may reach.
        domains: Vec<String>,
        /// Nothing else drives this browser, so secrets may be typed into it (ADR 0056). When
        /// `false`, the program awaits each Collie call before its other code acts on the page, and an
        /// AI verb whose document a navigation it did not start replaces fails with
        /// `page_navigated_during_call` (ADR 0114).
        exclusive: bool,
    },
}

impl SessionSource {
    /// The Saved identity `name`.
    pub fn saved_identity(name: &str) -> Self {
        Self::SavedIdentity {
            name: name.to_owned(),
        }
    }
}

/// What a page looks like, matched in the engine: every part given must match.
#[derive(Debug, Default)]
#[must_use]
pub struct PageMatch {
    _private: (),
}

impl PageMatch {
    /// A CSS selector, or XPath starting with `//`, as `locator()` takes it.
    pub fn selector(selector: &str) -> Self {
        let _ = selector;
        unimplemented!()
    }

    /// A URL glob or regular expression; a predicate fails with `setting_invalid`.
    pub fn url(self, url: impl Into<UrlMatcher>) -> Self {
        let _ = url.into();
        unimplemented!()
    }

    /// Text on the page.
    pub fn text(self, text: impl Into<TextMatch>) -> Self {
        let _ = text.into();
        unimplemented!()
    }
}

/// `session.page_check` (ADR 0006): every page of the Session is judged against it. A login page
/// is a lost login, which recovery tries to bring back; a page that is neither expected nor a login
/// page fails with `page_check_failed`. Without one, the built-in detector applies and pages are
/// weakly checked.
#[derive(Debug, Default)]
#[must_use]
pub struct PageCheck {
    _private: (),
}

impl PageCheck {
    /// No declaration yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// What a login page looks like.
    pub fn login_page(self, page: PageMatch) -> Self {
        let _ = page;
        unimplemented!()
    }

    /// What an expected page looks like.
    pub fn expected_page(self, page: PageMatch) -> Self {
        let _ = page;
        unimplemented!()
    }
}

/// `session.http_credentials`: sent only to a listed domain that challenges, never on a browser
/// other code may read. A Recipe and a Client acting for a model never set it.
#[derive(Debug, Clone)]
pub struct HttpCredentials {
    /// The user name.
    pub username: String,
    /// The password's reference.
    pub password: SecretReference,
    /// The registrable domains it may go to.
    pub domains: Vec<String>,
}

/// A permission, as `browser.permissions` grants it, under Playwright's names. A sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Permission {
    /// `clipboard-read`.
    ClipboardRead,
    /// `clipboard-write`.
    ClipboardWrite,
    /// `geolocation`.
    Geolocation,
    /// `notifications`.
    Notifications,
    /// `camera`.
    Camera,
    /// `microphone`.
    Microphone,
    /// `local-network-access`.
    LocalNetworkAccess,
}

// Cookies and Saved identities.

/// A cookie's `SameSite` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SameSite {
    /// `Strict`.
    Strict,
    /// `Lax`.
    Lax,
    /// `None`.
    None,
}

/// A cookie, with Playwright's fields.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Cookie {
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
    /// Its domain.
    pub domain: String,
    /// Its path.
    pub path: String,
    /// When it expires; `None` for a session cookie.
    pub expires: Option<SystemTime>,
    /// `HttpOnly`.
    pub http_only: bool,
    /// `Secure`.
    pub secure: bool,
    /// `SameSite`.
    pub same_site: SameSite,
    /// Its partition key, if partitioned.
    pub partition_key: Option<String>,
}

/// A cookie to add: by URL, or by domain and path, as Playwright takes it.
#[derive(Debug, Clone)]
#[must_use]
pub struct CookieInput {
    _private: (),
}

impl CookieInput {
    /// A cookie for `url`.
    pub fn for_url(name: &str, value: &str, url: &str) -> Self {
        let _ = (name, value, url);
        unimplemented!()
    }

    /// A cookie for `domain` and `path`.
    pub fn for_domain(name: &str, value: &str, domain: &str, path: &str) -> Self {
        let _ = (name, value, domain, path);
        unimplemented!()
    }

    /// When it expires; a session cookie otherwise.
    pub fn expires(self, at: SystemTime) -> Self {
        let _ = at;
        unimplemented!()
    }

    /// `HttpOnly`.
    pub fn http_only(self) -> Self {
        unimplemented!()
    }

    /// `Secure`.
    pub fn secure(self) -> Self {
        unimplemented!()
    }

    /// `SameSite`.
    pub fn same_site(self, same_site: SameSite) -> Self {
        let _ = same_site;
        unimplemented!()
    }
}

/// Which cookies [`Session::clear_cookies`] removes: all of them unless narrowed.
#[derive(Debug, Default)]
#[must_use]
pub struct ClearCookies {
    _private: (),
}

impl ClearCookies {
    /// Every cookie.
    pub fn all() -> Self {
        Self::default()
    }

    /// Only these names.
    pub fn name(self, name: impl Into<TextMatch>) -> Self {
        let _ = name.into();
        unimplemented!()
    }

    /// Only these domains.
    pub fn domain(self, domain: impl Into<TextMatch>) -> Self {
        let _ = domain.into();
        unimplemented!()
    }

    /// Only these paths.
    pub fn path(self, path: impl Into<TextMatch>) -> Self {
        let _ = path.into();
        unimplemented!()
    }
}

/// A Saved identity (ADR 0035).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SavedIdentity {
    /// Its name.
    pub name: String,
    /// Where its login came from: an import's source kind, or `session` for a login made in a Session.
    pub source: String,
    /// The Access mode it was saved under.
    pub access_mode: String,
    /// When it was saved.
    pub saved_at: SystemTime,
    /// The earliest cookie expiry per registrable domain: cookies expire then, which says nothing
    /// of the site's own session (ADR 0056).
    pub cookies_expire: Vec<(String, SystemTime)>,
    /// An import on direct egress, or a login saved from a Session that came from one: usable only
    /// on this machine (ADR 0056).
    pub bound_to_machine: bool,
}

/// What encrypts an exported Saved identity: ADR 0015's `age` bundle.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ExportKey {
    /// An `age` public key.
    Recipient(String),
    /// A passphrase, by reference.
    Passphrase(SecretReference),
}

/// The install's Saved identities. No SDK writes a clear-text `storageState` (ADR 0056).
#[derive(Debug, Clone)]
pub struct Identities {
    _private: (),
}

impl Identities {
    /// Every Saved identity of the install.
    pub fn list(&self) -> Call<Vec<SavedIdentity>> {
        unimplemented!()
    }

    /// Imports a login into a Saved identity, as `goodfetch-collie identity import` does.
    pub fn import(&self, name: &str, source: SessionSource) -> Call<SavedIdentity> {
        let _ = (name, source);
        unimplemented!()
    }

    /// Deletes a Saved identity.
    pub fn delete(&self, name: &str) -> Call<()> {
        let _ = name;
        unimplemented!()
    }

    /// Writes ADR 0015's encrypted bundle, for another machine. A bundle of one bound to its machine
    /// keeps the binding: used on another machine, it is refused with `session_bound_to_machine`.
    pub fn export(&self, name: &str, path: impl Into<PathBuf>, key: ExportKey) -> Call<()> {
        let _ = (name, path.into(), key);
        unimplemented!()
    }
}

/// A Session's cookies and keeping its identity, beside its page members (ADR 0085).
impl Session {
    /// Playwright's `context.cookies()`, for `urls` or every URL when empty. Offered in the SDKs
    /// only, never on the CLI, MCP or Code mode (ADRs 0011, 0014).
    pub fn cookies(&self, urls: &[&str]) -> Call<Vec<Cookie>> {
        let _ = urls;
        unimplemented!()
    }

    /// Kept as session state, encrypted in run state and never logged (ADR 0015). Outside the
    /// source's `domains`, refused.
    pub fn add_cookies(&self, cookies: impl IntoIterator<Item = CookieInput>) -> Call<()> {
        let _ = cookies.into_iter().collect::<Vec<_>>();
        unimplemented!()
    }

    /// Removes cookies.
    pub fn clear_cookies(&self, filter: ClearCookies) -> Call<()> {
        let _ = filter;
        unimplemented!()
    }

    /// Keeps this Session's identity, cookies and storage as a Saved identity (ADR 0035), replacing
    /// one of that name, recorded in the Audit log. A Session whose cookies live outside run state
    /// (a Borrowed browser, a Profile directory used in place) fails with `session_not_savable`.
    pub fn save_identity(&self, name: &str) -> Call<SavedIdentity> {
        let _ = name;
        unimplemented!()
    }
}

impl Call<Session> {
    /// `session.source` for this Session alone (ADR 0056); by default the Run's or the
    /// connection's. Set here, `session::SOURCE` in `settings` fails with `setting_not_applicable`
    /// (S2).
    pub fn source(self, source: SessionSource) -> Self {
        let _ = source;
        unimplemented!()
    }

    /// ADR 0006's scripted login step: called with the page whose call met the login page, inside
    /// that call's future (ADR 0081), which gives up the page's turn meanwhile. The Page check
    /// judges the page it leaves; it counts toward the two recoveries a page gets, and an error it
    /// returns is chained to `logged_out`. A crawl takes its own the same way.
    pub fn login<F>(self, login: F) -> Self
    where
        F: AsyncFn(Page) -> Result<(), Error> + Send + Sync + 'static,
    {
        let _ = login;
        unimplemented!()
    }
}

// The Human pause.

/// What ended a Human pause.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolvedBy {
    /// Its condition held.
    Condition,
    /// A person said "Done".
    Person,
    /// A value this crate does not know, as received.
    Unknown(String),
}

/// How a Human pause ended. A person's "Decline" fails with `human_pause_declined` instead.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HumanPauseResult {
    /// What ended it.
    pub resolved_by: ResolvedBy,
    /// The person who answered, as the Console or the CLI knows them; `None` for a condition.
    pub person: Option<String>,
    /// How long it waited.
    pub waited: Duration,
}

/// A Human pause on a page, not sent until awaited.
#[must_use = "a pause does nothing until it is awaited"]
pub struct Pause {
    _private: (),
}

/// What ends a pause by itself: a locator becoming visible, or a URL matching as `wait_for_url`
/// takes it (ADR 0092).
#[non_exhaustive]
pub enum PauseUntil {
    /// The locator is visible.
    Visible(Locator),
    /// The page's URL matches.
    Url(UrlMatcher),
}

impl fmt::Debug for PauseUntil {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Visible(_) => f.write_str("Visible(..)"),
            Self::Url(url) => f.debug_tuple("Url").field(url).finish(),
        }
    }
}

impl From<&Locator> for PauseUntil {
    fn from(locator: &Locator) -> Self {
        Self::Visible(locator.clone())
    }
}

impl From<UrlMatcher> for PauseUntil {
    fn from(url: UrlMatcher) -> Self {
        Self::Url(url)
    }
}

impl From<&str> for PauseUntil {
    fn from(glob: &str) -> Self {
        Self::Url(UrlMatcher::from(glob))
    }
}

impl fmt::Debug for Pause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pause(..)")
    }
}

impl Pause {
    /// Shown in the Console's "Needs you", the Notification and the Live view, and logged at info.
    pub fn reason(self, reason: &str) -> Self {
        let _ = reason;
        unimplemented!()
    }

    /// Ends the pause by itself once a locator is visible or the page's URL matches; a person's
    /// "Done" is accepted only then. One name for both shapes, as TypeScript's `until` (ADR 0099).
    pub fn until(self, until: impl Into<PauseUntil>) -> Self {
        let _ = until.into();
        unimplemented!()
    }

    /// At most this long; by default `timeout.human_pause`, 24 hours.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// Waits without limit.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }
}

impl IntoFuture for Pause {
    type Output = Result<HumanPauseResult, Error>;
    type IntoFuture = CallFuture<HumanPauseResult>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A page's `localStorage` or `sessionStorage` for its current origin, Playwright's `WebStorage`.
/// Offered in the SDKs only (ADRs 0011, 0014); outside a bound source's `domains`, refused. A value
/// set is session state; a Saved identity keeps `localStorage`, and `sessionStorage` travels only in
/// an export bundle.
#[derive(Debug, Clone)]
pub struct WebStorage {
    _page: Page,
}

impl WebStorage {
    /// Every item, as name and value.
    pub fn items(&self) -> Call<Vec<(String, String)>> {
        unimplemented!()
    }

    /// One item's value, `None` when absent.
    pub fn get_item(&self, name: &str) -> Call<Option<String>> {
        let _ = name;
        unimplemented!()
    }

    /// Sets an item, replacing any value of that name.
    pub fn set_item(&self, name: &str, value: &str) -> Call<()> {
        let _ = (name, value);
        unimplemented!()
    }

    /// Removes an item.
    pub fn remove_item(&self, name: &str) -> Call<()> {
        let _ = name;
        unimplemented!()
    }

    /// Removes every item.
    pub fn clear(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Page {
    /// Playwright's `page.localStorage`, for the page's current origin.
    pub fn local_storage(&self) -> WebStorage {
        unimplemented!()
    }

    /// Playwright's `page.sessionStorage`, for the page's current origin.
    pub fn session_storage(&self) -> WebStorage {
        unimplemented!()
    }

    /// Playwright's `page.pause()`, with the Human pause's meaning: the program waits while a person
    /// acts in this page, in its window when it is headful on their desktop, else in the Live view
    /// on any device. A paused page takes no Gesture: its later Gestures wait; reads and waits run.
    /// No enclosing timeout counts its time; Usage still counts toward Limits. Dropping the future
    /// ends it.
    pub fn pause(&self) -> Pause {
        unimplemented!()
    }
}

// Details of the error codes. Each names the placeholder and the reference, never a value.

/// Details of `secret_unbound`, a program error: no scope binds this placeholder.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecretUnbound {
    /// The placeholder.
    pub placeholder: String,
    /// The names the scope binds.
    pub bound: Vec<String>,
}

/// Details of `secret_unavailable`: the provider gave no value (a missing variable, a locked
/// keychain, an unreachable vault, or a `caller:` request nobody answered).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecretUnavailable {
    /// The placeholder, when one was used.
    pub placeholder: Option<String>,
    /// The reference.
    pub reference: String,
    /// The provider asked, such as `env`, `keychain` or `caller`.
    pub provider: String,
    /// `missing`, `locked`, `unreachable` or `resolver_failed`; the code is not retryable.
    pub failure: String,
}

/// Details of `secret_domain_not_allowed`, a refusal: the field's frame or the request is outside
/// the binding's domains.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecretDomainNotAllowed {
    /// The placeholder.
    pub placeholder: String,
    /// The domain of the frame holding the field, or of the request.
    pub domain: String,
    /// The binding's domains.
    pub allowed: Vec<String>,
}

/// Details of `secret_input_refused`, a refusal: no secret is typed into or sent from a browser
/// other code may read, and no HTTP authentication challenge is answered there.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecretInputRefused {
    /// The placeholder.
    pub placeholder: String,
    /// `shared_browser` (ADR 0056), `cdp_attached` (ADR 0017) or `lent_page` (ADR 0089).
    pub reason: String,
}

/// One step of ADR 0006's recovery ladder and why it did not bring the login back.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RecoveryStep {
    /// `refresh`, `reimport`, `login`, `agent_login` or `human_pause`.
    pub step: String,
    /// `failed`, `skipped`, `declined` or `timed_out`.
    pub outcome: String,
    /// Why.
    pub reason: String,
}

/// Details of `logged_out`, a page error: the Page check found a login page and recovery did not
/// bring the login back, or was off.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoggedOut {
    /// Each step tried or skipped.
    pub steps: Vec<RecoveryStep>,
}

/// Details of `human_pause_declined`, a refusal: a person declined the step the pause asked for.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HumanPauseDeclined {
    /// Who declined.
    pub person: Option<String>,
    /// Their note.
    pub note: Option<String>,
}

/// Details of `session_not_savable`, a program error: the Session's cookies live outside run
/// state; the error's `fix` names the import of its source.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SessionNotSavable {
    /// The source kind.
    pub source: String,
}
