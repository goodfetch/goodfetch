# A login is brought in through `session.source`, read by the browser that owns it

A person often has a login already, meaning the signed-in state a site keeps in cookies and storage. It can come from:
- a Playwright `storageState` file;
- a cookie export;
- a Profile directory;
- a browser they are using;
- a browser their own Playwright script launched.

Until now Goodfetch could only make a login inside its own Sessions (ADR 0006) and keep it as a Saved identity (ADR 0035). V1 brings every one of those logins in, through one setting, `session.source`, the Session source, whose value is exactly one kind of source. A Chromium-family browser's data is read by that browser itself. Firefox and Safari do not encrypt their cookies, so Goodfetch reads a copy of their data, with its own Firefox or WebKit for storage. Goodfetch never decrypts a browser's cookies, because browsers bind their cookies to their own build and machine and, for Google accounts, to a device key. Decided in [Bringing an existing login or browser](https://github.com/goodfetch/goodfetch/issues/117).

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
- **Uploading the person's cookies to a remote engine**, as Browserbase, Browser Use and Browserless do: rejected as the way to reach one. It breaks device-bound logins, moves the cookies to another IP under the same login, and puts them on someone else's machine. A local Browser host keeps them where they are. Moving a Saved identity on purpose still uses ADR 0015's session export, except for an import bound to its machine, which does not move: a remote engine uses the source browser through a Browser host (below), or the person uploads a file for a new import there, which warns that the cookies then reach sites from another IP.
- **Separate options per mechanism** (`storageState`, `userDataDir`, `cdpUrl`, `connect`): rejected. They overlap, allow contradictory combinations, and each surface would validate them on its own.
- **A Saved identity as the only destination**: rejected. A CI job with a `storageState` file would have to create a stored object for one run, on a machine whose keychain may not exist.
- **Guessing which cookies are a login**: rejected. A wrong "anonymous" would hand an account to a Plugin or log it out through a Wall allowance refetch.
- **Letting a Recipe set the source**: rejected. A third-party Recipe could then read the person's browser into Sinks its author controls. As with `wall_domains` (ADR 0052), a Recipe cannot set it: only the person sets `session.source`, through their configuration, their run, or the Console or CLI, and Operator policy sets or limits it.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): a person reconciles an uncertain Effect, raises a Limit and answers a Human pause through the CLI's interactive prompts, or later Goodfetch Cloud, and logs in through a headful window locally or the Live view once it is built. The rule otherwise stands.
- **Exporting `cookies.txt`**: rejected as over-engineering. No V1 parity row or pain needs Goodfetch to produce it.

## Consequences

### One source setting

`session.source` is a tagged union defined once in `goodfetch-protocol` (ADR 0038), validated in one place, and shown as one picker in the Console. Its kinds:

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

| Kind | What it names | In a run | In `goodfetch identity import` |
|---|---|---|---|
| `saved_identity { name }` | A Saved identity (ADR 0035) | Used as today | — |
| `state_file { path \| upload, format?, key?, domains? }` | A Playwright `storageState` file, a Netscape `cookies.txt`, a JSON array of CDP `Network.Cookie` objects, or a session export bundle (ADR 0015, opened with `key`, a Secret reference). The format is detected; `format` overrides it. | Run-only import | Parsed |
| `cookie_header { header, domain }` | A raw `Cookie:` header, for Spider's `with_cookies` and Firecrawl's `headers` | Run-only import | Parsed |
| `profile_directory { path, browser, chromium_profile?, domains }` | A Profile directory, and for a Chromium-family browser the profile folder inside it | Launched in place | Read by the browser (Chromium family) or from a copy (Firefox, Safari) |
| `installed_browser { browser, chromium_profile?, domains }` | A browser installed on this machine, such as Chrome, Edge, Brave, Firefox or Safari | Run-only import | Read by the browser (Chromium family) or from a copy (Firefox, Safari) |
| `running_browser { browser, domains }` | The person's running browser | Borrowed browser | Read by the browser |
| `cdp_url { url, headers?, domains?, exclusive? }` | Any browser reachable over CDP; header values are Secret references, `exclusive` declares that nothing else drives it, and `domains` is required for an import and for a run by a Client acting for a model or a Recipe | Borrowed browser | Read by the browser |
| `client_browser` | A browser the SDK's `run.borrow()` relays | Borrowed browser | — |

**Who sets it.** Only Operator policy, user and project configuration, the developer's own code or the run layer, or a person in the Console or CLI sets `session.source`. A Recipe may expose it, but never sets or locks it. Plugins never set it. **Changes ADR 0028.**

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

**Destinations.** One importer serves two destinations, and neither keeps the source file:
- *a Saved identity*, the default in the CLI and the Console;
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- *a run-only import*, which seeds only that run's Session, in run state, encrypted under the run key (ADR 0015) and removed by the reset gate.

**Formats.**
- *`storageState`:* read and written exactly to Playwright's schema as of 1.64. It holds cookies and localStorage, plus what Playwright's opt-in options add: IndexedDB, OPFS, and virtual-authenticator credentials with their sign count. Export turns every option on.
- *sessionStorage:* travels only in Goodfetch's session export bundle, because Playwright's format has no field for it ([#8874](https://github.com/microsoft/playwright/issues/8874)).
- *Export:* `goodfetch identity export --format storage-state` writes a Saved identity as `storageState` in clear text. Only a person can run it, in the Console or an interactive CLI prompt, under ADR 0031's step-up on a Daemon with accounts; no agent surface offers it, and it is recorded in the Audit log. **Changes ADR 0011 and ADR 0015.**
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

**Domains.** A source read from a browser or a Profile directory takes a list of registrable domains (ADR 0004), with `*` only when written explicitly, as `consent_domains` and `wall_domains` do (ADRs 0048 and 0052). A file import keeps every domain it holds unless the person filters them with `domains`; a cookie header applies to its `domain`.

**Device-bound logins.** When the source is a live Chromium, the import lists the browser's device-bound sessions (`Network.enableDeviceBoundSessions`). It leaves out the cookies they keep refreshed, and says in plain words that this login can only be used in the browser that holds it: by borrowing it when it runs, or, for a non-default data directory, on its Profile directory in place. The command is experimental in CDP, so the import probes for it: a Chrome that lacks it gets a warning that the check could not run, and the import goes on. A file cannot show this, so a file import relies on the Page check (ADR 0006).

**Every source counts as logged-in.** The engine cannot tell which cookies are a login. A Session seeded from an import, a Profile directory or a Borrowed browser therefore counts as logged-in for every rule that depends on that, including:
- ADR 0033's `session_credentials` permission;
- ADR 0022's rule on Plugin-supplied browsers;
- ADR 0052's rule that no Wall allowance refetch runs on a Session logged in to that site; a Profile directory used in place or a Borrowed browser counts as logged in to every site. **Changes ADR 0052**, whose list of logged-in sources now includes these;
- ADR 0014, which now keeps a logged-in Session's cookies and storage away from models and observations. **Changes ADR 0014.**

**Consent answers.** An import keeps the Consent prompt answers the person gave in their own browser as that person's acceptance. Provenance and the Audit log record them as imported, and saving the identity does not drop them. **Changes ADR 0048.**

### The identity an import is bound to

The cookies were minted by the person's own browser on the person's own network, and ADR 0021 treats a fingerprint or IP that changes under the same cookies as a bot signal. So an import is bound to its source:
- **Access mode** `standard`.
- **Identity profile:** the source browser's language and time zone, and its family (Chrome, Edge, Brave, Firefox or Safari) when the browser that runs it is of that family, otherwise the running browser's own, with the warning below; the source's major version is recorded in Provenance. When a file does not carry them, they are taken from the machine the file comes from (for an upload, the Client's, as the SDK, CLI or Console reports it) and marked *inferred* in Provenance. A Safari import runs with WebKit's own identity, which never claims to be Safari (ADR 0035), and the import warns that the identity differs from the browser that minted the cookies.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Which browser runs it:** for the Chromium family, an installed browser of the source's family, by channel or path (ADR 0024), when one exists, else ADR 0024's default Chromium (Chrome for Testing, or the distribution's Chromium on musl); for a Firefox source, Goodfetch's Firefox; for a Safari source, Goodfetch's WebKit (ADR 0019). Version fields follow the running build, as in ADR 0035. When the browser that runs it is not of the source's family, the import warns that the identity differs from the browser that minted the cookies, as for Safari.
- **Egress:** direct, unless the person names a proxy at import, which becomes the binding. On direct egress the import is bound to the importing machine: a run using it elsewhere, including on another Group member, is refused with `session_bound_to_machine`, and a Daemon group places it only on that machine. With a proxy, language and time zone stay the source's, because the site knows the login by them, and the import warns when the exit's country differs. **Changes ADR 0021**, whose `standard` matches them to the proxy exit.
- **`stealth`** may be chosen at import, with a warning that the fingerprint changes under the same cookies. On a Safari import it is refused, because WebKit has no stealth mode (ADR 0035); on a Firefox import it runs on Camoufox, and on a Chromium-family import on the Goodfetch build (ADR 0035; on musl, its `basic` fallback). A `stealth` import's generated identity keeps the source's language and time zone. **Changes ADR 0035**, whose generated identities match them to the proxy exit. `identified` is refused at import, because a person's login is not a declared agent's (ADR 0021).
- **A session export bundle** (ADR 0015) is not rebound: it keeps its Saved identity's Access mode, Identity profile seed and proxy binding, and ADR 0035's reuse rules apply to it unchanged. A bundle of an import bound to its machine keeps that binding: using it on another machine is refused with `session_bound_to_machine`. **Changes ADR 0015 and ADR 0035.**
- **Goodfetch Cloud**, where each run has its own engine, designs what a direct-egress import is bound to with its Saved identity store (ADR 0035).

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
  - *localStorage and IndexedDB* are read by Goodfetch's Firefox (ADR 0019) on a temporary copy of the profile, over its pipe, with every request intercepted so that no site is contacted. The copy is removed when the import ends, which a conformance case checks.
  - *Source profile newer than Goodfetch's Firefox:* that part fails with `profile_directory_newer_than_browser`, and the cookies still import, with a warning.
- **Safari.** Cookies, localStorage and IndexedDB are in scope.
  - *Permission:* macOS allows reading them only after the person grants Full Disk Access. Without it, the import fails with `full_disk_access_required`, which explains how to grant it.
  - *Primary route:* Goodfetch's WebKit (ADR 0019) opens a copy of Safari's website data, as the Firefox route does, and the copy is removed the same way.
  - *Fallback for cookies:* an in-house decoder of `Cookies.binarycookies`, fuzzed, written because no crate is widely used (the best, `safari-binarycookies`, has about 12,000 downloads).

### A Profile directory, in place

A run whose source is `profile_directory` launches a browser on that directory:
- *Chromium family:* the installed browser that wrote it, by channel or path (ADR 0024), or the Chromium build that created it (Chrome for Testing or the Goodfetch build, ADRs 0024 and 0035).
- *Firefox:* Playwright's Firefox or Camoufox, only at the major version the profile's `compatibility.ini` records, which is the only part Firefox's downgrade check compares ([nsAppRunner.cpp](https://github.com/mozilla-firefox/firefox/blob/main/toolkit/xre/nsAppRunner.cpp)). Firefox cookies are not encrypted, so that build keeps them; Provenance records that the build differs from the one that wrote the profile.
- *WebKit:* only a directory WebKit created.
  > Amended by [ADR 0121](0121-service-workers-run-under-the-engines-interception-the-back-forward-cache-is-off-locators-reach-closed-shadow-roots-and-a-read-never-scrolls-a-list.md): a WebKit directory that holds a service worker registration is refused when its Session opens, with `capability_unavailable` naming `state_file`, since WebKit's `browser.service_workers: block` cannot keep that service worker from running and its requests would escape interception (ADR 0014). The same holds in any browser whose `block` cannot.

The rules for an in-place launch:
- **Lock and version.** The engine takes the browser's own lock and checks the directory's version before launching:
  - *Chromium family:* the singleton lock (`SingletonLock`, or `lockfile` on Windows), plus `Local State` and `Last Version`.
  - *Firefox:* `parent.lock` on Windows, `.parentlock` on macOS and Linux, plus `compatibility.ini`.

  It refuses with `profile_directory_in_use`, `profile_directory_browser_mismatch` or `profile_directory_newer_than_browser`. On Windows, a cookie file locked by a running browser counts as `profile_directory_in_use` and is never retried in a loop. On Firefox, a build of a newer major version than the profile's `compatibility.ini` is also refused with `profile_directory_browser_mismatch`, because launching it would upgrade the profile and Firefox's downgrade protection would then keep the person's own Firefox from opening it. Both Firefox refusals point to importing the same `profile_directory` with `goodfetch identity import`, which reads a copy, or to `installed_browser` for the browser's default profile on a local engine; behind a Browser host, where `installed_browser` is also used in place, they point to a file upload.
- **One Session.** A run on a Profile directory has one Session on it, in the run's Access mode (`standard` by default; `stealth` with the Goodfetch build (ADR 0035) on a directory it created, or with Camoufox; `identified` is refused, as at import), fixed for its life, with the launching browser's identity recorded in Provenance. Its language is the profile's own and its time zone that of the machine holding the directory, both kept under a proxy as for an import (above). **Changes ADR 0021 and ADR 0035.** The Session pool never rotates, retires or escalates it, and a failure that would do so raises a Human pause. **Changes ADR 0006 and ADR 0021**, whose pool rotates and retires and whose escalation opens a new Session, **and ADR 0035**, whose Session pool retires Sessions by age and use count. Recycling relaunches the same browser on the same directory. **Changes ADR 0040.**
  > Amended by [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md): the Human pause holds for a crawl's Session. When a program or an Agent opened the Session itself, a Challenge goes on down ADR 0035's ladder without its new `stealth` Session, and any other Block cause is reported on the call that met it.
- **The profile's own key store.** The launch never passes the mock-keychain or `basic` password-store flags that automation tools add by default, because they stop a real profile's cookies from decrypting ([browser-use#3225](https://github.com/browser-use/browser-use/issues/3225)).
- **Chrome's default data directory is refused**, pointing to `running_browser`.
- **Writes persist.** Cookies, storage and Consent prompt answers the run sets stay in the directory, as they would in the person's own use, marked in Provenance. **Changes ADR 0048.**
- **The reset gate.** The directory is the person's data, not Goodfetch state, so the reset gate neither removes nor scans it. Everything the run creates elsewhere is still removed. **Changes ADR 0001 and ADR 0015.**
- **Resuming elsewhere** (ADR 0002):
  - *Through a proxy, and not behind a Browser host:* allowed, because the Session reaches sites through the same exit. A Checkpoint of a Session on a Profile directory holds only the cookies and storage of its `domains`, and the resumed Session continues from those; Provenance records that the directory stayed behind. The resumed Session keeps its Access mode and is bound as an import of those cookies would be (above): for the Chromium family, an installed browser of the source's family, else ADR 0024's default Chromium with the family warning (the Goodfetch build under `stealth`); for Firefox, Goodfetch's Firefox (Camoufox under `stealth`); for WebKit, Goodfetch's WebKit; in each case with the language and time zone the Session had on the directory's machine (above).
  - *On direct egress, not behind a Browser host:* refused with `session_bound_to_machine`, since a new IP under the same cookies breaks ADR 0021.
  - *Behind a Browser host:* the directory never leaves the person's machine, so the run may resume on another engine or Group member and continues once the Browser host reconnects (below).

### Borrowed browsers

A Borrowed browser serves one run and one Session, in the context where the login lives: the browser's default context, or for `client_browser` the context of the borrowed page. It is never closed, and browser-wide settings such as download behaviour are never changed in it.

> Amended by [ADR 0089](0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md): through `run.borrow()`, each browser context a borrowed page belongs to is one Session of the run, joined by later pages of that context, so one run can borrow the pages a crawler opens in several contexts or browsers; a second run borrowing the same browser is still refused with `browser_borrowed`.

- **Work items:** they use it one at a time, each under its own lease (ADR 0003).
- **A second run** trying to borrow the same browser is refused with `browser_borrowed`, judged on the browser the connection reaches, not on the URL.
- **The Session pool** never rotates, retires or escalates its Session. A failure that would do so raises a Human pause. **Changes ADR 0006, ADR 0021 and ADR 0035.**
  > Amended by [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md): the Human pause holds for a crawl's Session. When a program or an Agent opened the Session itself, a Challenge goes on down ADR 0035's ladder without its new `stealth` Session, and any other Block cause is reported on the call that met it.
- **Identity:** the Session is `standard`, and its identity is the browser's own, recorded in Provenance.
- **Resources:** the governor neither recycles a Borrowed browser nor counts it in the memory allowance. **Changes ADR 0040.**
- **Refused Sessions:** a Session with a certificate exception (`certificate_exception_unsupported`, ADR 0042) or a local address (`local_address_unavailable`, ADR 0044).
- **Run state:** the Session's cookies and storage stay in the browser and are not copied into run state; a Checkpoint records only that the browser stayed behind. **Changes ADR 0006.**
- **What persists:** cookies, storage and Consent prompt answers the run sets stay in the person's browser, marked in Provenance. **Changes ADR 0001, ADR 0015 and ADR 0048.**

**The person's running browser.** Goodfetch uses Chrome 144's consented remote debugging ([Chrome blog](https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session)).
- *Setup:* the person enables it once at `chrome://inspect/#remote-debugging`, and the browser asks "Allow remote debugging?" for each WebSocket connection ([chrome-devtools-mcp#1794](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/1794)). ADR 0017's rejection of a debugging port covers the browsers Goodfetch launches; here the person turns the mode on and Chrome asks before each connection.
- *Finding it:* the browser is found through the `DevToolsActivePort` file in its profile, since its `/json/version` returns 404 in this mode ([chrome-devtools-mcp#2283](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/2283)). Chrome and Edge implement the mode; other Chromium-family browsers are probed.
- *One connection per run,* opened once after the person approves and held for the run's life. It is never reopened in a loop, because each attempt raises a dialog. While waiting for the person to enable the mode, the engine watches `DevToolsActivePort` for changes rather than polling it.
- *Tabs:* Goodfetch opens its own tabs and never touches the person's other tabs.
- *Refusals:*
  - when an administrator has turned remote debugging off, `remote_debugging_disabled_by_policy`;
  - a running Firefox or Safari, with `running_browser_unsupported`, because neither offers a way in.

**A browser the person's own code launched.** This is the reverse of ADR 0019, where the person's Playwright attaches to Goodfetch's browser.
- *The SDK relay:* `run.borrow(pageOrBrowser)`, in the TypeScript SDK for Playwright and Puppeteer, and in the Python SDK for Playwright's sync and async APIs.

  > Amended by [ADR 0078](0078-an-sdk-program-opens-an-explicit-engine-connection-and-closes-only-what-it-started.md): `borrow` is called on the Run whose Session it serves, never on a process-wide `goodfetch`; it returns a Goodfetch Page for a page and a Session for a browser, and closing either ends the borrow.

  - It opens a CDP session from the person's object (`newBrowserCDPSession`, `newCDPSession`, `createCDPSession`) and carries CDP over the JSON-RPC connection the SDK already holds.
  - Commands go to the browser under ADR 0019's per-command timeouts. The browser's events go to the engine under ADR 0024's credits, so a busy page cannot flood the connection.
  - `$/cancel` ends the borrow.
  - No port is needed, so it works on the pipe-launched browsers Playwright starts by default.
- *Through `cdp_url`:* Selenium 4 (through its `se:cdp` address), the Rust crate and the CLI.
- *Refused:* Playwright's Firefox and WebKit pages, which expose no CDP, with `borrow_requires_cdp`. Pyppeteer is not supported, because it is unmaintained.

**The person's code is a raw CDP client (ADR 0017).** On `client_browser`, on `running_browser` (which the person and their other tools, such as Chrome DevTools MCP or a browser extension, may drive too), and on a `cdp_url` the person's own code also drives, other code holds its own CDP session and can act outside the Effect journal and switch off interception. So:
- the engine refuses to type secrets;
- results are marked unguarded, and Provenance marks the run as sharing its browser;
- the egress check and the domain policy are interception only, which that code can switch off, and Provenance records that neither is guaranteed. **Changes ADR 0014.**
- that code's commands go outside any endpoint Goodfetch serves, so no lease fences them. **Changes ADR 0003.**

> Amended by [ADR 0114](0114-a-program-and-collie-take-turns-on-a-page-other-code-drives-and-an-ai-verb-fails-when-a-navigation-it-did-not-start-replaces-its-document.md): on such a browser the program and Collie take turns on a page, since nothing can hold it for a Collie call; an AI verb whose document a navigation it did not start replaces, between its first read of the page and its return, fails with `page_navigated_during_call`, while the other code's input within one document stays outside what the engine sees. `run.borrow()` gains no `exclusive` option.

A `cdp_url` the engine alone drives, such as a cloud browser, keeps ADR 0022's guarded path. The engine cannot see other CDP clients, so a `cdp_url` set by the developer's own code or at the run layer is treated as shared unless that code declares `exclusive: true`, as it does for a cloud browser it only creates and hands over. `exclusive` is honoured only on a `cdp_url` from the person's own code or CLI. On a run started by a Client acting for a model, a run-layer `cdp_url` is treated as shared whatever it declares, so no Secret is ever typed into a browser a model may also drive (ADR 0014). In every run, a model's or a Recipe's included, a `cdp_url` set in Operator policy or in user and project configuration, or a browser a browser backend Plugin creates (ADR 0033), is treated as driven by the engine alone. **Changes ADR 0022.**

**Egress on every Borrowed browser.** No egress proxy can be put under a browser that is already running, so the engine resolves each intercepted request's host itself and blocks private ranges. The browser resolves names itself, so a DNS-rebinding gap remains, and it is recorded in Provenance. **Changes ADR 0017**, whose connect-time check this replaces for Borrowed browsers.

**Which `cdp_url` addresses are allowed.** As for Plugin endpoints (ADRs 0017 and 0033), a `cdp_url` counts as operator-configured when it is set in Operator policy or in user and project configuration, or is a run value that Operator policy allows; a value from the person's own code or CLI, but not one from a Client acting for a model, may also use loopback, except on an Exposed Daemon. Otherwise it is treated as an address a page can reach, and private ranges are refused.

> Amended by [ADR 0115](0115-a-cdp-url-the-engine-cannot-use-and-a-missing-browser-build-fail-with-one-error-code-per-remedy.md): an address these rules refuse raises `cdp_url_not_allowed` (a `RefusedError`). Opening a Session on a `cdp_url` otherwise fails with `cdp_url_unreachable` (an `UnavailableError`, retryable) when nothing answers, `cdp_url_not_browser` (a `ProgramError`) when what answers is not a browser speaking CDP, and `cdp_url_unauthorized` (a `RefusedError`) on a 401 or 403; each names the address, and none is retried until a timeout.

> Note (agent decision, accepted by the owner on 2026-10-28, with the deep review's O8 and O9), 2026-10-24, [#633](https://github.com/goodfetch/goodfetch/issues/633): the address rules are `goodfetch-engine`'s `egress::judge_cdp_url`, run on every address the `cdp_url`'s host resolves to (the Run's `network.resolve` first) before any connection, which then tries only those addresses. A value from Operator policy or user configuration is an Operator-configured endpoint (any address, unless `egress.no_private_endpoints`), and so is one from project configuration, unless a Client acting for a model sent it or opens the Session: that Client sends the project configuration itself, so its value there is the model's, judged as an address a page can reach, loopback refused (owner, 2026-10-28, O8; this changes the paragraph above, which counted project configuration as operator-configured in a model's run too, and ADR 0115's fix, which now reads "set the address in Operator policy or in user configuration, or give a public address"). A run-layer value is the program's own unless a Client acting for a model gave it or the call that opens the Session comes from one, so a model opening a Session from a Run's `session.source` is judged as the model; a Recipe's is never the program's own. Each effective setting keeps whether a Client acting for a model gave it, so a model's value stays the model's whichever Client opens the Session. The program's own may use loopback (an IPv4-mapped loopback address included); every other address that is not globally reachable is judged as one a page can reach, so `egress.private_ranges` can allow it, but loopback from a model or a Recipe is refused whatever the ranges. `cdp_url_not_allowed` details `{ url, layer, rule: "loopback" | "private_address", address, setting?, fix }`, `url` without its query, fragment or credentials. "A run value that Operator policy allows" and the Exposed Daemon row (A04-56) are not built yet. On a Borrowed browser, each request the interception sends on is judged again on the addresses the engine resolves its host to, through the Run's `network.resolve`. A name the engine cannot resolve is left to a browser on the engine's machine, one whose `cdp_url` resolves to loopback alone, and refused on any other with `egress_refused` (`rule: "domain_policy"`, a request that could not be judged, `setting: "network.resolve"`), since an intranet name or a cloud's metadata name such as `metadata.google.internal` resolves only on the browser's network, an address the engine cannot judge (owner, 2026-10-28, O9); with `security.resolved_ip_check` off, which judges no address, it is left to the browser too. The Session's Provenance records `{ action: "interception_only", egress: "interception_only", gap: "dns_rebinding" }` once it opens. A refused navigation may still have opened a TCP connection the browser preconnected, which carries no request: the engine never changes a Borrowed browser's preferences. A Borrowed Session takes no `proxy.*` (`capability_unavailable`), and this engine reaches only a browser's own `ws://…/devtools/browser/<id>` address; the probe, `wss://`, `headers` and the probe's Error codes are the `cdp_url` Session's (#743).

**Audit.** Every import and every borrow is recorded in the Audit log.

**Third-party cloud browsers** that Goodfetch reaches by URL, without creating them, are Borrowed browsers. A cloud browser that a browser backend Plugin creates (ADR 0033) is released by that Plugin.

### Who may bring a login in

- **A person's own code** (a script, an SDK, the CLI) needs no extra approval, because running the code is the approval. A browser's own dialog still appears.
- **A Recipe or a Plugin is not the person's own code.** A Recipe run whose source is `profile_directory`, `installed_browser`, `running_browser` or `cdp_url` waits for a person's approval, as a Client acting for a model does.
  > Amended by [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md): a Recipe run also waits for a person's approval before it saves, imports, deletes or exports a Saved identity, whatever its source, so a third-party Recipe cannot replace a person's login with another.
- **A Client acting for a model** (MCP, an agent, code mode) waits for a person's approval before any import or borrow, through a Human pause in the Console, the CLI or a Notification. For a running browser, the browser's own "Allow remote debugging?" dialog counts as approval of the connection; `domains` chosen by a Client acting for a model are still approved in the Human pause. The reason is session fixation: a prompt-injected agent could import an attacker's cookies, and the person's later work would land in the attacker's account.
  > Amended by [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md): it also waits for a person's approval before it saves, deletes or exports a Saved identity, so it cannot replace a person's login with another.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **An Integration** (ADR 0027), whose run values a workflow, an inbound webhook or a model may fill in (ADR 0033), is treated as a Client acting for a model: its imports and borrows wait for a person's approval, it cannot declare `exclusive`, and its `cdp_url` may not use loopback.
- **Navigation on a borrowed running browser, on a `cdp_url` run that names `domains` (required for a Client acting for a model or a Recipe run), or on a Profile directory used in place, is bound to its source's `domains`,** which a person approves.
  - The list covers top-level navigations and new tabs. Subresources and frames follow their page, so a payment or CAPTCHA frame on an allowed page loads.
  - An unlisted navigation, such as a login redirect to an identity provider, is handled by who started the run:
    - for a Client acting for a model or a Recipe run, a Human pause asks the person to add the domain;
    - for the person's own code, it fails with `domain_not_allowed`.
- **Operator policy (ADR 0028) can lock which source kinds are allowed.** On a Daemon with accounts (ADR 0031), Exposed Daemons included (ADR 0049), `profile_directory`, `installed_browser`, `running_browser` and `cdp_url` are off by default when they would be read on the Daemon's own machine, because they would reach that account's browser or network for anyone holding a token. Served by a Browser host the person runs (below), they stay available. On a Daemon with accounts, `state_file` reads only content the Client uploads, never a server path; a Daemon group always has accounts, so a `state_file` path is refused there too.
- **Changes ADR 0031:** on a Daemon with accounts, the clear-text `storageState` export and widening a source's `domains` go through step-up, and only the operator role writes the `cdp_url` addresses used as operator-configured endpoints.
- **Daemon groups.** In a Daemon group, a run whose source is `profile_directory`, `installed_browser`, `running_browser` or a loopback `cdp_url` runs only on the Group member where that source exists (a source served by a person's Browser host is placed on any Group member, and the Browser host connects to it), a `client_browser` run on the Group member holding the SDK's connection, and a run using an import bound to its machine on that machine's Group member; the paths of those not served by a Browser host take the *group member* scope (ADR 0028). A Partitioned crawl (ADR 0054) of more than one Partition whose source is one of these, `client_browser` included, or a machine-bound import, or a run-only import (`state_file` upload or `cookie_header`) on direct egress, is refused at start with `session_bound_to_machine`, and one on any other Borrowed browser (a non-loopback `cdp_url`) with `browser_borrowed`. **Changes ADR 0054 and ADR 0055.**

### A remote engine and the person's browser

When the engine runs on a shared Daemon or in Goodfetch Cloud, the person runs a Browser host (ADR 0022) on their own machine, beside their running browser or a Profile directory. The Browser host carries out the Session's operations beside the browser, behind the guard.
- **The connection runs outward:** the Browser host connects out to the engine, and the person's browser never accepts an inbound connection. The rules above apply, and the Browser host's token is scoped to that one Session.
- **Nothing is wiped:** the Browser host wipes nothing. It never closes a Borrowed browser; a browser it launched on a Profile directory it closes, leaving the directory intact.
- **Installed browsers:** through a Browser host, `installed_browser` is used in place as `profile_directory` is, or as `running_browser` when the browser is running. An import of the person's own browser or Profile directory into a remote engine's Saved identity goes only through a file the person uploads or ADR 0015's session export of an identity not bound to its machine.
- **Cookies stay local:** the Session's cookies and storage are not copied back into run state. A Checkpoint records only that the browser stayed behind, so the run resumes only through that Browser host. **Changes ADR 0022.**
- **Events and cancellation:** the Browser host's events flow under ADR 0024's credits, and `$/cancel` ends the borrow.
- **When the machine sleeps or goes offline:**
  - the Session stops taking work and the run's frontier holds its pending items;
  - the work item's lease expires as ADR 0003 has it, and the item is retried once the browser is back;
  - the run waits in a Human pause ("your browser is offline") instead of failing;
  - it resumes when the Browser host reconnects; after the run moves, the Browser host reconnects to the resuming engine and authenticates as the person who runs it: in a Daemon group, through any Group member, as a Client does after `run_moved` (ADR 0055), with that person's account or named token (ADR 0031); in Goodfetch Cloud, through its gateway (ADR 0031), as Goodfetch Cloud's map designs. That engine issues it a new token scoped to the Session.

This is the only way a remote run can use a device-bound login, and it keeps the person's cookies on their own machine.

### When an imported login expires

- **What a Saved identity records:** its source (kind, browser, profile and domains), when it was imported, and the earliest cookie expiry per domain.
- **Changes ADR 0006:** the recovery ladder gains **re-import from source**, right after the declared refresh.
  - *Unattended:* only when the source can be read without a person. That means a closed, non-default Chromium data directory, a Firefox profile, or Safari once Full Disk Access is granted.
  - *Needing a person:* a running browser, Chrome's default data directory, or a busy source (`profile_directory_in_use`). The step becomes part of the Human pause, where the person approves a re-import or logs in through the Live view.
  - *Started by a Client acting for a model, or in a Recipe run:* always the Human pause.
  - *Budget:* re-import counts toward the limit of two recoveries per page.
  - *Writing:* a re-import writes the Saved identity under ADR 0035's install-wide lock, a lease in a Daemon group.
  - *File imports,* whose file is not kept, *and run-only imports,* which keep no Saved identity, have no source to read again and run the ladder as before.
- **Cookie expiry warnings.** A cookie's expiry is not the site's session expiry, so the warnings say "cookies expire on…", and only the Page check judges whether a login is still live.
  - *On a Daemon:* a Notification (ADR 0027) and a Console badge appear when the earliest expiry falls inside a window, by default 3 days, a setting. The Daemon schedules this as a timer at expiry minus the window. In a Daemon group, the Lead Daemon, which keeps Saved identities, runs the timers and sends each Notification once. **Changes ADR 0027 and ADR 0055.**
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - *On an Attached engine:* a warning at run start.
  - *In listings:* `goodfetch identity list` and its MCP operation return each identity's expiry.

### Other changes

- **Changes ADR 0001:** a Profile directory used in place and a Borrowed browser are outside the reset gate, as the person's own data. Saved identities remain the only Goodfetch state excepted from the reset rule.
- **Changes ADR 0002 and ADR 0055:** a run bound to its machine does not resume elsewhere. That covers `session_bound_to_machine` runs and runs on a Borrowed browser reached on the engine's machine (`running_browser`, a loopback `cdp_url`) or through its SDK connection (`client_browser`). When its Group member is removed, a run bound to its machine stays checkpointed, still assigned to that Group member, and resumes only when it comes back or a person discards it; `goodfetch runs adopt` refuses it with `session_bound_to_machine`. A run behind a person's Browser host is bound to that Browser host instead: it may resume on another engine or Group member, and continues once the Browser host reconnects to it.
- **Changes ADR 0033:** its `session_credentials` permission covers every source of this ADR.
- **Changes ADR 0035 and ADR 0045:** `goodfetch identity` now manages Saved identities (`import`, `export`, `list` and `delete`). The Web Bot Auth key commands become `goodfetch signing-key create` and `goodfetch signing-key directory`, which **changes ADR 0021** too.
- **Changes ADR 0037:**
  - Rows SA-08, SA-11, pain 4 and pain 12 become `decided`.
  - SA-21 and CO-13 cite it, and the area table lists it in areas 3–9 and 11–14.
  - Site access, Client surfaces and Self-hosted deployment no longer wait on this ticket.
- **To prove in the spec:**
  - the Safari primary route, by prototype;
  - which Chromium-family browsers other than Chrome and Edge implement the consented mode, and that `DevToolsActivePort` appears on the default data directory ([chrome-devtools-mcp#2283](https://github.com/ChromeDevTools/chrome-devtools-mcp/issues/2283)), by probe;
  - the Python sync relay, since Playwright's sync objects are bound to the thread that created them, while ADR 0038's sync API runs its core on its own thread;
  - the conformance cases:
    - each refusal code;
    - the reset gate leaving a Profile directory intact;
    - the egress check on a Borrowed browser;
    - the Human pause on a model's or a Recipe's import.
