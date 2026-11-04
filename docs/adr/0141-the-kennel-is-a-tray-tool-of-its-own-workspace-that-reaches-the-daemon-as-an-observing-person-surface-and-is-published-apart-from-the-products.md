# The Kennel is a tray tool in a workspace of its own that reaches the Daemon as an observing person surface and is published apart from the Products

The Kennel is the tool that ADR 0137 left to a decision of its own. It is a menu-bar tool, written in Rust, for watching and controlling the Runs of a machine's Daemon. Its code lives in a workspace of its own, built on `goodfetch-protocol` and no engine crate. It reaches the Daemon over the socket as a new `kennel` surface: a person surface whose Runs outlive it and which holds the Daemon neither awake nor back from a handover. It is published with the Products' first publish under its own version, and is part of no Product. The owner decided this in the `/wayfinder` grilling of 2026-10-29 to 2026-11-02 ([map](https://github.com/goodfetch/goodfetch/issues/1823)). A throwaway build checked it on the owner's Mac ([#1915](https://github.com/goodfetch/goodfetch/issues/1915#issuecomment-6580328695)).

## Context

On 2026-10-29 the owner asked for « a lightweight, best practice, performant tool to monitor all what's going on … when requested, it opens a tray icon that when clicking allows to control runs, monitor them with relevant data … it shall be in Rust obviously ». On 2026-11-02 they added that it lives in neither the engine nor Collie nor Foxhound and is downloaded apart (« ni foxhound, ni collie. mais telechargeable a part »), and that others download it as well (« D'autres aussi »).

The research behind the choices is kept in `rnd/research/2026-11-01-kennel-toolkits.md`, `rnd/research/2026-11-01-kennel-competitors.md` and `rnd/research/2026-11-01-kennel-macos.md`.

## Decision

### Where it lives and how it ships

- **A workspace of its own.** `kennel/` is a Cargo workspace with its own `Cargo.lock`, excluded from the root workspace. Its only path dependency is `goodfetch-protocol`. The Products' dependency graph, lockfile and gates do not see it. `xtask affected` runs the Kennel's checks when `kennel/` or `goodfetch-protocol` changes. It keeps `clippy.toml`'s rules, and its `deny.toml` has the same bans with exceptions of its own.
- **Its own version, published with the Products.** It is first published with the Products' first publish, never before, under a version of its own. It carries their compatibility promise (ADRs 0030, 0041 and 0047): at `hello` it checks the protocol version, and when it cannot talk to a Daemon it says so, naming the version it needs.
- **Install.** From the repository, `just kennel` builds the `.app`, installs it by rename (never by overwriting a signed binary in place) and opens it. A published release adds files of its own beside the Products': a `.app` signed with Developer ID and the hardened runtime and notarized under ADR 0039, and an install script. Builds from the repository are signed ad hoc. There is no App Sandbox, which would deny the Goodfetch home and the socket, and no auto-update.
- **Always started through LaunchServices.** On macOS the Kennel is started with `open`, by the Finder or by a login item, never by running the binary inside the `.app` directly. Run directly, the same `.app` is refused desktop notices with no prompt. Started through LaunchServices, an ad hoc build gets the prompt, and its grant survives rebuilds under the same bundle identifier ([#1915](https://github.com/goodfetch/goodfetch/issues/1915#issuecomment-6580328695)).
- **Start at login** is a setting, off by default. A launchd user agent starts the Kennel by running `/usr/bin/open` on the installed bundle.
- **No CLI subcommand.** The Products' CLI does not know the Kennel.
- **Platforms.** macOS is polished. Linux and Windows build and run with the same crates, with no polish of their own until users ask.

### Form

- **Two levels.** A click on the icon, left or right, opens a native menu. The menu holds the verdict, the live Runs and their controls, Needs you, and "Open details". The details window opens on demand, as a second process of the same executable, and exits when the window closes. A closed window costs nothing. With the setting `window` off, the Kennel is a menu only.
- **The menu is fresh when it opens.** It is rebuilt in the click's mouse-down handler, which runs before the menu opens, and it keeps updating while it stays open.
- **A hidden icon is caught.** Once at launch, the Kennel checks whether its icon is hidden: behind the notch, or in macOS 27's overflow. If it is, the Kennel opens the details window with a short note asking the person to free space in the menu bar. The check reads the status item window's occlusion state, not its geometry, which macOS 27 reports as on screen for hidden items. A setting, on by default.

### Toolkit and threads

- **The tray process** uses `tray-icon` and `muda` on `winit`. **The window process** uses `eframe` with the glow renderer and default features off.
  > Note (agent decision, pending owner review), 2026-11-03, [#2060](https://github.com/goodfetch/goodfetch/issues/2060): the window draws with the system's own fonts, read as it starts (SF and SF Mono on macOS), not with `eframe`'s `default_fonts`. egui's bundled fonts are under OFL-1.1 and the Ubuntu Font Licence, which ADR 0018's allowlist does not hold for a crate; with the system's fonts, no font ships in the executable. Allowing those licenses for `epaint_default_fonts` alone would bring egui's fonts back. The window also leaves out `accesskit`, whose Linux side adds unmaintained crates to the audit.
- **Threads.** A plain `fn main` builds a one-worker multi-thread Tokio runtime and holds it, then runs the UI loop on the main thread. ADR 0060 allows a runtime in `main`. Tasks go through a `TaskTracker`. Tokio reaches the UI through the event loop's proxy, and the UI thread makes no blocking call. Opening the socket carries an `#[expect]` naming this ADR, as the conformance harness's does.
- **Exceptions this ADR grants.** `STYLE.md` requires every dependency to build on static musl, and no graphical toolkit does, so the rule does not apply to the Kennel's graphical crates. The workspace pins its own toolchain, since `eframe` 0.37 needs rustc 1.95.

### Reaching the Daemon

- **A client module of its own** over `goodfetch-protocol`'s types. It reads `daemon.json`, opens the socket, sends `hello`, frames JSON-RPC by line and keeps credits. It is one deep module behind a narrow interface, so that moving it into a shared client crate later stays a small move. The Kennel links no engine crate.
- **The `kennel` surface.** `goodfetch-protocol`'s `Surface` gains `kennel`.
  - **A person surface.** The engine admits `Reach::PersonOnly` operations from it, as from `cli`, so the Kennel resolves Needs you items to the end.
  - **Runs outlive it.** A Run that a `kennel` Connection starts or resumes outlives that Connection, as a CLI command's does.
  - **It observes.** A `kennel` Connection holds off neither the Daemon's idle stop nor a handover to the major's pick. A setting, off by default, makes a connected Kennel keep the Daemon alive instead.
  - **Not a security boundary.** A surface is declared by the Client itself; the barrier stays the token.
- **Reconnecting.** When the Daemon stops, the icon shows it stopped and offers Start. When `daemon.json` changes, the Kennel learns it from the platform's file notifications, not by polling, and reconnects. On a handover refusal it reconnects to the pick.
- **Push, not poll.** The Kennel subscribes with `events.subscribe` (`run.axes_changed`, `needs_you.changed`) and grants about 64 credits, renewed as events are applied. A full queue falls back to `events.resync`, so no producer waits for the Kennel. It polls only `daemon.health`, on a slow timer. `engine.system_info` and `run.progress` are subscribed to only while the window shows them, `run.progress` for the Runs on screen.
  > Note (agent decision, pending owner review), 2026-11-03, [#2055](https://github.com/goodfetch/goodfetch/issues/2055): the open menu counts as a view that shows the Runs, and the reads the menu needs stay within this budget.
  > - **Counts and messages while the menu is open.** A crawl's succeeded, failed and pending counts exist only in `run.progress`, and `run.axes_changed` carries no progress message. While the menu is open, the Kennel follows `run.progress` for each Run the menu lists, at most 10. It reads each at once, then reads with `after` until the menu closes, which cancels the reads. While the menu is closed, it follows none. The menu is rebuilt from the latest data as the click starts and applies the fresh values within one round trip.
  > - **Initial state.** Each subscription names a position ahead of any the engine gave, which starts it with `events.resync` (ADR 0031). The Kennel then starts from the Daemon's live Runs and Needs you items, with no gap before the stream.
  > - **Rows and stop reasons.** The Kennel reads `run.list` of the running Runs after each resync, and when a Run it has no row for starts. That gives each Run's Recipe and progress message. It reads one `run.progress` of a Run that stopped, since `run.axes_changed` carries no stop reason and "stopped by a Limit" needs it.
  > - **Other platforms.** On Linux and Windows, `tray-icon` reports neither the menu's opening nor its closing. There, the menu counts as open for as long as the Kennel runs. A Run that failed or was stopped by a Limit, and a failed action's line, leave 30 minutes after the menu first drew them, since no closing ever comes (agent decision of spec #2052's review, pending owner review). On Linux, AppIndicator exports the menu over dbusmenu and the panel sends its `opened` and `closed` events, but `libdbusmenu-gtk` turns them into GTK's `show` and `hide` only on a submenu's widget, never on the root menu, so GTK gives no signal of the menu's opening to follow instead.
  >
  > Rejected: following every live Run's progress at all times, which keeps a long poll per Run open with no one looking; and a read on each menu opening without `after`, whose open menu then does not stay live.

### Data, controls and features

- **The icon** shows one verdict, worst first: Daemon unreachable, a Needs you item, a Run failed or stopped by a Limit, running, idle. It shows no numbers.
- **The menu** shows the Daemon's state, then the live Runs: Recipe or Run ID, Lifecycle, the `ProgressMessage`, and for a crawl its succeeded, failed and pending counts. Each Run has a submenu of controls. Then come Needs you and "Open details". Foxhound's views are hidden on a Collie Daemon.
- **Controls, each labelled with what it does:**
  - Stop: a Checkpoint, and the Run can resume.
  - Abort: no attempt in flight commits. It is not immediate.
  - Pause: a Checkpoint and a stop, not a pause in memory.
  - Resume, where the engine can run the Run; otherwise "Copy resume command" (ADR 0031).
  - Retry failed.
  - Add Seeds, in the window.
- **Needs you items** are resolved through their person-only operations. Answering a Human pause and reconciling wait for `human_pause.answer` and `run.reconcile`.
- **The window** has:
  - a Runs table with filters;
  - a Run's progress;
  - Usage;
  - failures by Error code;
  - the Run report;
  - Records, through `dataset.query`;
  - settings, through `config.explain`;
  - governor readings, through `engine.system_info`.
- **Never in the Kennel:** starting a Run, since `run.start` and `crawl.start` open the Run of the calling program; and accounts and sharing, which belong to Goodfetch Cloud.
- **Every feature is a setting** with a free off state: `window`, `notifications` and each desktop notice type, `controls` (off gives a read-only Kennel), the Stop confirmation, the hidden-icon check, keeping the Daemon alive, start at login, and a switch per view.

### Security

- **The token.** The Kennel reads `daemon.json` (`0600`) at each connection and holds the token in memory only. It sends the token only in `hello` and never logs it. It never passes the token to the window process, which reads the file itself. There is no Keychain copy. A rotation is picked up on reconnect.
- **No network surface.** No listening port or socket, no HTTP, no ops listener. Only the local Daemon.
- **No implicit Daemon start.** With no Daemon, the Kennel shows "No Daemon" and a "Start Daemon" action. That action runs the installed Product's `daemon start` by absolute path, with no shell.
- **Confirmations** name the Run and the consequence for Abort, Retry failed and Add Seeds. Stop's confirmation is a setting, on by default. Pause has none.
- **Hostile text.** URLs, messages and Error text come from crawled sites. The Kennel shows them as plain text, strips control and bidirectional characters, truncates them, renders no HTML, opens no URL unasked, and masks Secret values.

### Desktop notices

- **When.** A desktop notice fires on a transition to a state that needs a person. On by default: a new Needs you item, a Run failed or stopped by a Limit, and the Daemon lost while a Run was live. Off by default: a Run that ended well. A stuck Run comes later.
- **Each type is a setting.** With `notifications` off, no desktop-notice code runs.
- **Bursts coalesce.** The first notice goes out at once, then one summary per 20 minutes; the period is a setting. Nothing fires for a resync's replayed state or on the Kennel's own start. The text is plain and truncated.
- **APIs.** macOS uses `UNUserNotificationCenter` from the `.app`; Linux and Windows use `notify-rust`.
- **Not the engine's Notification.** The Kennel's notices exist only while it runs and need no engine change, so a Notification still reaches a person when the Kennel is closed.

### Performance targets

The targets are measured on the owner's Mac, not used as a gate, and kept in the Kennel's README with their measurement date. The Kennel is in no Product, so ADR 0071's Performance budgets and ADR 0057's Size budget do not cover it.

| Target | Measured on 2026-11-02, Mac15,11, macOS 27.2 |
|---|---|
| Tray memory at most 30 MB `phys_footprint` | 13 MB idle, 20 MB with the window open |
| Idle CPU below 0.1% over 60 s | 0.00% |
| CPU below 0.5% with a live Run | measured on the first real build |
| Window at most 100 MB once it has settled (no frame for 5 s), none when closed | 52 MB settled; the 258 MB start-up peak is reported beside it |
| One executable at most 6 MB, stripped, with LTO | 4.77 MB |
| At most one redraw a second, none while hidden | about one a second; the redraw count is checked in tests |

The 3 MB "tray binary" target of the grilling assumed a binary holding only the tray. The window is a second process of the same executable, so the target counts the whole executable.

### Tests

- **The state module is pure.** It takes events in and gives a view model out, so most tests live there, with no toolkit and no socket.
- **The client module** is tested against `goodfetch-scripted-engine` over the socket (ADR 0069).
- **One test runs against a real Daemon** with a test Run, which it pauses and stops. This test also proves that a Daemon accepts `run.stop` and `run.abort` from a Connection that did not start the Run. That path has only been read in the code, not run.
- **No Product UI tooling.** No UI automation, Reference screenshots, Journeys or accessibility matrix. Each change to the UI gets a manual check on a Mac.

### Open questions for the spec

- How Clippy finds the root configuration from the nested workspace.
- How `just merge`'s display-less Linux gate builds the Kennel, or skips its graphical part.
- The format and place of the Kennel's own settings.
- CPU with a live Run, measured on the first build that reaches a Daemon.

## Considered Options

- **A crate in the root workspace:** rejected. It shares the lints and the lockfile, but pulls GTK3 and the graphical toolkits into every gate of the Products, and the owner wants the Kennel out of them.
- **Another repository:** rejected. The protocol types would drift, for the reasons that keep both Products in one repository (ADR 0084).
- **Linking `goodfetch-engine` for its `connect_daemon`, extracting a shared client crate now, or building on the Rust SDK:** rejected. The first goes against the lightweight goal. The second changes the Products' shared code for the Kennel. The third links the engine, and does not exist yet.
- **Claiming the `cli` surface, or adding a general `hello` option "this Connection's Runs outlive it":** rejected. The first is false in the Audit log. The second no Client asks for once `kennel` exists.
- **A connected Kennel holding the Daemon awake by default:** rejected. The Kennel watches; it does not create work. The opposite is a setting.
- **A native menu only, or a menu and a popover:** rejected. A menu only has no charts or tables, against the owner's ask for « precise data and analytics and metrics ». Neither `tao` nor `winit` offers a panel window, and Linux has no reliable equivalent.
- **Tauri, Slint, iced or gpui:** rejected. Tauri is a web view, measured at about 109 MB idle on macOS. Slint's licence is off the allowlist. iced has no upstream accessibility. gpui needs a git dependency.
- **wgpu for the window:** rejected after measurement. It settles 5 MB lower, but its start-up peak is 136 MB higher and it adds 3.2 MB of binary.
- **Polling the Daemon, as most tools studied do:** rejected. The protocol pushes the events the Kennel needs.
- **A `goodfetch kennel` subcommand:** rejected. The CLI ships in both Products and would carry knowledge of the Kennel.

## Consequences

- **Amends ADR 0137.** The tool it announced is the Kennel, decided here. It stays part of no Product. It is no longer the owner's alone: it is published for others, apart from the Products, under its own version. ADR 0137's "The owner's monitoring tool is designed apart" reads with that change.
- **Amends ADR 0039.** The published Kennel's `.app` carries a Platform signature under the Goodfetch entity: Developer ID, the hardened runtime and notarization, in the Products' release signing. Builds from the repository stay ad hoc.
- **Amends ADR 0094.** A release publishes the Kennel's files beside the Products' own. The Kennel's version is its own, not the Products'.
- **Engine changes, for every Client.** These come with the Kennel's spec:
  - `Surface::Kennel` in `goodfetch-protocol`, with `xtask codegen` and its committed output;
  - `Reach::PersonOnly` admitting `kennel`;
  - the rule that Runs outlive their Connection, extended to `kennel`;
  - the Daemon's idle-stop and handover counts leaving `kennel` Connections out unless the Connection asks to keep the Daemon alive.

  No Kennel code enters the engine.
- **Shared paths (ADR 0140).** While a Kennel is open, its Subscription adds one encode per event on the JSON-RPC message path. That is within that path's Cost contract of one encode per event. No new path and no per-unit work. The `hello` check runs once per Connection.
- **ADR 0060.** The Kennel's socket opening carries an `#[expect]` naming this ADR. A toolkit's internal threads do not trip the lints.
- **Glossary.** **Kennel** and **Desktop notice** are defined.
- **Throwaway build.** The build that checked this design is kept on a throwaway branch, out of `main`, linked from [#1915](https://github.com/goodfetch/goodfetch/issues/1915).
