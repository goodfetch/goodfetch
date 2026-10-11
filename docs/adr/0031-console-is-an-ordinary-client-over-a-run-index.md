# The Console is an ordinary Client: one Console per user sees every run through a Run index

The Console reads and acts through the same JSON-RPC protocol and Operation catalog as every other Client (ADRs 0011 and 0024). It carries them over a WebSocket on its own origin, authenticated by the `HttpOnly` cookie of ADR 0024, and fetches large files (page bodies, screenshots, Exports) by handle with plain HTTP `GET`, as a remote Client streams `page.body`. Its `hello` names the surface `console`, which gives the Audit log the surface ADR 0029 records with each reconciliation. One Console per user and per product major (ADR 0030) sees every run on the machine:
- **The Run index lists runs.** It is a small per-user, per-major SQLite catalog that every engine, Attached or Daemon, writes when a run starts and when one of its three axes changes. Each row holds the run's ID, file location, Recipe, Lifecycle, Verdict, Completeness, timestamps, a Usage summary and, while the run is live, the engine that owns it.
- **The run files stay the truth.** `automan runs reindex` rebuilds the index by scanning them. A row whose file is gone shows as missing; it is never silently dropped.
- **Committed state comes from the file.** The Daemon reads any run's file through SQLite WAL readers, live or not, so the single writer of ADR 0002 holds.
- **Live actions go to the owner.** On a live run it does not own, the Daemon forwards each live action to the owning engine. That covers takeover, confirmation, reconciliation, the event stream and live frames. It reaches an Attached engine through the private endpoint ADR 0024 gives it for its workers; the index records that endpoint and its token in a `0600` file.
- **An Attached engine can start the Daemon.** When a Human pause or a Notification needs a link and no Console is running, it starts the Daemon on demand. A script's own Console (ADR 0024) remains for people who want no Daemon.

