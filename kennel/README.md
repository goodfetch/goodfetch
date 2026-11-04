# The Kennel

The Kennel is Goodfetch's local tool for watching and controlling the Runs of a machine's Daemon, from the menu bar. It is published apart from the Products, under its own version, and is part of no Product ([ADR 0141](../docs/adr/0141-the-kennel-is-a-tray-tool-of-its-own-workspace-that-reaches-the-daemon-as-an-observing-person-surface-and-is-published-apart-from-the-products.md)).

## Run it

`just kennel`, from the repository's root.

- **On macOS** it builds the release executable, assembles `Kennel.app` (bundle identifier `dev.goodfetch.kennel`), signs it ad hoc, installs it by rename into `~/Applications` and opens it with `open`. A Kennel already running is quit first.
  - Always start the Kennel through LaunchServices: with `open`, the Finder or a login item. Run directly, the binary inside the `.app` is refused desktop notices.
  - The bundle identifier stays the same across builds, so a rebuild keeps the notice permission.
- **On Linux and Windows** it builds and runs the executable.
  - On Linux the build needs the development files of GTK 3 and libdbus (`libgtk-3-dev` and `libdbus-1-dev` on Debian and Ubuntu), through `pkg-config`. AppIndicator's library is loaded at run time.
  - On Linux the icon goes in the panel through AppIndicator, which needs GTK 3 and an AppIndicator library (`libayatana-appindicator3-1` or `libappindicator3-1`), and a panel that shows AppIndicator icons: on GNOME, the AppIndicator extension. It needs no Linux setup of its own beyond that, and has no polish of its own (ADR 0141).

The icon shows one verdict for the Daemon, worst first, with no numbers:

| Verdict | Icon |
|---|---|
| No Daemon is reachable | a ring |
| A Needs you item waits | a disc with an exclamation mark cut out |
| A Run failed or was stopped by a Limit | a disc with a bar cut across it |
| A Run runs | a ring with its left half filled |
| Idle | a filled disc |

A left or a right click opens the menu. It shows the Daemon's state, then its live Runs, then the Needs you items, then "Open details" and Quit. With no Daemon running, the menu offers "Start Daemon".

- **Each Run** shows its Recipe, or its Run ID when it ran none, and its Lifecycle. Its progress message goes on the line below. On a Foxhound Daemon, a crawl also shows its succeeded, failed and pending counts.
- **What it lists.** The menu lists at most 10 Runs and counts the others. A Run that failed or was stopped by a Limit comes first. It stays in the menu and in the icon until the menu that showed it closes.
- **Needs you.** The menu lists the Daemon's Needs you items, the longest waiting first, at most 10, and counts the others. Each item opens a submenu that says what waits and offers the actions the item allows, each through its operation:

  | Action | Operation | Offered |
  |---|---|---|
  | Raise the Limit | `usage.raise` | to twice the amount the Limit allows now, the one amount a menu can offer without asking for a number; not for a Limit of nothing, which the CLI raises (`usage raise`) |
  | Accept the consent walls of a domain | `needs_you.allow_consent_domain` | when the item allows it |
  | Deny a domain | `needs_you.deny_wall_domain` | when the item allows it, with the reason the Audit log records: "Denied by a person from the Kennel's menu" |
  | Remove a domain from the refetch domains | `needs_you.remove_refetch_domain` | unless only `*` lists the domain, which takes the list of domains that replace `*`: the CLI changes it |
  | Dismiss | `needs_you.dismiss` | when the item allows it |

  Answering a changed certificate, answering a Human pause and reconciling are not offered from the menu, nor is an item's resume: a Run resumes from its own submenu. An action the engine refuses, or one no Daemon received, shows under the Daemon's state with the Error's message until the menu that showed it closes. An action that succeeds shows in the list: its item leaves it.
