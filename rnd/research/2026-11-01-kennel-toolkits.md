# Which Rust tray and UI toolkits fit the Kennel?

Date: 2026-11-01. Ticket: [#1824](https://github.com/goodfetch/goodfetch/issues/1824), on the map [#1823](https://github.com/goodfetch/goodfetch/issues/1823).

Scope: facts and a ranking of fit against published evidence. The decision is the owner's. Nothing was built or run for this note, so every figure comes from a published source and every behaviour comes from reading source or documentation.

Registry data comes from the crates.io API (`https://crates.io/api/v1/crates/<name>`), which answered on 2026-11-01. Repository data comes from the GitHub API on the same day. Source references are to each repository's default branch that day unless a tag is named.

## Summary

- **Most candidates' tray is the same code.** Tauri, Dioxus desktop and Freya all sit on `tray-icon`. Slint is the exception, with its own backends: `objc2-app-kit`, `windows` and `ksni` ([Cargo.toml](https://github.com/slint-ui/slint/blob/master/internal/core/Cargo.toml)). The real choice is what draws anything beyond a native menu.
- **`tray-icon` and `muda` have no serious rival** as a tray layer: maintained in October 2026, MIT or Apache-2.0, native `NSStatusItem` and `NSMenu`.
- **Published measurements are thin.** No source measures any candidate as a tray tool on an M3 Max. No source measures egui, iced, Slint, gpui or Dioxus for memory or idle CPU on macOS at all. See [Measured figures](#measured-figures).
- **Three findings can change the shape of the design:**
  1. An open bug says `tao` auto-dismisses an open tray menu ([tao#1324](https://github.com/tauri-apps/tao/issues/1324)). `winit` registers its observers the same way, and no source says whether it has the same effect.
  2. Slint's licences are not on the allowlist, so Slint fails `cargo-deny` until the owner amends it.
  3. `STYLE.md` requires every dependency to build on static musl. No source shows a graphical toolkit doing so, and this was not tested. See [Project constraints](#project-constraints).

## Ranking of fit

The ranking weighs: tray and popover support on macOS, footprint evidence, licence and dependency bar, Tokio coexistence, accessibility, and Rust-only UI. It is not a recommendation.

| Rank | Stack | Why it ranks here | Main gap |
|---|---|---|---|
| Baseline | `tray-icon` + `muda` on `tao` or `winit`, native menu only | Smallest published footprint (800 KB, about 20 MB). Native accessibility. No renderer. | A menu cannot show charts or rich data. Live refresh is unproven (see [menu refresh](#how-a-native-menu-refreshes-while-open-on-macos)). |
| 1 | `tray-icon` + `muda` + egui/`eframe` | Permissive, released 2026-10-15, AccessKit on by default, `Context` is `Send + Sync`, reactive repaint. A real tray app uses this mix. | Hidden root window bug is open ([egui#8459](https://github.com/emilk/egui/issues/8459)). The default wgpu renderer costs about 2.4 to 3.3 times glow's working set on Windows. No macOS figures. |
| 2 | Slint 1.18 | Built-in `SystemTrayIcon`, tray-only programs are valid, AccessKit, native macOS menus. | Licence needs an allowlist change and attribution. Its documented tray API exposes no icon position to anchor a popover. |
| 3 | `tray-icon` + `muda` + iced 0.14 | MIT, `daemon` runs with no window, which suits a tray tool. | No upstream accessibility. No built-in tray. Last `iced` release is 11 months old. |
| 4 | gpui family | Real macOS menu-bar apps with popovers exist, and `NSPanel` is built in. | No status-item API. `gpui` on crates.io is a year old, lacks AccessKit, and git dependencies are denied. Linux and Windows tray would be separate code. |
| 5 | Tauri 2 | The most complete tray and panel ecosystem. Highest maintenance activity. | The UI is a web view, not Rust widgets. The one measured macOS idle figure that states its counting is 109 MB with web view helpers, against about 20 MB for the baseline. |
| 6 | Dioxus desktop 0.7 | Rust components, re-exports `tray-icon`. | Same web view cost as Tauri with a smaller tray ecosystem. Open macOS tray bug ([dioxus#3635](https://github.com/DioxusLabs/dioxus/issues/3635)). Native renderer is labelled experimental. |
| 7 | Freya 0.4 | MIT, AccessKit, `tray-icon` with KSNI. | Skia's build script downloads binaries by default, which the dependency bar forbids. Reported macOS 26 tray icon failure ([tray-icon#273](https://github.com/tauri-apps/tray-icon/issues/273)). |
| Not fitting | Xilem, Floem, Makepad, Vizia, GTK4 and Relm4, cacao, `muri`, `tray`, `tray-item`, `systray2`, `native-windows-gui`, FLTK, `libcosmic` | See [Other candidates](#other-candidates). | |

## Maintenance and licence

Stable release, and the newest prerelease where one exists. "Pushed" is the repository's last push.

| Candidate | Latest stable, date | Prerelease | Pushed | Licence | Allowlist |
|---|---|---|---|---|---|
| `tray-icon` | 0.26.1, 2026-10-07 | | 2026-10-26 | MIT OR Apache-2.0 | Yes |
| `muda` | 0.21.2, 2026-10-08 | | 2026-10-29 | Apache-2.0 OR MIT | Yes |
| `tao` | 0.37.2, 2026-10-23 | | 2026-10-28 | Apache-2.0 | Yes |
| `winit` | 0.30.13, 2026-03-02 | 0.31.0-beta.4, 2026-10-24 | 2026-11-01 | Apache-2.0 | Yes |
| `egui`, `eframe` | 0.37.0, 2026-10-15 | | 2026-10-30 | MIT OR Apache-2.0 | Yes |
| `slint` | 1.18.1, 2026-09-21 | | 2026-11-01 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | **No** |
| `iced` | 0.14.0, 2025-12-07 (`iced_winit` 0.14.1, 2026-09-13) | | 2026-10-31 | MIT | Yes |
| `tauri` | 2.12.5, 2026-10-21 | | 2026-10-31 | Apache-2.0 OR MIT | Yes |
| `gpui` | 0.2.2, 2025-10-22 | | 2026-11-01 (Zed) | Apache-2.0 | Yes |
| `gpui-ce` (fork) | 0.3.0, 2026-10-20 | | 2026-10-30 | Apache-2.0 | Yes |
| `gpui-kit` (components) | 0.8.1, 2026-10-26 | | 2026-11-01 | Apache-2.0 | Yes |
| `dioxus`, `dioxus-desktop` | 0.7.10, 2026-07-30 | 0.8.0-alpha.3, 2026-10-16 | 2026-11-01 | MIT OR Apache-2.0 | Yes |
| `freya` | 0.4.3, 2026-08-30 | 0.5.0-rc.13, 2026-10-25 | 2026-11-01 | MIT | Yes |
| `objc2`, `objc2-app-kit` | 0.6.5, 2026-10-05; 0.3.2, 2025-10-04 | | 2026-10-17 | MIT; Zlib OR Apache-2.0 OR MIT | Yes |
| `ksni` | 0.3.7, 2026-10-20 | | not checked | Unlicense | Yes |
| `accesskit` | 0.25.2, 2026-10-25 | | 2026-10-25 | MIT OR Apache-2.0 | Yes |

Allowlist: [`license-allowlist.toml`](../../license-allowlist.toml) and its copy in [`deny.toml`](../../deny.toml). "Every license not listed is denied", the GPL family included. "Yes" means the crate's own licence only. The transitive trees were not checked, since that needs `cargo deny` on each candidate.

### Slint's licence

Slint is free under its [Royalty-free licence](https://raw.githubusercontent.com/slint-ui/slint/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md) for desktop use. It requires either the `AboutSlint` widget on a screen reachable from the top-level menu (or a splash screen), or the Slint badge on a public web page, ideally where binaries are downloaded. It also forbids distributing an application that exposes Slint's APIs. Using it would need a new allowlist entry, which is the owner's to accept. ([Slint FAQ](https://github.com/slint-ui/slint/blob/master/FAQ.md#licensing).)

### An advisory that no longer applies

`tao` depends on `gtk 0.18` on Linux ([Cargo.toml](https://github.com/tauri-apps/tao/blob/dev/Cargo.toml)), as does `tray-icon`'s default backend through `muda`. RustSec once listed GTK3 bindings as unmaintained, and `deny.toml` sets `unmaintained = "all"`. [RUSTSEC-2024-0415](https://rustsec.org/advisories/RUSTSEC-2024-0415.html) is now withdrawn, because the repository was unarchived (last modified 2026-09-08). The sibling GTK3 advisories were not each checked.

## Platform support and accessibility

| Candidate | macOS menu-bar icon | Popover | Linux tray | Windows tray | Accessibility |
|---|---|---|---|---|---|
| `tray-icon` + `muda` | `NSStatusItem`, `NSMenu` ([source](https://github.com/tauri-apps/tray-icon/blob/dev/src/platform_impl/macos/mod.rs)) | Not provided. `Click` events carry the icon's `rect` for placing your own window ([lib.rs](https://github.com/tauri-apps/tray-icon/blob/dev/src/lib.rs)) | AppIndicator (GTK3, default) or KSNI over D-Bus. Left-click and right-click menu settings are unsupported on Linux, and on AppIndicator a menu cannot be replaced once set | Yes | Native AppKit controls: `muda` draws nothing itself, as [`muri`'s README](https://github.com/MattJackson/muri) notes |
| `tao`, `winit` | Neither has a tray. Both drive the loop that `tray-icon` needs | Plain window. Both set the Accessory activation policy ([tao](https://github.com/tauri-apps/tao/blob/dev/src/platform/macos.rs)) | Via `tray-icon` | Via `tray-icon` | None of their own. `accesskit_winit` exists for `winit` |
| egui, `eframe` | Via `tray-icon` ([shelv](https://github.com/twop/shelv/blob/HEAD/src/main.rs)) | Own window. `run_native` and `create_native` give event-loop access. Starting hidden is buggy ([egui#8459](https://github.com/emilk/egui/issues/8459), fix [#8737](https://github.com/emilk/egui/pull/8737) still open) | Via `tray-icon` | Via `tray-icon` | AccessKit, on by default ([Cargo.toml](https://github.com/emilk/egui/blob/main/crates/eframe/Cargo.toml)). An occluded window does not answer it ([egui#8671](https://github.com/emilk/egui/issues/8671)) |
| Slint | [`SystemTrayIcon`](https://docs.slint.dev/latest/docs/slint/reference/window/systemtrayicon/), added in 1.17.0 ([changelog](https://raw.githubusercontent.com/slint-ui/slint/master/CHANGELOG.md)). On macOS `clicked` fires only with no populated menu | Separate `Window`. The tray has no window of its own and the page exposes no icon position | StatusNotifierItem through `ksni`. Plain X11 trays unsupported. GNOME needs an extension | `Shell_NotifyIcon` | AccessKit through the `winit` backend's `accessibility` feature ([Cargo.toml](https://github.com/slint-ui/slint/blob/master/internal/backends/winit/Cargo.toml)) |
| iced | None upstream ([iced#124](https://github.com/iced-rs/iced/issues/124), open since 2019). A 2026-10-30 comment says `tray-icon` runs on the existing `winit` loop on macOS and Windows | Own window through [`daemon`](https://docs.rs/iced/latest/iced/fn.daemon.html), which runs with no window until one is opened | Via a tray crate. The same comment names `tray-icon`'s GTK need as the hard part | Via a tray crate | None upstream. [iced#3111](https://github.com/iced-rs/iced/pull/3111) is a draft covering only Button and Text. [iced#3281](https://github.com/iced-rs/iced/pull/3281) closed |
| Tauri | [Built in](https://v2.tauri.app/learn/system-tray/) on `tray-icon` | [`tauri-nspanel`](https://github.com/ahkohd/tauri-nspanel) 2.1.1 (2026-10-08), [`tauri-nspopover-plugin`](https://github.com/freethinkel/tauri-nspopover-plugin), `tauri-plugin-positioner` 2.4.0 | Icon and right-click menu. Its docs say tray events are not emitted on Linux | Yes | The OS web view's. No primary source was read for this |
| gpui | No status-item API in the two platform files read. [claudebar](https://github.com/gautham-v/claudebar/blob/HEAD/src/status_item.rs) writes its own with `objc2`, and [tray-icon#323](https://github.com/tauri-apps/tray-icon/issues/323) asks how to combine them | `WindowKind::PopUp` is a non-activating `NSPanel` ([window.rs](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/window.rs)). `AnchoredPopup` is rejected on macOS ([platform.rs](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/platform.rs)) | None found | None found | AccessKit on Zed's main branch and in `gpui-ce` 0.3.0. Not among `gpui` 0.2.2's published dependencies |
| Dioxus desktop | Re-exports `tray-icon` ([trayicon.rs](https://github.com/DioxusLabs/dioxus/blob/main/packages/desktop/src/trayicon.rs)). [dioxus#3635](https://github.com/DioxusLabs/dioxus/issues/3635), "Tray icon on macos not working", is open | `tao` window | Via `tray-icon` | Via `tray-icon` | The OS web view's. No primary source was read for this |
| Freya | `tray-icon` with `ksni` ([Cargo.toml](https://github.com/marc2332/freya/blob/HEAD/Cargo.toml)). One user reported no icon on macOS 26.1 ([tray-icon#273](https://github.com/tauri-apps/tray-icon/issues/273)) | Own window | KSNI | Via `tray-icon` | AccessKit (same Cargo.toml) |

### macOS 26 caution

[tray-icon#273](https://github.com/tauri-apps/tray-icon/issues/273) reported no menu-bar icon on macOS Tahoe betas when running outside a bundle. A maintainer found no general issue and pointed at overfull menu bars. A later reporter suspected a Dioxus and Freya cause. The thread was closed without a confirmed root cause, so the Kennel's bare-binary case on current macOS stays unverified.

## Threading and Tokio

| Candidate | What must be on the main thread | How other threads reach it | Tokio |
|---|---|---|---|
| `tray-icon` | macOS: create it on the main thread, after the event loop is running, and keep an event loop running there ([lib.rs](https://github.com/tauri-apps/tray-icon/blob/dev/src/lib.rs)). KSNI runs its own worker thread | `TrayIconEvent::set_event_handler` forwarding to an `EventLoopProxy` | Independent |
| `muda` | macOS: all menu use. "Most functionality will panic" elsewhere ([lib.rs](https://github.com/tauri-apps/muda/blob/dev/src/lib.rs)) | Same. A `snapshot` feature adds thread-safe snapshot types | Independent |
| `tao` | macOS: the `EventLoop` panics elsewhere ([source](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/macos/event_loop.rs)) | `Proxy` is `Send + Sync` | Independent |
| `winit` | macOS: same panic ([source](https://github.com/rust-windowing/winit/blob/v0.30.13/src/platform_impl/macos/event_loop.rs)) | `EventLoopProxy` is `Send + Sync` | Independent |
| egui, `eframe` | The `winit` event loop | [`Context`](https://docs.rs/egui/latest/egui/struct.Context.html) is `Send + Sync`, and `request_repaint` from another thread wakes the UI | Independent. A Tokio task can hold a `Context` clone |
| Slint | "The event loop must run in the main thread, in most backends", and components are created on that thread ([docs](https://docs.slint.dev/latest/docs/rust/slint/)) | `invoke_from_event_loop` | [`spawn_local`](https://docs.rs/slint/latest/slint/fn.spawn_local.html) cannot drive Tokio. Docs advise `async_compat`, which allocates a shared multi-threaded runtime implicitly |
| iced | The `winit` event loop | `Task` and `Subscription` messages | The `tokio` feature's executor calls `Runtime::new()` itself ([tokio.rs](https://github.com/iced-rs/iced/blob/master/futures/src/backend/native/tokio.rs)). The default is `thread-pool`. The `Executor` trait's `new()` takes no arguments |
| Tauri | The `tao` loop | `AppHandle` (from knowledge of the API, not re-read today) | [`async_runtime::set(handle)`](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/src/async_runtime.rs) adopts a runtime the program built |
| gpui | Its `Application`, with an executor integrated with the platform loop ([README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)) | Channels into a gpui task, as claudebar does | Zed's [`gpui_tokio`](https://github.com/zed-industries/zed/blob/main/crates/gpui_tokio/src/gpui_tokio.rs) has `init_from_handle`, but it is not on crates.io |
| Dioxus desktop | "*Must* be spawned on the main thread" ([launch.rs](https://github.com/DioxusLabs/dioxus/blob/main/packages/desktop/src/launch.rs)) | `Proxy` from `tao` | Uses the current runtime when one is entered and it is multi-threaded, and panics on current-thread. Otherwise it builds its own |

## Project constraints

### ADR 0060 and ADR 0081

[ADR 0060](../../docs/adr/0060-distributed-and-crash-behaviour-is-simulated-on-tokio-model-checked-and-faulted-at-io-seams.md) bans `std::thread::spawn` and its siblings outside `tasks`, allows runtime construction only in `main` and test helpers, and bans direct sockets. [ADR 0081](../../docs/adr/0081-each-sdk-follows-its-languages-concurrency-model-and-shapes-events-by-meaning.md) lets the Rust SDK's connection build a small client runtime through the helper `main` uses. It says the crate's "futures and streams only exchange messages with that runtime, so they run on any executor".

What this means for a graphical tool, read from those texts and `clippy.toml`:

- **The main thread conflict is smaller than it looks.** A plain `fn main` can build a multi-thread runtime through that helper, hold it, and run the UI loop on the main thread. Tokio starts its own workers, so the Kennel writes no bare thread spawn and needs no `#[tokio::main]`.
- **A toolkit's internal threads and runtimes do not trip the lints.** Clippy's `disallowed-methods` checks this workspace's code. Iced's `Runtime::new()`, the KSNI worker and `async_compat`'s runtime are third-party. They would still add a second runtime next to the SDK's, which is a design cost rather than a lint failure.
- **The SDK may remove the Kennel's own runtime altogether.** Under ADR 0081 its futures run on any executor, including a toolkit's own such as Slint's `spawn_local` or gpui's. This code is planned, not written ([#1223](https://github.com/goodfetch/goodfetch/issues/1223)), so it cannot yet be checked.
- **`goodfetch::blocking` waits on a channel**, so calling it on the UI thread would freeze the UI, and on macOS the menu with it.
- **Tasks still go through trackers.** `Runtime::spawn` and `Handle::spawn` are banned outside an owner's `TaskTracker` (`clippy.toml`).
- **Reaching the ops listener directly would need an exception.** `reqwest::ClientBuilder::build` is banned outside the HTTP fetch tier's transport, and socket types are banned generally. ADR 0069 and the `goodfetch-conformance` note in ADR 0060 are precedents for an `#[expect]` naming an ADR.

### The dependency bar in `STYLE.md`

`STYLE.md`'s [Dependencies](../../STYLE.md) section requires that a dependency be on the allowlist and maintained, that it "builds on every target: static musl (ADR 0024), macOS and Windows", and that "its build script downloads nothing".

- **Static musl is an open question for every graphical candidate.** No source shows these toolkits building and running on static musl, and GTK or web view system libraries make that doubtful. This was not tested. The Kennel is part of no Product, so whether the bar applies to it unchanged is an owner decision.
- **`skia-bindings` downloads prebuilt binaries in `build.rs`** ([rust-skia README](https://github.com/rust-skia/rust-skia)). `FORCE_SKIA_BUILD=1` builds from source. This affects Freya, and Slint only if its Skia renderer is chosen. Slint's other renderers are [FemtoVG and software](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/). Freya depends on a `freya-skia-safe` fork, whose build script was not read.
- **`deny.toml` has `allow-git = []`**, so a git dependency on Zed's `gpui` fails. Only the crates.io releases are usable: `gpui` 0.2.2, `gpui-ce` 0.3.0, or `gpui-kit`, which pulls `gpui-pre =0.3.10` and `gpui-fast`. `gpui` 0.2.2 also depends on `cocoa` and `objc`, whose RustSec status was not checked.

## How a native menu refreshes while open on macOS

**The AppKit model.**
- While a menu is open, AppKit runs a nested loop in the event-tracking run loop mode, "the mode set when tracking events modally" ([Apple](https://developer.apple.com/documentation/foundation/runloop/mode/eventtracking)). Timers in the default mode do not fire.
- [`NSMenuDelegate`](https://developer.apple.com/documentation/appkit/nsmenudelegate) offers `menuNeedsUpdate:` and `menuWillOpen:` before display, and `menuDidClose:` after.
- Long-standing practice is to add a timer in tracking mode during `menuWillOpen:`, invalidate it in `menuDidClose:`, change the items, and call `-[NSMenu update]`. [This thread](https://lists.apple.com/archives/cocoa-dev/2012/Apr/msg00351.html) and [a Stack Overflow archive](https://kiwix.lema.org/stackoverflow.com_en_all_2023-11/questions/6301338/update-nsmenuitem-while-the-host-menu-is-shown) describe it, and both date from 2008 to 2014. They were seen through a search summary only, and the direct fetch of a related list message failed. Apple documents no such recipe that this research found.

**What `muda` and `tray-icon` provide.**
- `MenuItem::set_text` calls `setTitle:` on the live `NSMenuItem`, so a change reaches an open menu's item object ([source](https://github.com/tauri-apps/muda/blob/dev/src/platform_impl/macos/mod.rs)).
- `MudaMenuDelegate` implements none of the delegate's methods, so `muda` exposes no will-open hook, and neither crate has an open or close event.
- `tray-icon` attaches the menu only for the click: `setMenu(menu)`, `performClick`, `setMenu(None)`, where `performClick` "runs a nested event loop". The `Click` Down event is sent first, through the handler when one is set. So a handler can refresh the menu just before it opens ([source](https://github.com/tauri-apps/tray-icon/blob/dev/src/platform_impl/macos/mod.rs)). This is read from source, not run.
- **Replacing the whole menu while it is displayed crashed**, by use-after-free ([muda#328](https://github.com/tauri-apps/muda/issues/328)). `muda` 0.20.0 (2026-09-11) lists a matching fix ([#361](https://github.com/tauri-apps/muda/pull/361)). The link between the two is inferred from the changelog text.

**The event loop question, which is unresolved.**
- `winit` 0.30.13 and `tao` both register their observers, waker timer and proxy source in `kCFRunLoopCommonModes`, which includes tracking mode ([winit observer](https://github.com/rust-windowing/winit/blob/v0.30.13/src/platform_impl/macos/observer.rs), [tao observer](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/macos/observer.rs)). From that, a proxy event sent from Tokio should reach the handler during tracking, which is what a live refresh needs.
- [tao#1324](https://github.com/tauri-apps/tao/issues/1324) (open, 2026-09-05, `tao` 0.37.0, macOS 26.6.2) reports that the same registration closes an open tray menu by itself, with a minimal reproducer. Its proposed fix is to stop registering in tracking mode, which would also stop that delivery. No fix PR was found, and the issue has no maintainer reply. The diagnosis is the reporter's.
- Whether `winit` has the same effect is unknown. Its `cleared` handler runs under the same observer registration, but its `queue_closure` deliberately uses the default mode only, so that handlers do not run inside modal tracking.
- No source demonstrates a `tray-icon` menu updating live while open under either loop. A short run on the owner's Mac would settle this.

On Windows the menu blocks the loop's `about_to_wait` only, according to the last comment on the closed [tray-icon#358](https://github.com/tauri-apps/tray-icon/issues/358).

## How a popover window is done

All three shapes start the same way: turn off the left-click menu (`with_menu_on_left_click(false)` or the equivalent) so the click arrives as an event, and run as an Accessory app with no Dock icon.

| Shape | How | Evidence | Limits |
|---|---|---|---|
| Borderless window under the icon | Place a window using the `Click` event's `rect`. Close on Esc, outside click and focus loss. | [claudebar](https://github.com/gautham-v/claudebar/blob/HEAD/src/main.rs) (gpui and `objc2`) does exactly this | An ordinary window activates the app. Space and full-screen behaviour need setting by hand through `objc2`. |
| Non-activating `NSPanel` | Panel with `NonactivatingPanel`, so the frontmost app keeps focus. | gpui's `WindowKind::PopUp`, [`tauri-nspanel`](https://github.com/ahkohd/tauri-nspanel), [`muri`](https://github.com/MattJackson/muri) | `tao` and `winit` expose no panel window. A toolkit on them would have to change the window's class itself, as a Tauri plugin must. This research found no egui, iced or Slint example. |
| Real `NSPopover` | Set a `contentViewController` and call `show(relativeTo:of:preferredEdge:)` on the status button. It has `transient` and other closing behaviours ([Apple](https://developer.apple.com/documentation/appkit/nspopover)). | [`tauri-nspopover-plugin`](https://github.com/freethinkel/tauri-nspopover-plugin), last pushed 2026-05-16 | Needs the UI to live in an `NSView`. No published egui, iced or gpui example was found. |

Linux has no reliable equivalent: left-click settings are unsupported there, and Tauri's docs say tray events are not emitted. Whether `tray-icon`'s KSNI backend sends `Click` with a usable `rect` was not established. Slint's docs say its `clicked` fires on Linux through StatusNotifierItem, with the gesture left to the desktop.

## Measured figures

Every figure below is as published, on the setup stated. **None was taken on an M3 Max, and none is comparable across rows.** Figures for older versions may not hold today.

| Stack | Binary size | Memory | Idle CPU | Setup and source |
|---|---|---|---|---|
| `tray-icon` 0.21 + `tao` 0.34 + `objc2` (no UI toolkit) | ~800 KB (`opt-level = "z"`, LTO, stripped) | 11 MB at launch, ~20 MB steady (`phys_footprint`) | No source | Author's report, M-series MacBook Pro ([featherbar](https://github.com/nim444/featherbar)) |
| `eframe` 0.34 glow | 17.7 MB release, 5.62 MB with LTO | 54 MB working set, 31 MB VRAM | No source | Windows, `run_simple_native`, default features off ([egui#7761](https://github.com/emilk/egui/issues/7761)) |
| `eframe` 0.34 wgpu | 27.7 MB release, 11.3 MB with LTO | 129 MB (Vulkan) or 176 MB (DX12) working set; 265 or 215 MB VRAM | No source | Same |
| `eframe` 0.33.3 | 13.9 MB release, 4.44 MB with LTO | 53 MB working set, 31 MB VRAM | No source | Same |
| egui `hello_world`, 2023 | 18 MB | No source | No source | Plain `--release`, Ubuntu 20.04 ([Kalbertodt](http://lukaskalbertodt.github.io/2023/02/03/tauri-iced-egui-performance-comparison.html)) |
| iced `todos`, 2023 | 17 MB | No source | No source | Same |
| Tauri `helloworld`, 2023 | 5 MB | No source | No source | Same |
| Tauri demo, 2026 | 57 MB bundle | 109 MB idle, including web view helpers (main process ~36 MB) | No source | macOS, machine unstated, Activity Monitor ([Better Stack](https://betterstack.com/community/guides/scaling-nodejs/tauri-vs-electron-vs-deno-vs-electrobun/), updated 2026-07-20) |
| Tauri demo, 2025 | 8.6 MiB `.app` | ~172 MB with 6 windows | No source | MacBook Pro, N=1 ([Hopp](https://dev.to/gethopp/tauri-vs-electron-performance-bundle-size-and-the-real-trade-offs-1el4)) |
| Tauri v2 menu-bar monitor | < 7 MB | ~15 MB | < 0.1% | Self-reported, Apple M1, no method given. It cannot be said whether this counts web view helpers ([Better Resource Monitor](https://silicon-monitor.pages.dev/)) |
| Tauri, vendor claim | "as little as 600KB" | No source | No source | No conditions given ([tauri.app](https://tauri.app/)) |
| Dioxus desktop, vendor claim | "Portable <3mb binaries" | No source | No source | No conditions given ([README](https://github.com/DioxusLabs/dioxus/blob/main/README.md)) |
| Slint Rust printer demo, 2021 | 6.4 MB | No source | No source | Release build, then named SixtyFPS ([Slint](https://old.slint.dev/thisweek/2021-11-15)) |
| Slint, vendor claim | No source | "less than 300KiB RAM" | No source | Links to an RP2040 microcontroller demo, so not a desktop figure ([slint.dev](https://slint.dev/)) |

Three conflicting or qualitative points:

- The Better Stack and Better Resource Monitor figures for Tauri differ by a factor of seven. Counting web view helper processes is one likely cause, though only Better Stack states its counting method.
- [tauri#5889](https://github.com/tauri-apps/tauri/issues/5889) (2022) disputes Tauri's own older memory benchmark because of shared memory accounting.
- Idle behaviour is documented only qualitatively. egui repaints on request ([Context docs](https://docs.rs/egui/latest/egui/struct.Context.html)). An eframe loop was reported spinning at idle on Wayland ([egui#8523](https://github.com/emilk/egui/issues/8523)). [iced#1867](https://github.com/iced-rs/iced/issues/1867) reported idle CPU use and was closed as a `wgpu` matter, without a figure.

## Real tools by stack

Star counts and last push are from the GitHub API today. The small projects are evidence that the approach works, not of maturity.

| Stack | Tools |
|---|---|
| `tray-icon` + `tao` + `objc2` | [featherbar](https://github.com/nim444/featherbar), macOS menu-bar monitor (6 stars, 2026-06-06) |
| egui + `tray-icon` | [shelv](https://github.com/twop/shelv), notes app with a template tray icon, `eframe` 0.36.2 on patched forks (18 stars, 2026-10-27). [Rerun](https://github.com/rerun-io/rerun) uses egui without a tray (11,675 stars) |
| gpui + `objc2` | [Zed](https://github.com/zed-industries/zed), the origin of gpui, without a tray. [claudebar](https://github.com/gautham-v/claudebar) (3 stars) and [swapbar](https://github.com/afatmustafa/swapbar) (18 stars) on `gpui` 0.2.2. [tidegauge](https://github.com/henrikogaard/tidegauge) (1 star) and [SysVitals](https://github.com/fynhq/sysvitals) (2 stars) on `gpui-kit` |
| Tauri | [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev), which has a tray module (155,517 stars) |
| iced | [Halloy](https://github.com/squidowl/halloy), IRC client (4,580 stars, no tray evidence checked). [sshoal](https://github.com/japananh/sshoal) appeared in an iced and `tray-icon` search, but its manifest was not confirmed |
| Slint | [LibrePCB](https://github.com/LibrePCB/LibrePCB) was found by name only and its use not confirmed. [slint.dev](https://slint.dev/) lists WesAudio's Prometheus plugin and OTIV products. [slint_systry_app](https://github.com/Guiguiprim/slint_systry_app) is a tray demo |
| Freya | [tray-icon#273](https://github.com/tauri-apps/tray-icon/issues/273) mentions an unnamed Freya app |
| Dioxus desktop | No tray tool was found |

## Other candidates

| Candidate | Status | Why it does not fit |
|---|---|---|
| `objc2-app-kit` directly | Maintained base of `muda`, `winit` and others | Fully native on macOS, but it covers only macOS, so Linux and Windows would need separate code |
| [`muri`](https://github.com/MattJackson/muri) 0.14.7 | Started 2026-09, 3 stars, MIT | Custom-drawn replacement for `muda` with an `NSPanel` popup and AccessKit. Too new, "macOS-first WIP" per its README, though the README's version text lags the release |
| [`tray`](https://github.com/nobane/tray-rs) 0.1.2 | 2025-12-02, 9 stars | Has egui and iced helpers but little use and no release since |
| Xilem and Masonry 0.4.0 | 2025-10-29, active repository | No tray found. Early-stage status not verified further |
| Floem 0.2.0 | 2024-11-14, repository active | No release in two years. No AccessKit in the root manifest (not proof of absence) |
| Makepad 1.0.0 | 2025-05-13, 1,523 recent downloads | Same manifest result. No tray found |
| Vizia 0.4.0 | 2026-04-23, 680 recent downloads | AccessKit, low use, no tray found |
| GTK4 0.11.5 and Relm4 0.11.0 | Maintained | GTK 4 has no `GtkStatusIcon` ([GNOME Discourse](https://discourse.gnome.org/t/what-to-use-instead-of-statusicon-in-gtk4-to-display-the-icon-in-the-system-tray/7175)), and needs the GTK runtime on macOS |
| cacao 0.4.0-beta2 | 2023-08-01, repository idle since 2025-02 | Stale |
| `tray-item`, `systray2`, `native-windows-gui` | Last release 2024, 2021, 2022 | Stale. `native-windows-gui` is Windows-only |
| FLTK 1.5.23 | 2026-05-03 | No tray found in this research |
| `libcosmic` | Not on crates.io, MPL-2.0 | Git dependencies are denied, and it targets the COSMIC desktop |

## What could not be verified

- **Any figure on an M3 Max**, and any tray-tool figure at all for egui, iced, Slint, gpui, Dioxus or Freya. No source gives macOS memory or idle CPU for them. No source gives gpui's binary size either.
- **Whether a `tray-icon` menu can update live while open**, under `tao` or `winit`, and whether tao#1324 reproduces outside its reporter's setup. This research found no second report of it from Tauri's tray users, which leaves open whether it depends on the reporter's setup.
- **Whether anything here builds on static musl.**
- **The transitive licence and advisory results** for each candidate, which need `cargo deny`. So does `gpui`'s use of `cocoa` and `objc`.
- **egui's macOS behaviour for a hidden window that wakes only on demand**, including whether #8737's fix covers macOS. The reports are from Windows.
- **Whether Slint's Rust API exposes the tray icon's position.** Its element docs show none, and the Rust API was not read for it.
- **The cocoa-dev and Stack Overflow advice** on live menus was read through a search summary. One direct fetch returned a 502. Apple's pages were read through their JSON data endpoints, which were queried at `developer.apple.com/tutorials/data/`.
- **ADR 0137**, which the map cites, is not in this worktree's `docs/adr`, so it was not read. The Kennel's glossary entry was.
- **Web view accessibility** for Tauri and Dioxus desktop was not checked against a primary source.

## Questions this raises for the owner

These are facts to settle in later tickets, not recommendations.

1. Does the `STYLE.md` dependency bar, static musl in particular, apply to the Kennel, given that it is part of no Product?
2. Is a native menu enough, or does the Kennel need a popover? Footprint and accessibility evidence differ sharply between the two.
3. Would a new allowlist entry for Slint's Royalty-free licence be acceptable, with its attribution duty?