Decided in [Console data access](https://github.com/Jbmanllr/automan/issues/104).

Evidence from the competitor survey made for that ticket:
- Temporal, Prefect and Hatchet serve their UI from the same API as their SDKs. Dagster's GraphQL is "primarily for internal use by the Dagster webserver", and n8n keeps an internal API beside its public one.
- MLflow names its unindexed file store as the usual cause of a slow UI.
- No live-view vendor documents a control lock between automation and a person.

## Considered Options

- **ADR 0027's HTTP listener as the Console's transport**: rejected. It is off by default and built for workflow tools, so the Console would break whenever it is off.
- **A private API for the Console** (REST or GraphQL, as Dagster and n8n have): rejected. It is a second API to keep in step with the catalog, and Automan Cloud would have to copy it.
- **Scanning run directories to list runs** (TensorBoard's log directories): rejected. It gets slower with every run and misses runs stored outside the scanned folder.
- **One Console per engine**: rejected. Two scripts would give two Consoles on two ports, each with its own "Needs you" list.
- **Short-lived signed live-view links in Notifications** (Browserbase, Browserless, Anchor): rejected. A link that grants control sits in chat history and is forwarded with messages.
- **Answering pauses and confirmations with chat-app buttons**: rejected. A Slack or Teams button identifies a chat user, not an Automan person who can verify a passkey.
- **An in-page recorder such as rrweb** (Browserbase's recordings): rejected. Injecting script into the page is what the guard forbids (ADR 0019).

## Consequences

**Where runs live.**
- **Default folder.** Run files and directories go by default to a per-user, per-major data folder, such as `~/.local/share/automan/1/runs/<run-id>/` and its macOS and Windows equivalents. The location is a setting (ADR 0028), and the index finds runs wherever they are.
- **Changes ADR 0002:** the per-run layout gains this default location and the Run index.

**Events and reconnects.**
- **Positions.** Every event carries a position that only goes up. The Console keeps one position per engine and resubscribes from it after a reconnect. When the engine no longer holds that position, it sends a fresh snapshot of the screen's state and continues from there.
- **No loss.** Lossless events (records, axis changes, "Needs you" items) are never missed across a reconnect.
- **Fallback.** Where a WebSocket cannot be opened, the Console falls back to Server-Sent Events plus `POST`, carrying the same messages.

**Runs that no engine is running.**
- **Taking the writer role.** To write to such a run, the Daemon takes the next run epoch (ADR 0003). While a live owner holds the run, the Daemon forwards the action to that owner instead.
- **Answers that need no user code** apply at once, such as "it happened", "leave uncertain" or an erasure.
- **Answers that need handlers.** "It did not happen" and "raise the Limit and resume" resume a Recipe run in the Daemon. They mark an SDK script's run as waiting for its script, which resumes it the next time it resumes that run ID. The Console says so and offers **Copy resume command**.
- **A script that ends.** A Human pause pending when the script ends stops with the run.

**The live view.**
- **A channel of its own.** It is a per-Session stream, opened only while someone watches and kept apart from the credit-based event stream. The newest frame wins. A person's input travels on an ordered path where nothing is dropped.
- **Tiers, chosen automatically and each settable:**
  1. **The real window.** When the browser is headful on the person's own desktop, "Take control" can bring that window to the front.
  2. **CDP screencast** everywhere else, the default for headless browsers. It sends JPEG frames sized by `devicePixelRatio` under acknowledgement-based flow control. For remote viewers the frames are re-encoded into a WebRTC video stream.
  3. **WebRTC H.264.** Where the browser runs headful in a virtual display (Linux containers, the OCI image, a Browser host, Automan Cloud), the engine captures that display at about 25 fps and falls back to tier 2. Steel moved from screencast ("4-12fps, unstable") to this.
  
  Input goes through CDP (`Input.dispatch*`, `Input.insertText`, `Input.imeSetComposition`).
- **The Session, not one tab.**
  - **Tabs:** it lists the Session's tabs and follows new popups.
  - **Dialogs:** native dialogs and basic-auth prompts become Console dialogs.
  - **Files:** the page's file chooser opens a Console file picker, whose file lands in the run's directory; downloads appear in the drawer.
  - **Clipboard:** paste and copy need an explicit gesture.
  - **Keyboard:** keys carry both `code` and `key`, so non-QWERTY layouts work, and IME composition is forwarded.
  - **Phones:** touch input and a pop-up keyboard field.
- **One holder at a time.**
  - **Watching and control.** Any number of people can watch. "Take control" locks the Session for one person, and automation, already waiting in its Human pause, resumes only after "Hand back" and a passing Page check, or when the pause is resolved.
  - **Idle holder.** After 2 minutes without input the holder is warned. After 3, control returns to *waiting*, never to automation.
  - **Taking over.** Another person may ask for control, which is granted after 30 s without an answer. An operator can force it.
  - **Timings** are settings.
  - **Audit.** Taking control, handing back and the start and end of input go to the Audit log. Keystrokes never do.
- **Embeddable in users' own products**, as Browserbase, Steel, Hyperbrowser and Kernel allow.
  - **The grant.** `liveView.grant(session, mode, ttl, origins)` returns a token scoped to one Session. It is short-lived, single-use for opening the connection, tied to its origins through `frame-ancestors`, and revoked when the Session ends or by `liveView.revoke`.
  - **View-only is enforced by the engine,** which drops the viewer's input. Browser Use's and Anchor's view-only modes are only drawn in the embedding page.
  - **Limits, audit and Usage.** The number of viewers is a setting, grants are audited, and live-view minutes count as Usage.
  - **Packages.** A `<automan-live-view>` web component and a React wrapper ship with connection, control and Session events.

**Run traces.**
- **What is recorded,** all on the CDP side with nothing injected into the page: the screencast as low-frame-rate video, a DOM snapshot at every action and Effect, the network log as HAR, and console messages. Playwright's trace has the same model. HTTP-tier runs record the network part only.
- **Replay and export.** The run's Timeline replays the trace. It is exported in Playwright's trace format only if a conformance spike proves that format can be produced reliably, since it is undocumented.
- **Masking.** Password fields and values filled from Secret references are masked everywhere, and typed input is masked by default.
- **Retention.**
  - **Kept by default** for runs that are `failed`, `rejected` or `needs-review`, or that had a takeover or an Effect. A takeover is always recorded.
  - **Budget.** Each run gets 1 GB by default. Past it, the oldest video goes first, while snapshots and the HAR are kept.
  - **Erasure.** Traces are erased with their run.
- **Cost.** Traces are a costly control (ADR 0023).

**People and tokens.**
- **Changes ADRs 0017 and 0024**, which gave every endpoint one per-launch token.
- **Single user.** On a single-user local Daemon, the OS user who owns it is the person: no login beyond the cookie, and the Audit log records that user and the surface.
- **Shared or exposed Daemon.**
  - **Accounts and roles.** Per-person accounts sign in with passkeys, or optionally OIDC single sign-on, under three roles. A *viewer* reads. A *member* also runs, answers Human pauses, confirms and reconciles. An *operator* also edits the Operator policy and accounts.
  - **Recovery and teams.** Recovery codes and an operator reset cover lost passkeys, and OIDC groups map to roles.
- **Web protections.** Cookies are `SameSite=Strict`, the `POST` fallback carries a CSRF token, and sign-in is rate-limited.
- **Programs.** Programs use named tokens, each limited to certain operations, Recipes, runs or Datasets, with an expiry, and revocable in Engine & settings. The per-launch token stays for local Clients that find the Daemon through its runtime file.
- **Step-up.** Confirming an Effect (ADR 0014) or reconciling one (ADR 0029) asks for the person's passkey again.

**Settings** (changes ADR 0028).
- **What the Console edits.** It edits user-level values and a run's own values, and shows every effective value with its layer and lock. It never edits project configuration or Recipe manifests, because those are files under version control.
- **The Operator policy.** The operator role edits it in the Console. The Console writes the file atomically, and only if its digest has not changed since it was read, so a hand edit is never lost.
- **Managed policies and audit.** A policy marked *managed* (deployed by Ansible or MDM) is read-only in the Console. Every change goes to the Audit log with a before-and-after diff.
- **When changes apply.** A tightened cap reaches running runs at their next admission point. Other changes apply to new runs, and each run report names the policy version it ran under.

**Availability.**
- **Idle stop.** A visible Console tab counts as a connected Client, so the Daemon does not stop idle while someone is looking. A hidden tab does not count.
- **Socket activation.** `automan daemon install` registers launchd or systemd socket activation, so the first connection to the Console or the protocol starts the Daemon. On Windows, the installed service stays running.
- **Logins survive restarts.** A per-user Console key in a `0600` file signs Console logins. They expire after 30 idle days and are revocable, and rotating the key signs everyone out. The protocol socket keeps the per-launch token.
- **While disconnected,** the tab stays read-only under a banner and reconnects as above.
- **Offline.** Run summaries and metadata are cached in the browser per major. Records are cached only when the person keeps a view offline, encrypted under a key that expires with the Console session. Secret fields are never cached. Sign-out wipes the cache, and Dataset erasures reach it as soon as a tab reconnects. A setting switches caching off.

**Records at scale.**
- **Paging.** Records page by key, with filtering, sorting and column selection done in the engine.
- **Search across runs** runs on DuckDB inside the engine, reading run files read-only, as ADR 0015 allowed by not using SQLCipher. Each run keeps a SQLite FTS5 index for full-text search.
- **The SQL tab.** The Datasets screen offers a read-only SQL tab over the selected runs. DuckDB's external access is off and its configuration locked, so a query reads only those run files. Queries have time and memory limits and secret columns are hidden. The SQL tab is a catalog read, so every surface gets it.

**Reaching the Console remotely.**
- **Links.** Notification links carry no credential. They point to the Console address the person sets, such as an exposed Daemon's TLS address.
- **Remote access** means exposing the Daemon with TLS and passkeys (ADR 0017). The docs give tested setups for Tailscale, Cloudflare Tunnel and a reverse proxy.
- **Out of scope.** A relay through Automan Cloud belongs to the Cloud map.

**Automan Cloud.**
- **One contract.** The Console calls only catalog operations, including `runs.list` over the Run index, and takes its endpoint and authentication as configuration.
- **The gateway.** Automan Cloud gives it a gateway that implements the same operations across its per-run engines (ADR 0001), reading its own index and forwarding live calls as the Daemon does.
- **Testing.** One Console conformance suite runs against a Daemon and a stub gateway.
- **Out of scope.** A hosted Console reaching a local Daemon belongs to the Cloud map.

**Hardening.**
- **Scraped content on its own origin.** Scraped HTML, screenshots and Record previews are served from a separate origin and shown in sandboxed iframes with no cookies, so a hostile page can never act as the Console.
- **Content Security Policy.** The Console runs under a strict policy: its own scripts only, no inline code or `eval`, Trusted Types, embedded assets, and no third-party or telemetry calls.
- **Headers.**
  - `frame-ancestors 'none'` everywhere except the embed endpoint, for granted origins only;
  - `Referrer-Policy: no-referrer`;
  - COOP and CORP.
- **Checks.** CI checks these headers. The Proving ground (ADR 0025) includes pages that try to escape the content origin.