- **Fresh and live.** The menu is rebuilt from the latest data when the click starts, before it opens, and it keeps updating while it stays open.
- **Redraws.** There is at most one redraw a second, and no redraw of a hidden menu.
- **On Linux** the menu follows the same view as on macOS. AppIndicator keeps the first menu it is given, so the Kennel changes that menu's items in place: a row that keeps its kind keeps its item, and only a row of another kind, or a change in the number of rows, adds or removes items. An open submenu stays open while its Run's text changes. A left and a right click open the menu as the panel decides; AppIndicator reports neither.
- **Run controls.** Each listed Run has a submenu of controls, each labelled with what it does:

  | Control | What it does | Offered for | Asks first |
  |---|---|---|---|
  | Pause | `run.pause`: a Checkpoint and a stop | a running Run | never |
  | Stop | `run.stop`: a Checkpoint, and the Run can resume | a running Run | by default (`confirm_stop`) |
  | Abort | `run.abort`: no attempt in flight commits; not immediate | a running Run | always |
  | Retry failed | `run.retry_failed`: the crawl's failed requests run again | a running crawl, on a Foxhound Daemon | always |
  | Resume | `run.resume` on the Daemon | a paused Run, or one stopped by a Limit, that a Recipe runs | never |
  | Copy resume command | copies `goodfetch-<product> run resume <run-id>`, for the Daemon's Product | a paused Run, or one stopped by a Limit, that a script runs: only its program can run it (ADR 0031) | never |

  - A confirmation is the platform's own dialog, naming the Run and the consequence: GTK's on Linux.
  - A Run the person or a program paused stays in the menu as "Paused", with its Resume or "Copy resume command", until it runs again or ends. After a reconnection the Daemon lists only its live Runs, so a paused Run leaves the menu then.
  - Retry failed is not offered on an ended crawl: the engine would make the Connection that retries serve the crawl's handlers, which the Kennel cannot.
  - A failed Run offers no control.
  - A control the Daemon refuses shows its reason under the Daemon's state, such as "Pausing nightly failed: …", as a Needs you action's does.
  - Quitting the Kennel stops none of the Runs, those it resumed included.
- **Open details.** It opens the details window. While the window is open, it brings the window forward on macOS.
- **Shown text.** Text from the Daemon, a Run's program or a crawled site is shown as plain text. Control and bidirectional characters are stripped and long text is truncated. Credentials are masked as `••••`: a URL's password, a credential's query parameter (`token`, `password`, `api_key`, a signed URL's signature) and the value after `Authorization:`. The engine already masks the Secret values a Run resolved.

## The details window

"Open details" starts the window as a second process of the same executable, `goodfetch-kennel window`, with `eframe` on glow. The process exits when the window closes, so a closed window costs nothing.

- **Its own Connection.** The window reads `daemon.json` itself and connects as its own `kennel` Connection, which only observes. The tray passes it nothing but the argument `window`, never the token.
- **The Runs table** lists the Run index's rows, newest first, filtered by a text (found in a Run ID or a tag, the Recipe's among them) and a Lifecycle. It is listed again after each resync and each Run that starts or ends.
- **A chosen Run.** A click on a Run shows its progress, its Usage with the Limits that apply to it, a crawl's failures by Error code, and its Run report. A live Run's report comes when it ends. A second click puts the Run away.
- **Add seeds.** A live crawl on a Foxhound Daemon offers Add seeds under its report, while `controls` is on: a Label, the Label of the handler the seeds go to, and URLs, one a line.
  - **The confirmation.** "Add seeds…" asks the platform's own dialog first, naming the Run, how many seeds, the Label and the consequence: the crawl fetches them beside its other requests, a seed it already holds is not added again, and adding cannot be undone.
  - **The call.** Once confirmed, the window streams the seeds to `run.add_seeds` within the credits the call grants, then shows how many were added and, by reason, how many were not, such as `duplicate` or `invalid_url`. A refusal shows its reason.
  - **Not offered** on a Collie Daemon, for a Run that is no crawl or no longer runs, while `controls` is off, or before the window has read `kennel.toml`. A confirmation given before `controls` went off adds nothing.
