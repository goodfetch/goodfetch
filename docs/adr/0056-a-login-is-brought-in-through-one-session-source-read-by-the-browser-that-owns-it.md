# A login is brought in through `session.source`, read by the browser that owns it

A person often has a login already, meaning the signed-in state a site keeps in cookies and storage. It can come from:
- a Playwright `storageState` file;
- a cookie export;
- a Profile directory;
- a browser they are using;
- a browser their own Playwright script launched.

Until now Goodfetch could only make a login inside its own Sessions (ADR 0006) and keep it as a Saved identity (ADR 0035). V1 brings every one of those logins in, through one setting, `session.source`, whose value is exactly one kind of source. Each browser's data is read by that browser itself, never decrypted by Goodfetch, because browsers bind their cookies to their own build and machine and, for Google accounts, to a device key. Decided in [Bringing an existing login or browser](https://github.com/Jbmanllr/automan/issues/117).

**Why the browser reads its own data.**
- **macOS:** Chrome encrypts cookies under the Keychain item "Chrome Safe Storage", while other Chromium builds use "Chromium Safe Storage" ([keychain_password_mac.mm](https://chromium.googlesource.com/chromium/src/+/main/components/os_crypt/common/keychain_password_mac.mm)).
- **Windows:** App-Bound Encryption ties Chrome's cookies to Chrome itself. Another program reads them only with system privileges or by injecting code into Chrome, which are the infostealer techniques ([Google Security Blog](https://security.googleblog.com/2024/07/improving-security-of-chrome-cookies-on.html)). A yt-dlp maintainer expected it to leave Chrome cookie import "permanently broken on Windows" ([#10927](https://github.com/yt-dlp/yt-dlp/issues/10927)), and the Rust reader `rookie` is now archived.
- **Chrome 136** refuses automation on its default data directory, and any other directory uses a different encryption key, so a copy of the default profile cannot decrypt its cookies ([Chrome blog](https://developer.chrome.com/blog/remote-debugging-port)).
- **Device Bound Session Credentials** (DBSC) are generally available on Chrome for Windows since 2026-05-28, for personal and Workspace Google accounts. They keep short-lived cookies refreshed by a key in the TPM, so exported Google cookies stop working soon after export ([Workspace Updates](https://workspaceupdates.googleblog.com/2026/05/prevent-account-takeovers-with-DBSC-now-generally-available-in-the-Chrome-browser-for-Windows.html)). Such a login can only be used where it lives.

**Competitors.**
- **Per-run settings:**
  - Playwright: `storageState`, `launchPersistentContext(userDataDir)` and `connectOverCDP`.
  - Puppeteer: `connect`, including `connect({channel})` since 24.32.
  - Crawlee: per-session cookies and `userDataDir`.
  - Stagehand v4: `userDataDir` and `localBrowser.connect({cdpUrl})`.
  - Browser Use: `storage_state`, `user_data_dir` and `cdp_url`.
  - Spider: `with_chrome_connection` and `with_cookies`.
- **Named stores, filled from the person's machine:**
  - Browserbase's `cookie-sync` skill reads cookies from local Chrome over CDP.
  - Browserless's `profile upload` reads them from disk.
  - Browser Use's profile sync uploads them.
  - Steel takes a zipped profile.

  Each of these moves the person's cookies to another IP, which Browser Use's own docs say sites may expire.
- **Driving the person's real browser:**
  - Chrome DevTools MCP (`--autoConnect`) and Puppeteer use Chrome 144's consented remote debugging.
  - The Playwright MCP extension and Claude in Chrome go through a browser extension.

## Considered Options

- **Decrypting browsers' cookie files**, as yt-dlp and `rookie` do: rejected. On Windows it needs system privileges or code injection, on macOS it raises a Keychain prompt, antivirus tools flag the pattern, and the browser already does it correctly on every OS.
- **Copying a Chrome profile into Goodfetch's Chromium**, as Browser Use copies a profile into a temporary directory: rejected. Another build or another directory cannot decrypt its cookies, and on Windows the copy fails while Chrome runs ([browser-use#4546](https://github.com/browser-use/browser-use/issues/4546)).
- **A browser extension that relays CDP**, as the Playwright MCP extension does: rejected on two grounds:
  - *Distribution:* an extension holding the `debugger` permission goes through review in every browser's store.
  - *Administrator policy:* it would get around an administrator who turned remote debugging off.

  Chrome 144's consented mode needs neither.
- **Uploading the person's cookies to a remote engine**, as Browserbase, Browser Use and Browserless do: rejected as the way to reach one. It breaks device-bound logins, moves the cookies to another IP under the same login, and puts them on someone else's machine. A local Browser host keeps them where they are. Moving a Saved identity on purpose still uses ADR 0015's session export.
- **Separate options per mechanism** (`storageState`, `userDataDir`, `cdpUrl`, `connect`): rejected. They overlap, allow contradictory combinations, and each surface would validate them on its own.
- **A Saved identity as the only destination**: rejected. A CI job with a `storageState` file would have to create a stored object for one run, on a machine whose keychain may not exist.
- **Guessing which cookies are a login**: rejected. A wrong "anonymous" would hand an account to a Plugin or log it out through a Wall reset.
- **Letting a Recipe set the source**: rejected. A third-party Recipe could then read the person's browser into Sinks its author controls. As with `wall_domains` (ADR 0052), only the person's own configuration sets `session.source`.
- **Exporting `cookies.txt`**: rejected as over-engineering. No V1 parity row or pain needs Goodfetch to produce it.

## Consequences

### One source setting

`session.source` is a tagged union defined once in `goodfetch-protocol` (ADR 0038), validated in one place, and shown as one picker in the Console. Its kinds:

| Kind | What it names | In a run | In `goodfetch identity import` |
|---|---|---|---|
| `saved_identity { name }` | A Saved identity (ADR 0035) | Used as today | — |
| `state_file { path, format?, key? }` | A Playwright `storageState` file, a Netscape `cookies.txt`, a JSON array of CDP `Network.Cookie` objects, or a session export bundle (ADR 0015, opened with `key`, a Secret reference). The format is detected; `format` overrides it. | Run-only import | Parsed |
| `cookie_header { header, domain }` | A raw `Cookie:` header, for Spider's `with_cookies` and Firecrawl's `headers` | Run-only import | Parsed |
| `profile_directory { path, browser, profile_name?, domains }` | A Profile directory, and for Chromium the profile inside it | Launched in place | Read by the browser |
| `browser { family, profile_name?, domains }` | A browser installed on this machine | Run-only import | Read by the browser |
| `running_browser { browser, domains }` | The person's running browser | Borrowed browser | Read by the browser |
| `cdp_url { url, headers }` | Any browser reachable over CDP; header values are Secret references | Borrowed browser | Read by the browser |
| `client_browser` | A browser the SDK's `borrow()` relays | Borrowed browser | — |

**Who sets it.** Only user and project configuration, the run layer, or a person in the Console or CLI sets `session.source`. A Recipe may expose it, but never sets or locks it. Plugins never set it. **Changes ADR 0028.**

**Destinations.** One importer serves two destinations, and neither keeps the source file:
- *a Saved identity*, the default in the CLI and the Console;
- *a run-only import*, which seeds only that run's Session, in run state, encrypted under the run key (ADR 0015) and removed by the reset gate.

**Formats.**
- *`storageState`:* read and written exactly to Playwright's schema as of 1.64. It holds cookies and localStorage, plus what Playwright's opt-in options add: IndexedDB, OPFS, and virtual-authenticator credentials with their sign count. Export turns every option on.
- *sessionStorage:* travels only in Goodfetch's session export bundle, because Playwright's format has no field for it ([#8874](https://github.com/microsoft/playwright/issues/8874)).
- *Export:* `goodfetch identity export --format storage-state` writes a Saved identity as `storageState` in clear text. Only a person can run it, through the CLI or the Console. It is refused to every Client acting for a model and recorded in the Audit log. **Changes ADR 0015.**

**Domains.** A source read from a browser or a Profile directory takes a list of registrable domains (ADR 0004), with `*` only when written explicitly, as `consent_domains` and `wall_domains` do (ADRs 0048 and 0052). A file import keeps every domain in the file unless the person filters it.

**Device-bound logins.** When the source is a live Chromium, the import lists the browser's device-bound sessions (`Network.enableDeviceBoundSessions`). It leaves out the cookies they keep refreshed, and says in plain words that this login can only be used by borrowing the running browser. The command is experimental in CDP, so the import probes for it: a Chrome that lacks it gets a warning that the check could not run, and the import goes on. A file cannot show this, so a file import relies on the Page check (ADR 0006).

**Every source counts as logged-in.** The engine cannot tell which cookies are a login. A Session seeded from an import, a Profile directory or a Borrowed browser therefore counts as logged-in for every rule that depends on that, including:
- ADR 0033's `session_credentials` permission;
- ADR 0022's rule on Plugin-supplied browsers;
- ADR 0052's rule that no Wall reset runs on a logged-in Session;
- ADR 0014, which now keeps a logged-in Session's cookies and storage away from models and observations. **Changes ADR 0014.**

**Consent answers.** An import keeps the Consent prompt answers the person gave in their own browser as that person's acceptance. Provenance and the Audit log record them as imported, and saving the identity does not drop them. **Changes ADR 0048.**

### The identity an import is bound to

The cookies were minted by the person's own browser on the person's own network, and ADR 0021 treats a fingerprint or IP that changes under the same cookies as a bot signal. So an import is bound to its source:
- **Access mode** `standard`.
- **Identity profile:** the source browser's family, major version, language and time zone. When a file does not carry them, they are taken from the importing machine and marked *inferred* in Provenance.
- **Which browser runs it:** an installed browser of the source's family (by channel, ADR 0024) when one exists, else Goodfetch's build of that engine. Version fields follow the running build, as in ADR 0035, and Provenance records the source's major.
- **Egress:** direct, unless the person names a proxy at import, which becomes the binding. With a proxy, language and time zone stay the source's, because the site knows the login by them, and the import warns when the exit's country differs. **Changes ADR 0021**, whose `standard` matches them to the proxy exit.
- **`stealth`** may be chosen at import, with a warning that the fingerprint changes under the same cookies.

From then on, ADR 0035's reuse rules apply.

### Reading a browser's data

- **Chromium family** (Chrome, Edge, Brave and the others). The person's own installed browser reads its data over CDP, without loading a page:
  - cookies, HttpOnly ones included, through `Storage.getCookies`;
  - localStorage through `DOMStorage`;
  - IndexedDB through `IndexedDB`.

  How Goodfetch reaches the browser depends on its state:
  - *Running:* through the consented connection below.
  - *Closed, non-default data directory:* the engine launches that same browser on the directory, over a pipe, under the Profile directory rules below.
  - *Default data directory, closed:* the person is asked to open the browser and approve the connection, since Chrome allows nothing else there.
- **Firefox.**
  - *Cookies* are read from a copy of `cookies.sqlite` and its `-wal` file, with `rusqlite`. Firefox does not encrypt them and the `moz_cookies` table has kept its shape for years; yt-dlp and gallery-dl read them the same way.
  - *localStorage and IndexedDB* are read by Goodfetch's Firefox (ADR 0019) on a temporary copy of the profile, over its pipe, with every request intercepted so that no site is contacted.
  - *Source profile newer than Goodfetch's Firefox:* that part fails with `profile_directory_newer_than_browser`, and the cookies still import, with a warning.
- **Safari.** Cookies, localStorage and IndexedDB are in scope.
  - *Permission:* macOS allows reading them only after the person grants Full Disk Access. Without it, a typed error explains how.
  - *Primary route:* Goodfetch's WebKit (ADR 0019) opens a copy of Safari's website data, as the Firefox route does.
  - *Fallback for cookies:* an in-house decoder of `Cookies.binarycookies`, fuzzed, written because no crate is widely used (the best, `safari-binarycookies`, has about 12,000 downloads).

### A Profile directory, in place

A run whose source is `profile_directory` launches a browser on that directory:
- *Chromium family:* the installed browser that wrote it, by channel or path (ADR 0024), or Goodfetch's Chromium for a directory it created.
- *Firefox:* Playwright's Firefox or Camoufox. Firefox cookies are not encrypted, so a different Firefox build keeps them; Provenance records that the build differs from the one that wrote the profile.
- *WebKit:* only a directory WebKit created.

The rules for an in-place launch:
- **Lock and version.** The engine takes the browser's own lock and checks the directory's version before launching:
  - *Chromium family:* `SingletonLock`, plus `Local State` and `Last Version`.
  - *Firefox:* `parent.lock`, plus `compatibility.ini`.

  It refuses with `profile_directory_in_use`, `profile_directory_browser_mismatch` or `profile_directory_newer_than_browser`. On Windows, a cookie file locked by a running browser counts as `profile_directory_in_use` and is never retried in a loop.
- **The profile's own key store.** The launch never passes the mock-keychain or `basic` password-store flags that automation tools add by default, because they stop a real profile's cookies from decrypting ([browser-use#3225](https://github.com/browser-use/browser-use/issues/3225)).
- **Chrome's default data directory is refused**, pointing to `running_browser`.
- **Writes persist.** Cookies, storage and Consent prompt answers the run sets stay in the directory, as they would in the person's own use.
- **The reset gate.** The directory is the person's data, not Goodfetch state, so the reset gate neither removes nor scans it. Everything the run creates elsewhere is still removed. **Changes ADR 0001 and ADR 0015.**
- **Resuming elsewhere** (ADR 0002):
  - *Through a proxy:* allowed, because the Session reaches sites through the same exit. The resumed Session continues from its checkpointed cookies and storage, and Provenance records that the directory stayed behind.
  - *On direct egress:* refused with `session_bound_to_machine`, since a new IP under the same cookies breaks ADR 0021.

### Borrowed browsers

A Borrowed browser serves one run and one Session, in its default context, where the login lives. It is never closed, and browser-wide settings such as download behaviour are never changed in it.
- **Work items:** they use it one at a time, each under its own lease (ADR 0003).
- **A second run** trying to borrow the same browser is refused with `browser_borrowed`.
- **The Session pool** never rotates, retires or escalates its Session. A failure that would do so raises a Human pause.
- **Identity:** the Session is `standard`, and its identity is the browser's own, recorded in Provenance.
- **Resources:** the governor neither recycles a Borrowed browser nor counts it in the memory allowance, as for a vendor's browser. **Changes ADR 0040.**
- **Refused Sessions:** a Session with a certificate exception (`certificate_exception_unsupported`, ADR 0042) or a local address (`local_address_unavailable`, ADR 0044).
- **What persists:** cookies, storage and Consent prompt answers the run sets stay in the person's browser.

**The person's running browser.** Goodfetch uses Chrome 144's consented remote debugging ([Chrome blog](https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session)).
- *Setup:* the person enables it once at `chrome://inspect/#remote-debugging`, and the browser asks "Allow remote debugging?" for each WebSocket connection ([chrome-devtools-mcp#1794](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/1794)).
- *Finding it:* the browser is found through the `DevToolsActivePort` file in its profile, since its `/json/version` returns 404 in this mode ([chrome-devtools-mcp#2283](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/2283)). Chrome and Edge implement the mode; other Chromium-family browsers are probed.
- *One connection per run,* opened once after the person approves and held for the run's life. It is never reopened in a loop, because each attempt raises a dialog. While waiting for the person to enable the mode, the engine watches `DevToolsActivePort` for changes rather than polling it.
- *Tabs:* Goodfetch opens its own tabs and never touches the person's other tabs.
- *Refusals:*
  - when an administrator has turned remote debugging off, `remote_debugging_disabled_by_policy`;
  - a running Firefox or Safari, because neither offers a way in.

**A browser the person's own code launched.** This is the reverse of ADR 0019, where the person's Playwright attaches to Goodfetch's browser.
- *The SDK relay:* `goodfetch.borrow(browserOrPage)`, in the TypeScript SDK for Playwright and Puppeteer, and in the Python SDK for Playwright's sync and async APIs.
  - It opens a CDP session from the person's object (`newBrowserCDPSession`, `newCDPSession`, `createCDPSession`) and carries CDP over the JSON-RPC connection the SDK already holds.
  - Commands go to the browser under ADR 0019's per-command timeouts. The browser's events go to the engine under ADR 0024's credits, so a busy page cannot flood the connection.
  - `$/cancel` ends the borrow.
  - No port is needed, so it works on the pipe-launched browsers Playwright starts by default.
- *Through `cdp_url`:* Selenium 4 (through its `se:cdp` address), the Rust crate and the CLI.
- *Refused:* Playwright's Firefox and WebKit pages, which expose no CDP. Pyppeteer is not supported, because it is unmaintained.

**The person's code is a raw CDP client (ADR 0017).** On `client_browser` and `cdp_url`, that code holds its own CDP session and can act outside the Effect journal and switch off interception. So:
- the engine refuses to type secrets;
- results are marked unguarded, and Provenance marks the run as sharing its browser;
- the egress check is interception only.

**Egress on every Borrowed browser.** No egress proxy can be put under a browser that is already running, so the engine resolves each intercepted request's host itself and blocks private ranges. The browser resolves names itself, so a DNS-rebinding gap remains, and it is recorded in Provenance. **Changes ADR 0017**, whose connect-time check this replaces for Borrowed browsers.

**Which `cdp_url` addresses are allowed.** A `cdp_url` counts as operator-configured only when it is set in Operator policy or in user and project configuration. Otherwise it is treated as an address a page can reach, and private ranges are refused.

**Audit.** Every import and every borrow is recorded in the Audit log.

**Third-party cloud browsers** that Goodfetch reaches by URL, without creating them, are Borrowed browsers. A cloud browser that a browser backend Plugin creates is released by that Plugin, as ADR 0022 has it.

### Who may bring a login in

- **A person's own code** (a script, an SDK, the CLI) needs no extra approval, because running the code is the approval. A browser's own dialog still appears.
- **A Recipe or a Plugin is not the person's own code.** A Recipe run whose source is `profile_directory`, `browser`, `running_browser` or `cdp_url` waits for a person's approval, as a Client acting for a model does.
- **A Client acting for a model** (MCP, an agent, code mode) waits for a person's approval before any import or borrow, through a Human pause in the Console, the CLI or a Notification. For a running browser, the browser's own "Allow remote debugging?" dialog counts as that approval. The reason is session fixation: a prompt-injected agent could import an attacker's cookies, and the person's later work would land in the attacker's account.
- **Navigation on a borrowed running browser, or on a Profile directory used in place, is bound to its source's `domains`,** which a person approves.
  - The list covers top-level navigations and new tabs. Subresources and frames follow their page, so a payment or CAPTCHA frame on an allowed page loads.
  - An unlisted navigation, such as a login redirect to an identity provider, is handled by who started the run:
    - for a Client acting for a model or a Recipe run, a Human pause asks the person to add the domain;
    - for the person's own code, it fails with `domain_not_allowed`.
- **Operator policy (ADR 0028) can lock which source kinds are allowed.** On an Exposed Daemon (ADR 0049), `profile_directory`, `browser`, `running_browser` and `cdp_url` are off by default, because they would reach the server account's browser or network for anyone holding a token. There, `state_file` reads only content the Client uploads, never a server path.
- **Daemon groups.** In a Daemon group, a run whose source is `profile_directory`, `browser` or `running_browser` runs only on the Group member where that source exists, and its paths take the *group member* scope (ADR 0028). **Changes ADR 0055.**

### A remote engine and the person's browser

When the engine runs on a shared Daemon or in Goodfetch Cloud, the person runs a Browser host (ADR 0022) on their own machine, beside their running browser or a Profile directory. The Browser host carries out the Session's operations beside the browser, behind the guard, and serves that Borrowed browser.
- **The connection runs outward:** the Browser host connects out to the engine, and the person's browser never accepts an inbound connection. The rules above apply, and the Browser host's token is scoped to that one Session.
- **Nothing is wiped:** the Browser host wipes nothing and never closes the browser.
- **Cookies stay local:** the Session's cookies and storage are not copied back into run state. A Checkpoint records only that the browser stayed behind, so the run resumes only through that Browser host. **Changes ADR 0022.**
- **Events and cancellation:** the Browser host's events flow under ADR 0024's credits, and `$/cancel` ends the borrow.
- **When the machine sleeps or goes offline:**
  - the Session is suspended;
  - the work item's lease is released after its grace period (ADR 0003);
  - the run waits in a Human pause ("your browser is offline") instead of failing;
  - it resumes when the Browser host reconnects.

This is the only way a remote run can use a device-bound login, and it keeps the person's cookies on their own machine.

### When an imported login expires

- **What a Saved identity records:** its source (kind, browser, profile and domains), when it was imported, and the earliest cookie expiry per domain.
- **Changes ADR 0006:** the recovery ladder gains **re-import from source**, right after the declared refresh.
  - *Unattended:* only when the source can be read without a person. That means a closed, non-default Chromium data directory, a Firefox profile, or Safari once Full Disk Access is granted.
  - *Needing a person:* a running browser, Chrome's default data directory, or a busy source (`profile_directory_in_use`). The step becomes part of the Human pause, where the person approves a re-import or logs in through the Live view.
  - *Started by a Client acting for a model:* always the Human pause.
  - *Budget:* re-import counts toward the limit of two recoveries per page.
  - *File and run-only imports* have nothing to re-read and run the ladder as before.
- **Cookie expiry warnings.** A cookie's expiry is not the site's session expiry, so the warnings say "cookies expire on…", and only the Page check judges whether a login is still live.
  - *On a Daemon:* a Notification (ADR 0027) and a Console badge appear when the earliest expiry falls inside a window, by default 3 days, a setting. The Daemon schedules this as a timer at expiry minus the window. In a Daemon group, the Lead Daemon, which keeps Saved identities, runs the timers and sends each Notification once.
  - *On an Attached engine:* a warning at run start.
  - *In listings:* `goodfetch identity list` and its MCP operation return each identity's expiry.

### Other changes

- **Changes ADR 0001:** a Profile directory used in place is outside the reset gate, as the person's own data. Saved identities remain the only exception to the reset rule.
- **Changes ADR 0033:** its `session_credentials` permission covers every source of this ADR.
- **Changes ADR 0035 and ADR 0045:** `goodfetch identity` now manages Saved identities (`import`, `export`, `list` and `delete`). The Web Bot Auth key commands become `goodfetch signing-key create` and `goodfetch signing-key directory`, which **changes ADR 0021** too.
- **Changes ADR 0037:**
  - Rows SA-08, SA-11, pain 4 and pain 12 become `decided`.
  - Site access, Client surfaces and Self-hosted deployment no longer wait on this ticket.
- **To prove in the spec:**
  - the Safari primary route, by prototype;
  - which Chromium-family browsers other than Chrome and Edge implement the consented mode, by probe;
  - the Python sync relay, since Playwright's sync objects are bound to the thread that created them, while ADR 0038's sync API runs its core on its own thread;
  - the conformance cases:
    - each refusal code;
    - the reset gate leaving a Profile directory intact;
    - the egress check on a Borrowed browser;
    - the Human pause on a model's or a Recipe's import.
