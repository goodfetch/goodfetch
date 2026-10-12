# A login is brought in through one Session source, read by the browser that owns it

A person often has a login already: a Playwright `storageState` file, a cookie export, a Profile directory, a browser they are using, or a browser their own Playwright script launched. Until now Goodfetch could only make a login inside its own Sessions (ADR 0006) and keep it as a Saved identity (ADR 0035). V1 brings every one of those logins in, through one setting, `session.source`, whose value is exactly one kind of source. Each browser's data is read by that browser itself, never decrypted by Goodfetch, because browsers bind their cookies to their own build, machine and, for Google accounts, device key. Decided in [Bringing an existing login or browser](https://github.com/Jbmanllr/automan/issues/117).

**Why the browser reads its own data.**
- **macOS:** Chrome encrypts cookies under the Keychain item "Chrome Safe Storage", other Chromium builds under "Chromium Safe Storage" ([keychain_password_mac.mm](https://chromium.googlesource.com/chromium/src/+/main/components/os_crypt/common/keychain_password_mac.mm)).
- **Windows:** App-Bound Encryption ties Chrome's cookies to Chrome's own executable, so another program decrypts them only with admin rights and by impersonating Chrome's elevation service, which is the infostealer technique ([Google Security Blog](https://security.googleblog.com/2024/07/improving-security-of-chrome-cookies-on.html)). yt-dlp calls its Chrome cookie import "permanently broken on Windows" ([#10927](https://github.com/yt-dlp/yt-dlp/issues/10927)), and the Rust reader `rookie` was archived in June 2026.
- **Chrome 136** refuses automation on its default profile, and any other directory uses a different key, so a copied profile loses its cookies ([Chrome blog](https://developer.chrome.com/blog/remote-debugging-port)).
- **Device Bound Session Credentials** (DBSC), generally available on Chrome for Windows since May 2026 for personal and Workspace Google accounts, keep short-lived cookies refreshed by a key in the TPM, so exported Google cookies stop working within minutes ([Workspace Updates](https://workspaceupdates.googleblog.com/2026/05/prevent-account-takeovers-with-DBSC-now-generally-available-in-the-Chrome-browser-for-Windows.html)). Such a login can only be used where it lives.

**Competitors.**
- **Per-run settings:** Playwright has `storageState`, `launchPersistentContext(userDataDir)` and `connectOverCDP`. Puppeteer has `connect`, including `connect({channel})` since 24.32. Crawlee has per-session cookies and `userDataDir`. Stagehand v4 has `userDataDir` and `localBrowser.connect({cdpUrl})`. Browser Use has `storage_state`, `user_data_dir` and `cdp_url`. Spider has `with_chrome_connection` and `with_cookies`.
- **Named stores, filled from the person's machine:**
  - Browserbase's `cookie-sync` skill reads cookies from local Chrome over CDP.
  - Browserless's `profile upload` reads them from disk.
  - Browser Use's profile sync uploads them.
  - Steel takes a zipped profile.

  Each of these sends the person's cookies to another IP, which Browser Use's own docs say sites may expire.
- **Driving the person's real browser:**
  - Chrome DevTools MCP (`--autoConnect`) and Puppeteer use Chrome 144's consented remote debugging.
  - The Playwright MCP extension and Claude in Chrome go through a browser extension.

## Decisions

**One source setting.** `session.source` is a tagged union defined once in `goodfetch-protocol` (ADR 0038), validated in one place, and shown as one picker in the Console:

| Kind | What it names | In a run | In `goodfetch identity import` |
|---|---|---|---|
| `saved_identity { name }` | A Saved identity (ADR 0035) | Used as today | — |
| `state_file { path, format?, key? }` | A Playwright `storageState` file, a Netscape `cookies.txt`, a JSON array of CDP `Network.Cookie` objects, or a session export bundle (ADR 0015, opened with `key`, a Secret reference). The format is detected; `format` overrides it. | Run-only import | Import |
| `cookie_header { header, domain }` | A raw `Cookie:` header, for Spider's `with_cookies` and Firecrawl's `headers` | Run-only import | Import |
| `profile_directory { path, browser }` | A Profile directory | Launched in place | Read |
| `browser { family, profile, domains }` | A browser installed on this machine | Run-only import | Import |
| `running_browser { browser, domains }` | The person's running browser | Borrowed browser | Read |
| `cdp_url { url, headers }` | Any browser reachable over CDP; header values are Secret references | Borrowed browser | Read |
| `client_browser` | A browser the SDK's `attach()` relays | Borrowed browser | — |

- **Two destinations, one importer.**
  - *Saved identity*, the default in the CLI and Console: the import creates a named one.
  - *Run-only import*: the import seeds only that run's Session, in run state, encrypted under the run key (ADR 0015) and removed by the reset gate.

  Neither keeps the source file.
- **Formats.**
  - *`storageState`:* read and written exactly to Playwright's schema as of 1.64: cookies, localStorage, IndexedDB, OPFS and passkeys with their sign count.
  - *sessionStorage:* travels only in Goodfetch's session export bundle, because Playwright's format has no field for it ([#8874](https://github.com/microsoft/playwright/issues/8874)).
  - *Export:* `goodfetch identity export --format storage-state` writes a Saved identity as `storageState` in clear text. Only a person can run it, through the CLI or the Console. It is refused to every Client acting for a model (ADR 0014) and recorded in the Audit log.

**The identity an import is bound to.** The cookies were minted by the person's own browser on the person's own network, and ADR 0021 treats a fingerprint or IP that changes under the same cookies as a bot signal. So an import is bound to its source:
- Access mode `standard`.
- An Identity profile with the source browser's family, major version, language and time zone. When a file does not carry them, they are taken from the importing machine and marked *inferred* in Provenance.
- Direct egress, unless the person names a proxy at import, which becomes the binding.
- `stealth` may be chosen at import, with a warning that the fingerprint changes under the same cookies.

From then on, ADR 0035's reuse rules apply.

**Domains.** An import from a browser or a Profile directory takes a list of registrable domains (ADR 0004), and `*` only when written explicitly, as `consent_domains` and `wall_domains` do (ADRs 0048 and 0052). A file import keeps every domain in the file unless the person filters it.

**Device-bound logins.** When the source is a live Chromium, the import lists the browser's device-bound sessions (`Network.enableDeviceBoundSessions`). It leaves out the cookies they keep refreshed, and says in plain words that this login can only be used by borrowing the running browser. The command is experimental in CDP, so it is probed: when a Chrome lacks it, the import warns that it could not check and goes on. A file cannot show this, so a file import relies on the Page check (ADR 0006).

**Every source counts as logged-in.** The engine cannot tell which cookies are a login. A Session seeded from an import, a Profile directory or a Borrowed browser therefore counts as logged-in for every rule that depends on it:
- ADR 0033's `session_credentials` permission;
- ADR 0052's rule that no Wall reset runs on a logged-in Session;
- ADR 0014's rule that keeps cookies away from models.

### Reading a browser's data

- **Chromium family** (Chrome, Edge, Brave and the others). The person's own installed browser reads its data over CDP: `Storage.getCookies` with HttpOnly cookies, `DOMStorage` and `IndexedDB`, without loading a page.
  - *Browser running:* through the consented attach below.
  - *Browser closed, non-default directory:* the engine launches that same browser on the directory, over a pipe, under the Profile directory rules below.
  - *Default profile with the browser closed:* the person is asked to open the browser and approve the attach, since Chrome allows nothing else there.
- **Firefox.**
  - *Cookies* are read from `cookies.sqlite` with `rusqlite`. Firefox does not encrypt them and the `moz_cookies` table has kept its shape for years, which is how yt-dlp and gallery-dl read them.
  - *localStorage and IndexedDB* are read by Goodfetch's Firefox (ADR 0019) on a temporary copy of the profile, over its pipe, with every request intercepted so that no site is contacted.
  - *Source profile newer than Goodfetch's Firefox:* that part fails with `profile_newer_than_browser`, and the cookies still import, with a warning.
- **Safari.** Cookies, localStorage and IndexedDB are in scope, and macOS allows reading them only after the person grants Full Disk Access; without it, a typed error explains how.
  - *Primary route:* Goodfetch's WebKit (ADR 0019) opens a copy of Safari's website data, as the Firefox route does.
  - *Fallback, for cookies:* an in-house decoder of `Cookies.binarycookies`, fuzzed, because no crate is widely used (the best, `safari-binarycookies`, has about 12,000 downloads).
  - The spec proves the primary route with a prototype; whichever route ships, all three stores are in scope.

### A Profile directory, in place

A run whose source is `profile_directory` launches, on that directory, the browser that wrote it: an installed browser by channel or path (ADR 0024), Goodfetch's Chromium for a directory it created, Firefox through Playwright or Camoufox, or WebKit for a directory WebKit created.
- **Lock and version.** The engine takes the browser's own lock and checks the directory's version before launching:
  - *Chromium family:* `SingletonLock`, plus `Local State` and `Last Version`.
  - *Firefox:* `parent.lock`, plus `compatibility.ini`.

  It refuses with `profile_in_use`, `profile_browser_mismatch` or `profile_newer_than_browser`. On Windows, a cookie file locked by a running browser is `profile_in_use`, never retried in a loop.
- **The profile's own key store.** The launch never passes the mock-keychain or `basic` password-store flags that automation tools add by default, since they stop a real profile's cookies from decrypting ([browser-use#5415](https://github.com/browser-use/browser-use/issues/5415)).
- **Chrome's default profile is refused**, pointing to `running_browser`.
- **The reset gate.** The directory is the person's data, not Goodfetch's, so the reset gate neither removes nor scans it. Everything the run itself creates elsewhere is still removed.
- **Resuming elsewhere** (ADR 0002):
  - *Through a proxy:* allowed, because the Session reaches sites through the same exit. The resumed Session continues from its checkpointed cookies and storage, and Provenance records that the directory stayed behind.
  - *On direct egress:* refused with `session_bound_to_machine`, since a new IP under the same cookies breaks ADR 0021.

### Borrowed browsers

A Borrowed browser is never closed and is leased to one work item at a time (ADR 0003). Goodfetch's own actions in it are guarded and journaled.

- **The person's running browser.** Goodfetch uses Chrome 144's consented remote debugging ([Chrome blog](https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session)). The person enables it once at `chrome://inspect/#remote-debugging`, and the browser asks "Allow remote debugging?" for each connection.
  - *Which browsers:* every Chromium-family browser that implements the mode, found through the `DevToolsActivePort` file in its profile (its `/json/version` returns 404 in this mode).
  - *Connecting:* one connection per lease, never reopened in a loop, because each attempt raises a dialog ([chrome-devtools-mcp#1794](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/1794)). While waiting for the person to enable the mode, the engine watches `DevToolsActivePort` for changes rather than polling it.
  - *Tabs:* Goodfetch opens its own tabs in the default context, where the login lives, and never touches the person's other tabs.
  - *Access mode:* `standard` only. Neither masking nor a proxy can change a running browser.
  - *Refusals:* when an administrator has turned remote debugging off, `remote_debugging_disabled_by_policy`. A running Firefox or Safari is refused, because neither offers a way in.
  - *Egress:* the egress check of ADR 0017 resolves each intercepted request's host in the engine and blocks private ranges. Because the browser resolves names itself, the DNS-rebinding gap that remains is recorded in Provenance.
- **A browser the person's own code launched.**
  - *SDK relay:* `goodfetch.attach(browserOrPage)`, in the TypeScript SDK for Playwright and Puppeteer, and in the Python SDK for Playwright's sync and async APIs. The relay opens a CDP session from the person's object (`newBrowserCDPSession`, `newCDPSession`, `createCDPSession`) and carries CDP to the engine over the JSON-RPC connection the SDK already holds. Events flow under ADR 0024's credits, and `$/cancel` ends the attach. No port is needed, so it works on the pipe-launched browsers Playwright starts by default.
  - *Through `cdp_url`:* Selenium 4 (its `se:cdp` address), the Rust crate and the CLI.
  - *Refused:* Playwright's Firefox and WebKit pages, which expose no CDP. Pyppeteer is not supported, because it is unmaintained.
  - *Provenance* marks such a run as sharing its browser, since the person's code can act on the same page outside the Effect journal.
- **Third-party cloud browsers** are Borrowed browsers as ADR 0022 already reaches them.

### Who may bring a login in

- **A person's own code** (a script, an SDK, the CLI) needs no extra approval, because running the code is the approval. A browser's own dialog still appears.
- **A Client acting for a model** (MCP, an agent, code mode) waits for a person's approval before any import or attach, through a Human pause in the Console, the CLI or a Notification. For a running browser, the browser's own "Allow remote debugging?" dialog counts as that approval. Without this gate, a prompt-injected agent could import an attacker's cookies, which is session fixation: the person's later work would land in the attacker's account.
- **A borrowed running browser is bound to a domain list** that a person approves.
  - *What it covers:* top-level navigations and new tabs. Subresources and frames follow their page, so a payment or CAPTCHA frame on an allowed page loads.
  - *An unlisted navigation*, such as a login redirect to an identity provider:
    - for a Client acting for a model, it raises a Human pause that asks the person to add the domain;
    - for the person's own code, it fails with `domain_not_allowed`.
- **Operator policy (ADR 0028) can lock which source kinds are allowed.** On an Exposed Daemon (ADR 0049), `profile_directory`, `browser` and `running_browser` are off by default, since they would read the server account's browser for anyone holding a token.

### A remote engine and the person's browser

When the engine runs on a shared Daemon or in Goodfetch Cloud, the person runs a Browser host (ADR 0022) on their own machine, beside their running browser or a Profile directory. It connects out to the engine and serves that Borrowed browser.
- The rules above apply, and the Browser host's token is scoped to that one Session.
- The person's browser never accepts an inbound connection.
- CDP over that link flows under ADR 0024's credits, and `$/cancel` ends the attach.
- **When the machine sleeps or goes offline:**
  - the Session is suspended;
  - the work item's lease is released after its grace period (ADR 0003);
  - the run waits in a Human pause ("your browser is offline") instead of failing;
  - it resumes when the Browser host reconnects.

This is the only way a remote run can use a device-bound login, and it keeps the person's cookies on their own machine.

### When an imported login expires

- **What a Saved identity records:** its source (kind, browser, profile and domains), when it was imported, and the earliest cookie expiry per domain.
- **Changes ADR 0006:** the recovery ladder gains **re-import from source**, right after the declared refresh. It runs unattended only when the source can be read without a person, that is a closed, non-default Profile directory.
  - *A source that needs a person*, such as a running browser or Chrome's default profile, or a source that is busy (`profile_in_use`): the step becomes part of the Human pause, where the person approves a re-import or logs in through the Live view.
  - *Runs started by a Client acting for a model:* always the Human pause.
  - *Budget:* re-import counts toward the limit of two recoveries per page.
  - *Sources with nothing to re-read:* file imports and run-only imports run the ladder as before.
- **Cookie expiry warnings.** A cookie's expiry is not the site's session expiry, so warnings say "cookies expire on…", and only the Page check judges whether a login is live.
  - *On a Daemon:* a Notification (ADR 0027) and a Console badge arrive when the earliest expiry falls inside a window, by default 3 days, as a setting. The Daemon schedules this as a timer at expiry minus the window.
  - *On an Attached engine:* a warning at run start.
  - *`goodfetch identity list` and its MCP operation* return each identity's expiry.

## Considered Options

- **Decrypting browsers' cookie files, as yt-dlp and `rookie` do**: rejected. On Windows it needs admin rights and the infostealer technique, on macOS it raises a Keychain prompt, antivirus tools flag the pattern, and the browser itself already does it correctly on every OS.
- **Copying a Chrome profile into Goodfetch's Chromium**, as Browser Use copies a profile into a temporary directory: rejected. Its cookies cannot be decrypted by another build or in another directory, and on Windows the copy fails while Chrome runs ([browser-use#4546](https://github.com/browser-use/browser-use/issues/4546)).
- **A browser extension that relays CDP**, as the Playwright MCP extension does: rejected on two grounds:
  - *Distribution:* an extension holding the `debugger` permission goes through review in every browser's store.
  - *Administrator policy:* it would get around an administrator who turned remote debugging off.

  Chrome 144's consented mode needs neither, and it asks the person on every connection.
- **Uploading the person's cookies to the remote engine**, as Browserbase, Browser Use and Browserless do: rejected as the way to reach a remote engine. It breaks device-bound logins, moves the cookies to another IP under the same login, and puts them on someone else's machine. A local Browser host keeps them where they are. Moving a Saved identity on purpose still uses ADR 0015's session export.
- **Separate options per mechanism** (`storageState`, `userDataDir`, `cdpUrl`, `connect`): rejected. They overlap, allow contradictory combinations, and each surface would validate them on its own.
- **A Saved identity as the only destination**: rejected. A CI job with a `storageState` file would have to create a stored object for one run, on a machine whose keychain may not exist.
- **Guessing which cookies are a login**: rejected. A wrong "anonymous" would hand an account to a Plugin or log it out through a Wall reset.
- **Exporting `cookies.txt`**: rejected as over-engineering. No V1 parity row or pain needs Goodfetch to produce it.

## Consequences

- **Changes ADR 0001:** a Profile directory used in place is outside the reset gate, as the person's own data. Saved identities remain the only Goodfetch state that outlives a run.
- **Changes ADR 0033:** "imported cookies" now means every source of this ADR, and the `session_credentials` permission covers them all.
- **Changes ADR 0035 and ADR 0045:**
  - `goodfetch identity` now manages Saved identities: `import`, `export`, `list` and `delete`.
  - The Web Bot Auth key commands become `goodfetch signing-key create` and `goodfetch signing-key directory`.
- **Changes ADR 0037:** rows SA-11, pain 4 and pain 12 become `decided`, and the Site access, Client surfaces and Self-hosted deployment areas no longer wait on this ticket.
- **ADR 0017 is unchanged:** the engine still reaches the browsers it launches over a pipe and never opens a debugging port. A running browser's consented mode is opened by the person, and Goodfetch only connects to it.
- **To prove in the spec:**
  - the Safari primary route, by prototype;
  - which Chromium-family browsers other than Chrome implement the consented mode, by probe;
  - the conformance cases: each refusal code; the reset gate leaving a Profile directory intact; the egress check on a Borrowed browser; and the Human pause on a model's import.