- **Other views.** Beside the Runs, which `window` switches with the window itself, the window offers a view per setting of `[views]`, each one off neither shown nor read:

  | View | Shows | Reads |
  |---|---|---|
  | Records | the chosen Run's Records, 50 at a time, each Dataset's in order: dataset, number, URL, commit time and value. Foxhound's: hidden on a Collie Daemon | `dataset.query` over `records('<run>')`, when the view shows, at each page and at "Refresh"; a query no longer wanted is cancelled |
  | Daemon settings | each setting's value, the layer that set it (for the Run's layer, where it came from), the layer that locked it, an Operator policy's cap, and a key only the other Product honours; the Operator policy's version and the configuration files read. A filter narrows the keys | `config.explain`, each time the view shows and at "Refresh" |
  | Resource governor | its level and what holds it there, memory used against the Memory allowance and what set it, the browsers' memory, the host's memory pressure, CPU and Tokio busy time, and each kind's concurrency | `engine.system_info`, only while the view shows: the window's subscription is taken again with the readings, after the last event it read, and again without them once the view goes, so no event is lost and no other subscription is opened |
- **What it reads.** On the Runs view, it follows `run.progress` only of the running Runs on screen and of the chosen Run while it runs. It reads the chosen Run's report with `run.report`, and its Usage with `run.usage`, again at each new progress while it runs. On another view, it reads only what that view shows, and the Runs' listing. Nothing is read once the window is closed.
- **Its settings.** The window reads `kennel.toml` itself, at its start and on each change, as the tray does.
- **Repaints.** The window asks for a repaint only when what it shows changed, at most once a second. The person's own clicks and typing are drawn as they come.
- **Fonts.** The window draws with the system's own fonts, read as it starts (SF and SF Mono on macOS, with Helvetica and Menlo after them), so no font ships in the executable.

## Desktop notices

The Kennel shows a desktop notice when a Run or the Daemon changes to a state that needs a person. Each type is a setting (see Settings):

| Notice | When | On by default |
|---|---|---|
| Needs you | a new Needs you item, with what waits | yes |
| Run failed | a Run failed, named by its Recipe or its Run ID | yes |
| Run stopped by a Limit | a Run was stopped by a Limit | yes |
| Daemon lost | the Daemon's Connection ended with no Daemon in its place while a Run was live, with how many | yes |
| Run finished | a Run ended well | no |

- **Not for what the Kennel finds.** Nothing fires for the state the Kennel finds when it starts, or for what a resync lists, at a Connection's start or after a full queue. A Run the Kennel saw running that a resync leaves out did end while it watched: its end fires once its progress says how it ended.
- **One notice for a Limit.** A Run stopped by a Limit also makes a Needs you item: whichever of the two notices comes first is sent, not both.
- **Bursts coalesce.** The first notice goes out at once. Those that come within the period after it are held, and when the period ends one summary goes out, titled such as "3 more changes" and saying "1 new Needs you item; 2 Runs failed", which starts a new period. A summary of one notice is that notice. A period with nothing held ends the burst.
- **Text.** A notice's title and body go through the Text function, like the menu's lines: at most 60 characters of title and 200 of body.
- **Off means no notice code.** With `notifications.enabled` off, the Kennel builds no notice adapter, and on macOS it never asks for the permission. It builds the adapter once the settings are read and turn notices on, and drops it when they turn them off.
- **macOS.** The notices go through `UNUserNotificationCenter`, from `Kennel.app` started through LaunchServices; a binary run directly gets none, and the log says so. The Kennel asks for the permission as the adapter is built, so the prompt shows at the first launch, and the grant survives `just kennel`'s rebuilds under the same bundle identifier. Banners show even while the Kennel counts as the frontmost app, after a menu click.
- **Linux and Windows.** The notices go through `notify-rust`: the desktop's notification service over D-Bus (the system's libdbus) on Linux, a toast on Windows. A body that looks like markup shows as written.

## Release

The Kennel is first published with the Products' first publish, never before, under its own version: `[workspace.package]`'s `version` in `Cargo.toml`, which `Kennel.app`'s `Info.plist` carries. It is built from the same commit as the Products, through its path dependency on `goodfetch-protocol`, so a Kennel released with a preview minor of the Products speaks that minor's protocol.

`scripts/release.sh` holds the Kennel's part of a release, in ADR 0039's order. It publishes nothing: putting its folder's files beside the Products' is the release pipeline's step.

