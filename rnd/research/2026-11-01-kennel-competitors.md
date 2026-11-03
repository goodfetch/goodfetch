# How leading tools let a person watch and control running work

**Date:** 2026-11-01. **Ticket:** [#1825](https://github.com/goodfetch/goodfetch/issues/1825), on the map [#1823](https://github.com/goodfetch/goodfetch/issues/1823).

**Purpose:** evidence for the design of the Kennel, the tray-icon tool one person runs to watch and control the Runs of a local Daemon. This file reports patterns. The decisions are the owner's.

**Method:** official documentation, source code on default branches (read 2026-11-01 through the GitHub API) and issue trackers. No tool was installed or run, so everything about look and feel comes from what vendors and users wrote.

**Evidence labels:** `doc` official page, `src` source code, `issue` a tracker report (usually one user's; comment counts are given where they show demand), `snippet` seen only in a search result. Competitors' own words (job, alert, terminate) are kept when describing them; Goodfetch terms are used everywhere else.

## Comparison

The sources for every cell are in that tool's section below. "Not found" means I looked and found no statement; see [What I could not verify](#what-i-could-not-verify).

### Menu-bar tools

| Tool | Form; what a click does | On the icon | At a glance when open | Controls and guards | Refresh | Notifies about; tuning | Users ask or complain |
|---|---|---|---|---|---|---|---|
| **OrbStack** | Native-style nested menu; opens it. Can be turned off | Not documented; a greyed-out state is mentioned by a user | Containers, Compose projects, machines, ports, mounts | Start, stop, restart, delete, logs, terminal, open in browser. Guards not found | Not found | Not found; a user says a VM death went unannounced | Silent VM death, health colours for colour blindness, start in stopped state |
| **Docker Desktop** | Menu, with actions that jump to the Dashboard window | Moon during Resource Saver; update notice | Menu contents not documented now; Dashboard has the list with optional live stats | Dashboard: start, stop, pause, restart, delete, bulk. Guards not found | Not found | Per-type toggles; errors and new releases always on; shown inside the Dashboard | Paused state not on icon, remove the icon, OS notification on health change, wrong status in the menu |
| **Tailscale** | Menu or popover, plus a separate window and a "mini player" window | Exclamation mark on connectivity problems | Status, devices, exit nodes, health items; search on macOS 26 and later | Connect, exit node, account. Guards not found | Not found | Connectivity trouble, key expiry; OS permission and managed policies | Icon hidden by the notch reads as "it's gone", popover ignoring clicks |
| **Ollama** | Native menu; opens it | Update icon (Windows source); "Not responding" reported | Nothing about models or server state | Open Ollama, Settings, Restart to update, Quit | No live data in the menu | Update available (Windows source) | Quit leaves processes running; "Not responding" with no interface |
| **Stats** | Floating panel per module; click toggles it | Live numbers or graphs, one item per module | Per-module detail | Fan control only (legacy); turn modules off | Reader interval per module, default 1 s for CPU | Threshold per metric, disabled by default | Notification floods, spike alerts with no duration, idle overhead |
| **iStat Menus** | Dropdown per item, or one combined item | Live values, customizable | Per-core load, top apps, history graphs | Monitoring only | Global update frequency setting; values not found | Rules with duration, e.g. above 60% for over 10 s | No public tracker found |
| **Sprocket** (CI) | Popover | One glyph: worst state over all repositories | Account, repositories, runs, rate-limit usage | Not found beyond sign-in | Poll 30 s, 15 s while running; ETag; battery-aware backoff | Transition to failure, timed_out, action_required; bursts coalesced | Small project; no real demand signal |
| **ActionBar** (CI) | Not documented beyond the screenshot | Not described | Run status per repository | Choose repositories, poll interval, manual refresh | Poll 30 s, configurable | Run completed; OS permission | Rate limits with many repositories (vendor FAQ) |
| **CCMenu** (CI) | Menu, plus a pipeline window; an item opens the build page | Summary icon: running first, then broken | Each pipeline, optional details, estimated time left | None on builds (read-only) | Classic version default poll 5 s | Build start and finish | Not researched further |

### Run monitors

| Tool | Form | At a glance | Controls and guards | Refresh | Notifies about; tuning | Users ask or complain |
|---|---|---|---|---|---|---|
| **Temporal UI** | Web pages | List filterable by status, saved views, flagged Task Failures | Cancel, Terminate, Reset, Signal, Update, batch. Modal, "cannot be undone", destructive button, optional reason; per-action off switches | Polls: 10 s run page timer, 15 s toggle, hard-coded | None in the UI docs; flags a workflow after 5 task failures | Configurable intervals, batch actions everywhere, disabling writes |
| **Prefect** | Web pages | Run lists, state, late-run count | Cancel, pause, resume, retry, each in a dialog; cancel can include sub-flows | Polls: 5 s on a live run, stops when terminal; 30 s lists; 60 s events | Automations: on an event or on its absence; many actions; templated messages | Bulk cancel |
| **Dagster** | Web pages | Run list, Gantt chart, event and raw logs | Terminate dialog with count; force checkbox with warning; progress bar. Re-execute | Polls 15 s, adaptive; run logs by subscription | OSS: code sensors. UI alert policies are Dagster+ only | Bulk beyond 25 runs, steps surviving termination, auto-refresh on more pages |
| **Crawlee** | Library, no UI | Log lines: statistics every 60 s, status every 10 s | `stop()`, and in current source `pause()`/`resume()` | Push to log, timer based | None; status callback | Pause, hangs |
| **Apify** | Web pages | Status, one status sentence, log, live view, 30-day charts | Abort (immediate or graceful 30 s), Resurrect with new limits | Not found | Metric and status alerts; first after 1 min, then aggregated every 20 min; email, Slack, in-Console | No public tracker found |
| **ScrapydWeb** | Web pages | Jobs table, log statistics, progress charts | Start, Stop, force stop; no confirm string in the Jobs template | Polls: jobs page 300 s, node load 10 s, monitor round 300 s | Off by default; thresholds per log level, working days and hours, finish, repeat interval; may stop the job | Alert delivery, timer pages with long history |
| **k9s** | Terminal UI | Live resource tables, pulses view | Delete: Tab and Enter. Kill: no confirmation. `--readonly`; plugin `dangerous` flag | Watches for changes, UI redraw every 2 s (minimum) | None found | Accidental kill and delete, confirm modal not focused |

## Menu-bar tools in detail

### OrbStack

- Menu contents: start, stop, restart and delete containers and projects; logs and terminal; open a web service; ports and bind mounts; copy ID, image and domain; Compose as a group; machines stop, restart, terminal, Finder; switch Kubernetes context. It can be turned off in Settings. `doc` <https://docs.orbstack.dev/menu-bar>
- Submenus exist, and one bug is that hovering an item with a submenu closes the menu. `issue` <https://github.com/orbstack/orbstack/issues/2494>
- Requests: add a shortcut for `compose watch` <https://github.com/orbstack/orbstack/issues/1049>; launch at login with the icon greyed out and the engine stopped <https://github.com/orbstack/orbstack/issues/2206>; more contrast between health colours for colour-blind users <https://github.com/orbstack/orbstack/issues/703>.
- A VM was killed, and a user reports no clear notice for over two hours before it restarted. `issue` <https://github.com/orbstack/orbstack/issues/2314>

### Docker Desktop

- The Docker menu opens Settings and other actions, and shows update notices. `doc` <https://docs.docker.com/desktop/settings-and-maintenance/settings/> Its redesign was for consistency across systems and a faster jump to the Dashboard. `doc` <https://www.docker.com/blog/new-docker-menu-improved-release-highlights-with-docker-desktop-4-5/>
- The Dashboard window is the real surface, with Quick Search and a bell for the Notifications center. `doc` <https://docs.docker.com/desktop/use-desktop/>
- Containers view: optional columns for live CPU, memory and network; per-item and bulk start, pause, stop and delete; Stats and Exec tabs. `doc` <https://docs.docker.com/desktop/use-desktop/container/>
- A moon shows on the tray icon in Resource Saver mode; idle time defaults to 5 minutes and is tunable; it is on by default. `doc` <https://docs.docker.com/desktop/use-desktop/resource-saver/>
- Notifications tab: status updates, recommendations, announcements and surveys can be switched off; errors and new releases cannot. They appear briefly in the Dashboard, then move to a drawer. `doc` <https://docs.docker.com/desktop/settings-and-maintenance/settings/>
- Requests, all open: show Paused on the icon <https://github.com/docker/roadmap/issues/530>, <https://github.com/docker/roadmap/issues/298>; make the icon optional (17 comments) <https://github.com/docker/roadmap/issues/281>, and (9 comments) <https://github.com/docker/roadmap/issues/486>; OS notifications on container health change <https://github.com/docker/roadmap/issues/224>. A menu showed the wrong Kubernetes status <https://github.com/docker/for-mac/issues/6665> (title only).

### Tailscale

- Tailscale keeps the menu bar app and added a window beside it, stating two limits: dropdowns "don't allow us to easily convey information through changes in shape or color" and "can't include a search bar". The window has a searchable device list, a red dot on the Dock icon for critical errors, a debug window, and a compact "mini player" for status and exit nodes. Quitting offers to stay in the menu bar and leave the Dock. `doc` <https://tailscale.com/blog/windowed-macos-ui-beta>
- The search limit has since moved: v1.102.1 (2026-08-03) added search to the main menu on macOS 26 and later, and v1.104.2 (2026-10-21) fixed menu construction timing. `doc` <https://tailscale.com/changelog>
- v1.70.0 (2024-07-17): health notifications in the client menu, and connectivity problems change the icon to show an exclamation mark. Other entries: warnings and push notifications when connectivity is blocked (v1.56.0), key expiry notice (v1.38.1). `doc` <https://tailscale.com/changelog>
- The notch hides icons and macOS gives apps no placement control. Users reported the app "gone" while it ran, so Tailscale detects the hidden icon, warns, and offers the window. `doc` <https://tailscale.com/blog/macos-notch-escape>
- Holding Option while clicking opens a debug menu. `snippet` <https://tailscale.com/docs/reference/debug-menu> Policies can silence specific notifications. `snippet` <https://tailscale.com/docs/features/tailscale-system-policies>
- Complaints: the popover ignores clicks until the window is opened <https://github.com/tailscale/tailscale/issues/21984>; an animated icon was "super distracting" (closed as no longer reproducible) <https://github.com/tailscale/tailscale/issues/1896>.

### Ollama

- A native `NSMenu` with Open Ollama, Settings, update items, Quit Ollama, and now Claude and ChatGPT entries. Nothing about loaded models or server state. `src` <https://github.com/ollama/ollama/blob/HEAD/app/cmd/app/app_darwin.m>
- On Windows, an available update swaps the icon and sends one notification. `src` <https://github.com/ollama/ollama/blob/HEAD/app/wintray/menus.go>
- Its only guard I found warns that restarting ChatGPT stops any running task. `src`, same `app_darwin.m`.
- Complaints: the server stays after closing (20 comments) <https://github.com/ollama/ollama/issues/1925>, and <https://github.com/ollama/ollama/issues/14628>; the status item says "Not responding" with a yellow exclamation mark while the interface never loads <https://github.com/ollama/ollama/issues/11746>.

### Stats

- Open source, active (v3.0.24 on 2026-11-01, about 43,000 stars). Each enabled module has its own menu bar icon. Sensors and Bluetooth are named the least efficient; disabling them can cut CPU use by up to 50%. `doc` <https://github.com/exelban/stats>
- The popup is a floating `NSPanel` that does not hide on deactivate and can be locked. `src` <https://github.com/exelban/stats/blob/HEAD/Kit/module/popup.swift>
- Each reader has a selectable interval, starting at 1 s (<https://github.com/exelban/stats/blob/HEAD/Kit/types.swift>), and CPU defaults to 1 s (<https://github.com/exelban/stats/blob/HEAD/Modules/CPU/settings.swift>). `src`
- Notification thresholds are a list from "Disabled" through 3%, 5%, 10% and upward. `src` <https://github.com/exelban/stats/blob/HEAD/Kit/types.swift>
- Complaints: alerts on a fraction-of-a-second spike, asking for "more than x seconds" <https://github.com/exelban/stats/issues/2436>; 5,776 false Wi-Fi notifications in 19 hours <https://github.com/exelban/stats/issues/3631>; IP notifications that fire with no real change <https://github.com/exelban/stats/issues/3447>; 160 MB at launch (title only) <https://github.com/exelban/stats/issues/3713>; about 49 messages a second to `bluetoothd` from a module with no widget <https://github.com/exelban/stats/issues/3690>.

### iStat Menus

- Items with customizable display, detailed dropdowns, a combined mode that merges items into one, and rules such as "if CPU usage is above 60% for more than 10 seconds". `doc` <https://bjango.com/mac/istatmenus/>
- "A quicker update frequency samples stats more regularly. A slower update frequency uses slightly less resources." `doc` <https://bjango.com/help/istatmenus7/global/>

### CI watchers

- **Sprocket:** one glyph shows "the worst current state across every watched repo", ordered failure, running, rate-limited, success. The popover lists account, repositories and runs. Refresh is 30 s, or 15 s with running jobs, ETag-aware, with battery-aware backoff. It notifies on transitions to failure, timed_out or action_required and coalesces bursts. `doc` <https://github.com/MRL-00/Sprocket> (11 stars)
- **ActionBar:** polls every 30 s, configurable, with manual refresh; notifies on completion once allowed in System Settings; warns that each repository costs one call per refresh, so many repositories hit rate limits. `doc` <https://ptrchm.com/actionbar>, MIT source <https://github.com/pch/actionbar> (5 stars). A search snippet lists an App Store version at $9.99; that page would not load, so the price is unverified.
- **CCMenu:** icons for succeeded, failed, running after success, running after failure, grey pause for "waiting for approval", grey circle for unavailable. Running has display priority over broken. It times builds itself to show an estimate. `doc` <https://ccmenu.org> The classic version's poll default is 5 (seconds, by the setting's use; unit not stated) `src` <https://github.com/ccmenu/ccmenu/blob/HEAD/CCMenu/FactoryDefaults.plist>. The rewrite is <https://github.com/ccmenu/ccmenu2>.
- **Deplog:** colour at a glance and failure notifications, on a polling schedule. A commenter's questions are about rate limits and whether polling adapts to activity. `issue` <https://github.com/orgs/community/discussions/198739>
- **Octocheck:** green, red, orange and grey icon by polling. `doc` <https://github.com/warmwind/octocheck> (0 stars)

None of these CI watchers documents a control such as re-run or cancel. They watch and link out.

## Run monitors in detail

### Temporal UI

- Pages: Workflows, saved views, a Task Failures view, Schedules. A workflow is flagged after 5 consecutive task failures, tunable with a dynamic config value. Controls: cancellation, Signal, Update, Reset, Terminate. `doc` <https://docs.temporal.io/web-ui>
- Terminate and Cancel each open a modal: "Are you sure you want to terminate this workflow? This action cannot be undone.", with a destructive button and an optional reason. `src` <https://github.com/temporalio/ui/blob/HEAD/src/lib/components/workflow/client-actions/terminate-confirmation-modal.svelte>, <https://github.com/temporalio/ui/blob/HEAD/src/lib/i18n/locales/en/workflows.ts>
- The batch modal names the count: "Are you sure you want to {{action}} {{count}} running workflows?" (same `workflows.ts`). A reason is the only record of why a workflow stopped. `doc` <https://docs.temporal.io/encyclopedia/workflow/cancellation-and-termination>
- Off switches, all default false: `disableWriteActions` overrides `workflowTerminateDisabled`, `workflowCancelDisabled`, `workflowSignalDisabled`, `workflowUpdateDisabled`, `workflowResetDisabled`, `workflowPauseDisabled`, `batchActionsDisabled`, `startWorkflowDisabled` and `activityCommandsDisabled`. Also `refreshWorkflowCountsDisabled`. `doc` <https://docs.temporal.io/references/web-ui-configuration>
- Refresh: a 10 s timer on the run page (<https://github.com/temporalio/ui/blob/HEAD/src/lib/layouts/workflow-run-layout.svelte>), and a remembered auto-refresh toggle labelled 15 seconds (<https://github.com/temporalio/ui/blob/HEAD/src/lib/components/auto-refresh-workflow.svelte>). `src`
- Requests: configurable refresh intervals <https://github.com/temporalio/ui/issues/3082>; refresh buttons <https://github.com/temporalio/ui/issues/862>, <https://github.com/temporalio/ui/issues/2246>; disable writes (10 comments, shipped) <https://github.com/temporalio/ui/issues/425>; terminate everything during development <https://github.com/temporalio/ui/issues/838>; batch signal <https://github.com/temporalio/ui/issues/2150>; bulk pause of schedules <https://github.com/temporalio/ui/issues/2918>.

### Prefect

- Dialogs exist for cancel, pause, resume and retry. <https://github.com/PrefectHQ/prefect/tree/HEAD/ui-v2/src/components/flow-runs/flow-run-actions> Cancel is an alert dialog with an optional checkbox for sub-flows, and it reports partial failures. `src` <https://github.com/PrefectHQ/prefect/blob/HEAD/ui-v2/src/components/flow-runs/flow-run-actions/cancel-flow-run-dialog.tsx>
- A live run's page refetches every 5 s and stops at a terminal state. `src` <https://github.com/PrefectHQ/prefect/blob/HEAD/ui-v2/src/components/flow-runs/flow-run-details-page/index.tsx> The repository's rule is 30 s for live operational data and 60 s for events. `src` <https://github.com/PrefectHQ/prefect/blob/HEAD/ui-v2/src/api/AGENTS.md> The late-run count is 60 s. `src` <https://github.com/PrefectHQ/prefect/blob/HEAD/ui-v2/src/api/flow-runs/index.ts>
- A flow can wait for a person: "the flow run pauses until a user clicks the Resume button in the Prefect UI, enters a name, and submits the form". Suspending tears down the infrastructure. `doc` <https://www.prefect.io/docs/v3/advanced/interactive>
- Automations trigger on an event or "in the absence of an expected event", for example a run still running after 30 minutes. Actions include cancel, suspend, resume, change state and send a notification, with Jinja templates. Email and incident actions are Cloud only. `doc` <https://www.prefect.io/docs/v3/automate/events/automations-triggers>
- Request: bulk cancel, from someone restarting flows while testing changes. `issue` <https://github.com/PrefectHQ/prefect/issues/14253>

### Dagster

- Runs list with filters; run details with a Gantt chart, structured events and raw stdout and stderr; Re-execute. `doc` <https://dagster.io/docs/guides/operate/webserver>
- Termination dialog: "N runs will be terminated. Do you wish to continue?". When a safe termination is possible, a "Force termination immediately" checkbox reveals "computational resources created by runs may not be cleaned up". When none is possible, that warning shows outright. During the work there is a progress bar and a navigation block. `src` <https://github.com/dagster-io/dagster/blob/HEAD/js_modules/ui-core/src/runs/TerminationDialog.tsx>
- The refresh hook skips hidden tabs, refreshes on return, polls slower when a query takes over a quarter of the interval (`max(interval, duration * 4)`), and can show a countdown. `src` <https://github.com/dagster-io/dagster/blob/HEAD/js_modules/ui-core/src/app/QueryRefresh.tsx> The runs feed uses 15 s. `src` <https://github.com/dagster-io/dagster/blob/HEAD/js_modules/ui-core/src/runs/RunsFeedRoot.tsx>
- Run logs come by subscription, batched every 100 ms. `src` <https://github.com/dagster-io/dagster/blob/HEAD/js_modules/ui-core/src/runs/LogsProvider.tsx>
- Alert policies are Dagster+ only <https://docs.dagster.io/guides/observe/alerts/example-config>; open source uses sensors such as `make_slack_on_run_failure_sensor` <https://docs.dagster.io/integrations/libraries/slack/dagster-slack>. `doc`
- Complaints: cancel over 25 runs at once (9 comments) <https://github.com/dagster-io/dagster/issues/3271>; steps surviving termination <https://github.com/dagster-io/dagster/issues/21476>, <https://github.com/dagster-io/dagster/issues/14248>; retries firing on manually terminated runs <https://github.com/dagster-io/dagster/issues/20217>; auto-refresh on more pages <https://github.com/dagster-io/dagster/issues/5597>, <https://github.com/dagster-io/dagster/issues/3953>.

### Crawlee

- No interface of its own. It logs statistics every 60 s by default (`logIntervalSecs`). `src` <https://github.com/apify/crawlee/blob/HEAD/packages/basic-crawler/src/internals/crawlers/statistics.ts>
- It sets a status message every 10 s by default, which a callback can override. The default turns into "Experiencing problems, N failed requests in the past M seconds." A bounded timeout stops a slow status call from hanging the crawler. `src` <https://github.com/apify/crawlee/blob/HEAD/packages/basic-crawler/src/internals/basic-crawler.ts>, `doc` <https://crawlee.dev/js/api/basic-crawler/interface/BasicCrawlerOptions>, fix <https://github.com/apify/crawlee/commit/9034f08106f53a70205695076e874f04f632c5bb>
- `stop()` lets a crawler end gracefully <https://github.com/apify/crawlee/pull/2792>. Current source has `pause(timeoutSecs)`, which stops dispatching and lets in-progress requests finish, and `resume()`. `src`, same `basic-crawler.ts`. Whether v3.18.2, the latest release, includes these is unverified.
- A heavily discussed report is a crawler that never finishes (58 comments). `issue` <https://github.com/apify/crawlee/issues/505>

### Apify

- Statuses: READY; RUNNING, TIMING-OUT, ABORTING; SUCCEEDED, FAILED, TIMED-OUT, ABORTED. Abort kills immediately by default, or gracefully with an `aborting` event and 30 s to save state. Resurrect restarts a finished run with the same storage, and its dialog can change timeout, memory, build and cost limit. `doc` <https://docs.apify.com/platform/actors/running/runs-and-builds>
- Each run has one `statusMessage`, set as often as needed, and the SDK calls the API only on change. `doc` <https://docs.apify.com/platform/actors/development/programming-interface/status-messages> The run object adds `isStatusMessageTerminal`. `snippet` <https://docs.apify.com/api/v2/actor-run-abort-post> It shows in the run detail. `snippet` <https://docs.apify.com/sdk/python/docs/2.7/concepts/actor-lifecycle>
- A web server in the run appears in the Live View tab. `snippet` <https://docs.apify.com/academy/running-a-web-server>
- Monitoring: a 30-day status chart, a chart of the last 200 runs, and alerts. "Higher than" is checked about every 5 minutes during a run; status alerts after it ends. "You will receive a single alert after the first failed run in 1 minute, and then aggregated alert every 20 minutes". Channels: email, Slack, in Console. `doc` <https://docs.apify.com/platform/monitoring>

### ScrapydWeb

- Scrapyd cluster management with log categorization, statistics and progress visualization. `doc` <https://github.com/my8100/scrapydweb> Last release v1.6.0 on 2025-02-16, last commit 2025-02-19, so it is effectively idle.
- All in `default_settings.py`: `ENABLE_MONITOR` False, `POLL_ROUND_INTERVAL` 300, `POLL_REQUEST_INTERVAL` 10, `ALERT_WORKING_DAYS` and `ALERT_WORKING_HOURS`, `ON_JOB_RUNNING_INTERVAL` 0, `ON_JOB_FINISHED` False, `LOG_*_THRESHOLD` 0, `LOG_*_TRIGGER_STOP`, `LOG_*_TRIGGER_FORCESTOP`, `JOBS_RELOAD_INTERVAL` 300, `DAEMONSTATUS_REFRESH_INTERVAL` 10, `ENABLE_SLACK_ALERT`, `ENABLE_TELEGRAM_ALERT`. Stop and force stop still act outside working hours; only the message is suppressed. `src` <https://github.com/my8100/scrapydweb/blob/HEAD/scrapydweb/default_settings.py>
- The Jobs row action is Start, Stop or Cancel by state, with a `forcestop` path. `src` <https://github.com/my8100/scrapydweb/blob/HEAD/scrapydweb/templates/scrapydweb/jobs.html> Scrapyd signals a running job to terminate. `doc` <https://github.com/scrapy/scrapyd/blob/HEAD/docs/api.rst>
- Complaints: alerts that fail to send <https://github.com/my8100/scrapydweb/issues/249>; a timer page slow with long history <https://github.com/my8100/scrapydweb/issues/126>.

### k9s

- "ctrl-d: To delete a resource (TAB and ENTER to confirm)", "ctrl-k: To kill a resource (no confirmation dialog…)", and `--readonly` with "all cluster modification commands disabled". Plugins have `confirm` and `dangerous`, the latter hidden in read-only mode. `doc` <https://raw.githubusercontent.com/derailed/k9s/master/README.md>
- `refreshRate` default 2, minimum 2.0, described as UI poll intervals; k9s also "continually watches Kubernetes for changes". `doc` <https://k9scli.io/topics/config/>, README above. `:pulses` opens a cluster overview. `doc` <https://k9scli.io/topics/commands/>
- Complaints: disable Ctrl-K (17 comments, closed in 2020, with later comments still asking) <https://github.com/derailed/k9s/issues/319>, and a closed PR adding that setting <https://github.com/derailed/k9s/pull/3824>; a namespace deleted by accident at speed <https://github.com/derailed/k9s/issues/1016>; a confirmation modal that never took focus, so controls drove the screen behind it <https://github.com/derailed/k9s/issues/3014>; showing several resource types at once (29 comments) <https://github.com/derailed/k9s/issues/771>; a low-memory warning that misread cached memory <https://github.com/derailed/k9s/issues/1587>.

## Patterns most of them share

1. **The icon carries one verdict, not a list.** Status tools reduce to the worst state: Sprocket, CCMenu, Tailscale's exclamation mark, Docker's moon, and Octocheck and Deplog by colour. Numbers on the icon appear only in system meters (Stats, iStat Menus).
2. **Clicking opens a short list, and depth lives elsewhere.** OrbStack hands off to logs and terminal, Docker to its Dashboard, Tailscale to its window, CCMenu and the CI watchers to the web. Every full run view here is a window, a page or a terminal.
3. **Polling is the norm, and streams are for logs.** Rates: k9s 2 s; Prefect 5 s on a live run and 30 s on lists; Temporal 10 to 15 s; Dagster 15 s; CI watchers 15 to 30 s; ScrapydWeb 300 s. The exceptions are Dagster's run logs and k9s's Kubernetes watch.
4. **Rate follows activity.** Faster while something runs (Sprocket), stop at a terminal state (Prefect), skip hidden views and slow down when the source is slow (Dagster), back off on battery (Sprocket). A manual refresh is common (ActionBar, requested for Temporal).
5. **Stopping comes in grades, and the words say which.** Apify immediate or graceful, Dagster safe or force, ScrapydWeb stop or force stop, Temporal cancel or terminate, k9s delete or kill, Prefect cancel, pause or suspend. The harsher grade carries the warning (Dagster) or is a separate action.
6. **Guards are modal, name the scope and state the consequence.** Temporal and Dagster state a count, a destructive button or a warning. Temporal adds an optional reason. Temporal and k9s also offer a read-only switch, and Temporal has one per action. No tool here requires typing a name to stop a run, as far as I found.
7. **Notifications fire on transitions to bad states, and floods are the top complaint.** Coalescing appears in Sprocket and Apify. Duration conditions appear in iStat Menus and are requested for Stats. Per-type toggles appear in Docker, with errors not silenceable. On macOS, several depend on the OS permission.
8. **Waiting for a person is its own visible state.** CCMenu's grey pause, Sprocket's action_required, Prefect's Resume form, Temporal's Task Failures view, Tailscale's reauthentication alerts.
9. **Bulk control is a recurring request.** Users ask for it in Prefect, Dagster and Temporal (terminate, signal, schedules), several of them while testing or developing; Docker's Dashboard already offers bulk.
10. **The tool's own health must be visible.** Tailscale's health items, Ollama's "Not responding", OrbStack's silent VM death, Docker's wrong status, Tailscale's hidden icon read as a dead app.

## What stands out

- **Tailscale's three levels:** menu, mini player, window, all kept. It also detects that its own icon is hidden.
- **Dagster's termination dialog and refresh hook:** the most complete guard and the most careful polling found here, both readable in source.
- **Apify's one sentence per run:** a free-text status the program writes, with an aggregate rule for failures (1 minute, then every 20).
- **Apify's Resurrect with changed limits:** the dialog adjusts the limit that stopped the run.
- **ScrapydWeb's rule-based stop:** a log threshold can stop or force stop a job. It also has working-hours quiet that leaves the actions running.
- **Prefect's triggers on absence:** notice what did not happen, such as a run stuck for 30 minutes.
- **CCMenu's learned estimate:** it times builds itself to estimate what the feed lacks.
- **Stats' per-module off switch and interval:** published with a statement of which modules cost most, yet it still draws overhead complaints.
- **k9s's unconfirmed kill:** the cautionary case, still documented in 2026 after years of requests to disable it.
- **Ollama as the minimal end:** a launcher with no state, and its complaints are about not knowing what still runs.

## How the evidence meets Goodfetch's terms and operations

Observations for the design sessions, not recommendations.

- **Push is available.** `events.subscribe`, with `run.progress` and `run.axes_changed`, gives the Kennel what most of these tools poll for. A Subscription has credits, and telemetry events replace unsent ones, which already coalesces (`GLOSSARY.md`, Subscription). Polling evidence applies to `daemon.health` and `needs_you.list` only if they lack events; I did not check this.
- **Stop grades differ here.** The glossary says a Limit, a timeout, an abort and a drain all stop a Run at its next Safe point. `run.stop` stops with a Checkpoint and leaves the Run resumable (ADR 0102). So competitors' graceful-or-immediate choice has no direct match among the listed operations: none is a stop that skips the Safe point. `run.abort` also acts on a Run no engine runs (ADR 0031's note on #219). Wording on the controls matters because "abort" means immediate at Apify.
- **A wait is bounded.** `run.stop` fails with `deadline_exceeded` past `timeout.close`, and the Run still stops (ADR 0102). Dagster's progress bar and Temporal's reason both address this moment.
- **Pause has a specific meaning.** ADR 0037's row RT-11 says "Pause is checkpoint, stop and resume, not an in-memory pause". Prefect's pause-or-suspend split is the nearest analogue. Human pause is a separate state of a Session's work, closer to Prefect's Resume form.
- **`needs_you.list` matches pattern 8,** and the icon-verdict order in pattern 1 is where it would compete with failures. Sprocket and CCMenu order their states differently, so there is no settled order.
- **Run report** corresponds to the end-of-run views; Apify's and Crawlee's status sentence has no named counterpart in the terms I was given. `run.progress` may carry one; I did not check.
- **Bulk.** The listed operations act per Run, and I saw no bulk form.
- **Glossary drift.** The Needs you entry still says "The Console's one list", though ADR 0137 removes the Console according to map #1823.

## What I could not verify

- **Nothing was run.** No tool was installed, so icon looks, click behaviour and dialogs come from documents, source and user reports. Source was read, not executed.
- **OrbStack:** icon states, refresh, notifications and delete guards are undocumented, and its app is closed source. The form (a nested menu) is inferred from a bug report.
- **Docker Desktop:** the current menu's contents, refresh and confirmations are undocumented in the pages I read. An older mirror describes the menu, which I did not rely on.
- **Tailscale:** the popover or menu form may vary by macOS version; reports use both words. The app is closed source. Guards not found.
- **Ollama:** how the menu updates on open was not read. "Not responding" comes from one issue, and I did not locate it in source.
- **iStat Menus:** update values were not found, and I found no public tracker. Rules help page not read.
- **ActionBar:** the App Store price conflicts with "free" on the author's page; both may be true for different builds. Click behaviour is undocumented.
- **CCMenu:** the unit and the rewrite's default poll rate were not confirmed.
- **Temporal:** how the 10 s timer and the 15 s toggle combine was not traced.
- **Prefect:** the rates come from `ui-v2`; whether it is the default UI in 3.9.1 was not checked. Open-source availability of automations is inferred from per-action availability notes. The default pause timeout of 3600 s came from a search result <https://reference.prefect.io/prefect/flow_runs/> and is not relied on.
- **Dagster:** whether the logs subscription runs over WebSocket in every deployment was not checked.
- **Apify:** the Console's refresh, abort dialog wording and exact tab names are undocumented; the Console settings page names no notification types <https://docs.apify.com/platform/console/settings>. I found no public tracker for the Console, so its row has no complaints.
- **ScrapydWeb:** the absence of a stop confirmation is from one template's text, and a script elsewhere could add one.
- **k9s:** the outcome of PR 3824 is only known as "closed". The notification row means none found.
- **Issue evidence** is sampled, not counted. Comment counts undercount demand, some items were read by title only (marked), and GitHub's search API rate limit stopped further tracker queries at the end. Search matched poorly on Prefect's and Dagster's trackers, so their complaint lists are the thinnest.
- **Aggregators:** none is relied on. A reader mirror of ActionBar's page and third-party OrbStack and Tailscale write-ups appeared in searches and were replaced by primary sources or dropped.