| Part | What it does | Where it runs |
|---|---|---|
| `build <folder> [<target>]` | the release executable for a macOS target, this Mac's by default, in `<folder>/<target>/Kennel.app`, signed ad hoc | a build job, which sees no signing credential |
| `sign <folder>` | each app signed with Developer ID and the hardened runtime, notarized, stapled and verified | the Products' signing job, with the `release` environment's credentials |
| `package <folder> <base URL>` | each app in `goodfetch-kennel-<version>-<target>.zip`, and `install-kennel.sh`, the install script stamped with each archive's name and SHA-256 and the URL they are served from | after signing |
| `verify <folder>` | the Platform signature gate: Developer ID, the hardened runtime, Gatekeeper's assessment and the stapled ticket; an app signed ad hoc fails it | before any publish |

- **Signing stays inert until the accounts exist.** `sign` reads `GOODFETCH_DEVELOPER_ID_P12` (the Developer ID Application certificate's `.p12`, in base64), `GOODFETCH_DEVELOPER_ID_P12_PASSWORD`, `GOODFETCH_NOTARY_KEY` (the App Store Connect API key's `.p8` text), `GOODFETCH_NOTARY_KEY_ID` and `GOODFETCH_NOTARY_ISSUER_ID`. With none set, it says it is inert and leaves the ad hoc signature, which no release publishes. With some set but not all, it fails and names the missing ones. The Goodfetch entity's Apple Developer account, which ADR 0039 requires before the first public release, gives these values.
- **The install script** downloads this Mac's archive, checks its SHA-256 and the app's signature, quits a running Kennel, installs `Kennel.app` into `~/Applications` by rename and opens it with `open`. It installs no login agent: start at login is a setting of the Kennel's own, off by default. `just kennel` installs its own build through the same script, with `--app`.
- **`just kennel-release [folder]`** runs `build`, `sign` and `package` on this Mac, in `target/release-files` by default, serving the archives from that folder: `bash <folder>/install-kennel.sh` installs that build.

## Which Daemon it reaches

- **Where it looks.** The Kennel reads the Goodfetch home: `GOODFETCH_HOME` when set, otherwise the `goodfetch` folder of the OS's data folder. On macOS that is `~/Library/Application Support/goodfetch`.
- **Which folder.** It looks in every per-major folder (`0.4`, `1`, …). A Daemon counts as running only while its runtime file, `daemon.json`, exists and its serving lock, `daemon.lock`, is held. When several Daemons run, the newest major's wins.
- **`hello`.** It connects to the Daemon's socket and says `hello` as the `kennel` surface, needing Collie, which both Products contain. It reads the Daemon's Product from the answer.
- **Protocol version.** A Daemon of another protocol version is named in the menu, with the version the Kennel needs.
- **The token.** The Kennel reads the Daemon's token from the runtime file at each connection and keeps it in memory only. It sends the token only in `hello` and never logs it.
- **It observes.** The Kennel holds the Daemon neither awake nor back from a handover, unless `keep_daemon_alive` is on.
- **Reconnecting.** The Kennel watches the home and each per-major folder through the platform's file notifications, never by polling. When a runtime file appears, changes or goes, it looks again. So it reaches a Daemon started from the CLI, an SDK or its own "Start Daemon". After a handover refusal, it reaches the major's pick once the pick runs. While connected, it moves to a newer major's Daemon that starts. A lost Connection makes it look again at once. A home that does not exist yet is watched from the next look, such as after "Start Daemon".
- **Health.** Its one poll is `daemon.health`, every 30 seconds while connected. The menu shows the answer under the Daemon: "Serving", "Draining", or why it takes no new Run.
- **What it reads.** The Kennel reads what changes as it is pushed, not by polling:
  - **The subscription.** It subscribes with `events.subscribe` and grants 64 credits. It renews them, at least 32 at a time, once it has applied the events they took. Each subscription starts with `events.resync`, which gives the live Runs and the Needs you items. A queue that filled while the Kennel was slow also comes back as `events.resync`.
  - **The live Runs' rows.** It reads `run.list` of the running Runs after each resync, and when a Run it has no row for starts. A row gives the Run's Recipe and progress message.
  - **Stop reasons.** It reads one `run.progress` of each Run that stopped, to learn whether a Limit stopped it.
  - **While the menu is open.** It follows `run.progress` of each Run the menu lists: one read at once, then reads that wait for the next change. Closing the menu cancels them. This is how a crawl's counts and a Run's progress message stay current. On Linux and Windows, `tray-icon` does not report when the menu opens or closes, so the menu counts as open for as long as the Kennel runs.
- **Start Daemon.**
  - The action runs `daemon start` of an installed Product, with no shell.
  - It uses the absolute path that the Product's executable recorded in `<home>/installed-executables.json`. Foxhound is preferred, since it contains Collie.
  - The Kennel does not look a Product up in `PATH`: an app started by LaunchServices has a minimal `PATH`.
  - The Kennel never starts a Daemon by itself.

## Settings

The Kennel's own settings live in `<home>/kennel.toml`, never in the engine's configuration. It reads the file at launch and again whenever it changes. A missing file gives the defaults. A file that cannot be read also gives the defaults, and the menu says why. An unknown key counts as unreadable, so a typo shows.

| Key | Default | Off |
|---|---|---|
| `controls` | `true` | a read-only Kennel: the menu offers no action, "Start Daemon", the Run controls and the Needs you items' actions included, and the details window offers no Add seeds |
| `confirm_stop` | `true` | Stop acts at once, without asking. Abort and Retry failed always ask, and Pause never does |
| `window` | `true` | a menu-only Kennel: the menu offers no "Open details" |
| `views.records` | `true` | the details window offers no Records view, and reads no Records |
| `views.daemon_settings` | `true` | the details window offers no Daemon settings view, and never reads `config.explain` |
| `views.governor` | `true` | the details window offers no Resource governor view, and never takes `engine.system_info` |
| `keep_daemon_alive` | `false` | the Kennel only observes: the Daemon stops at its idle timeout, and hands over to its major's pick, as if no Kennel were connected. On, a connected Kennel keeps the Daemon alive. A change reconnects, since `hello` carries it |
| `notifications.enabled` | `true` | no desktop notice, and no notice code runs (see Desktop notices) |
| `notifications.needs_you` | `true` | no notice of a new Needs you item |
| `notifications.run_failed` | `true` | no notice of a failed Run |
| `notifications.run_stopped_by_limit` | `true` | no notice of a Run stopped by a Limit |
| `notifications.daemon_lost` | `true` | no notice of a Daemon lost while a Run was live |
| `notifications.run_finished` | `false` | no notice of a Run that ended well |
| `notifications.period_minutes` | `20` | `0`: each notice goes out at once, with no summary |

The `notifications.` keys go in a `[notifications]` table:

```toml
[notifications]
run_finished = true
period_minutes = 5
```

Later features each add a switch here.

## Layout and checks

| Path | What |
|---|---|
| `crates/goodfetch-kennel` | The modules that need no graphical toolkit: the Client, the State (the pure model, the details window's and the desktop notices' included), the settings, the Daemon start, the window's process, the menu's edits |
| `crates/goodfetch-kennel-app` | The executable, `goodfetch-kennel`: the Tray, with `tray-icon` and `muda` on `winit` (on Linux, on a GTK thread of its own, `src/tray/linux.rs`), the details window, with `eframe` on glow, and the desktop notices' adapters, `UNUserNotificationCenter` on macOS and `notify-rust` elsewhere |
| `scripts/` | `install.sh` (`just kennel`), `check.sh` (`just kennel-check`), `release.sh` (`just kennel-release`), `bundle.sh` (assembles `Kennel.app`), `install-kennel.sh` (the install script a release publishes), `measure.sh` |
| `Info.plist` | The `.app`'s property list, which `bundle.sh` stamps with the Kennel's version |

- **A workspace of its own.** It has its own `Cargo.lock` and toolchain pin. Its only path dependency is `goodfetch-protocol`.
- **Clippy.** Clippy reads the root `clippy.toml`, the nearest one in a parent folder, and the root's `[workspace.lints]` are copied into `Cargo.toml`.
- **`just kennel-check`.** It runs Clippy and the tests. The Client's tests run `goodfetch-scripted-engine`, which the root workspace builds, as a process. One test runs a real Daemon of `goodfetch-collie`, also built by the root workspace with its Console build: a test program starts three Runs on its own Connection, and the Kennel's Client pauses one, stops one and aborts one from another Connection. Run straight from this folder without the test tool manifest that `just kennel-check` writes, those tests say they are skipped and pass.
- **The release scripts' tests.** On macOS, `goodfetch-kennel-app`'s `tests/release.rs` runs `bundle.sh`, `release.sh` and the install script it writes on a stand-in executable, in a home of their own, with `open`, `pgrep` and `pkill` stood in.
- **Off macOS.** The display-less Linux gate builds and tests `goodfetch-kennel` and type-checks `goodfetch-kennel-app` with Clippy, for macOS (`aarch64-apple-darwin`) and for the Linux host. The Linux type-check needs the development files of GTK 3 and libdbus (`libgtk-3-dev` and `libdbus-1-dev` on Debian and Ubuntu) and no display. Without the Rust target or those files, the check is listed in `target/skipped.txt` with the reason, and fails under `CI`.
- **Which changes run it.** The merge gate runs `just kennel-check` for a change to `kennel/` or `goodfetch-protocol`.
- **On every change.** `just lint` checks the formatting, the lockfile and the `cargo-deny` rules. `just audit` checks the advisories.
- **Advisory exceptions.** `deny.toml` and `.cargo/audit.toml` ignore two advisories of the GTK 3 bindings (gtk-rs 0.18), which only Linux builds link: RUSTSEC-2024-0370 (`proc-macro-error`, build time only) and RUSTSEC-2024-0429 (`glib`'s `VariantStrIter`, which nothing here calls). No maintained GTK 3 binding exists; `tray-icon`'s `ksni` backend, with no GTK, would remove both. `winit` builds without its own Wayland window decorations, so their unmaintained font parser (`ttf-parser`, RUSTSEC-2026-0192) is gone: the Tray opens no `winit` window, and on Wayland the details window takes the compositor's decorations where it offers them (KDE Plasma does; GNOME does not, so there the window has no title bar).

## Performance targets

These targets come from ADR 0141. They are measured on the owner's Mac, and no gate runs them. `scripts/measure.sh [seconds]` measures a running Kennel's memory (`phys_footprint`) and CPU, as #1915's throwaway build did.

The first figures are [#1915](https://github.com/goodfetch/goodfetch/issues/1915#issuecomment-6580328695)'s. They were measured on 2026-11-02, on the owner's Mac15,11 under macOS 27.2 (build 26B123), with the throwaway build.

| Target | Figure | Measured |
|---|---|---|
| Tray memory at most 30 MB `phys_footprint` | 13 MB idle (14 MB peak), 20 MB with the window open | 2026-11-02, Mac15,11, macOS 27.2 |
| Idle CPU below 0.1% over 60 s | 0.00% | 2026-11-02, Mac15,11, macOS 27.2 |
| CPU below 0.5% with a live Run | not measured yet: measured on the first build that reaches a Daemon with a live Run | |
| Window at most 100 MB once settled (no frame for 5 s), none when closed | 52 MB settled; 258 MB start-up peak: #1915's throwaway window, with egui's own fonts | 2026-11-02, Mac15,11, macOS 27.2 |
| Window, as above | not measured yet: the first build with the details window, measured on the owner's Mac with `scripts/measure.sh` while the window shows a Run, then once nothing changed for 5 s | |
| One executable at most 6 MB, stripped, with LTO | 4.77 MB (4,773,024 bytes), glow only | 2026-11-02, Mac15,11, macOS 27.2 |
| One executable, as above | 1.79 MB (1,789,744 bytes): this workspace's first build, the Tray alone, before the window | 2026-11-03, Mac15,11, macOS 27.2 |
| At most one redraw a second, none while hidden | about one a second | 2026-11-02, Mac15,11, macOS 27.2 |
