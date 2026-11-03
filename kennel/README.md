# The Kennel

The Kennel is Goodfetch's local tool for watching and controlling the Runs of a machine's Daemon, from the menu bar. It is published apart from the Products, under its own version, and is part of no Product ([ADR 0141](../docs/adr/0141-the-kennel-is-a-tray-tool-of-its-own-workspace-that-reaches-the-daemon-as-an-observing-person-surface-and-is-published-apart-from-the-products.md)).

## Run it

`just kennel`, from the repository's root.

- **On macOS** it builds the release executable, assembles `Kennel.app` (bundle identifier `dev.goodfetch.kennel`), signs it ad hoc, installs it by rename into `~/Applications` and opens it with `open`. A Kennel already running is quit first.
  - Always start the Kennel through LaunchServices: with `open`, the Finder or a login item. Run directly, the binary inside the `.app` is refused desktop notices.
  - The bundle identifier stays the same across builds, so a rebuild keeps the notice permission.
- **On Linux and Windows** it builds and runs the executable.

The icon shows one verdict for the Daemon, with no numbers: a ring while no Daemon is reachable, a filled disc when one is reached and idle. A left or a right click opens the menu, which shows the Daemon's state and Quit. With no Daemon running, the menu offers "Start Daemon".

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
- **Start Daemon.**
  - The action runs `daemon start` of an installed Product, with no shell.
  - It uses the absolute path that the Product's executable recorded in `<home>/installed-executables.json`. Foxhound is preferred, since it contains Collie.
  - The Kennel does not look a Product up in `PATH`: an app started by LaunchServices has a minimal `PATH`.
  - The Kennel never starts a Daemon by itself.

## Settings

The Kennel's own settings live in `<home>/kennel.toml`, never in the engine's configuration. It reads the file at launch and again whenever it changes. A missing file gives the defaults. A file that cannot be read also gives the defaults, and the menu says why. An unknown key counts as unreadable, so a typo shows.

| Key | Default | Off |
|---|---|---|
| `controls` | `true` | a read-only Kennel: the menu offers no action, "Start Daemon" included |
| `keep_daemon_alive` | `false` | the Kennel only observes: the Daemon stops at its idle timeout, and hands over to its major's pick, as if no Kennel were connected. On, a connected Kennel keeps the Daemon alive. A change reconnects, since `hello` carries it |

Later features each add a switch here.

## Layout and checks

| Path | What |
|---|---|
| `crates/goodfetch-kennel` | The modules that need no graphical toolkit: the Client, the State (the pure model), the settings, the Daemon start |
| `crates/goodfetch-kennel-app` | The executable, `goodfetch-kennel`: the Tray, with `tray-icon` and `muda` on `winit` |
| `scripts/` | `install.sh` (`just kennel`), `check.sh` (`just kennel-check`), `release.sh` (`just kennel-release`), `bundle.sh` (assembles `Kennel.app`), `install-kennel.sh` (the install script a release publishes), `measure.sh` |
| `Info.plist` | The `.app`'s property list, which `bundle.sh` stamps with the Kennel's version |

- **A workspace of its own.** It has its own `Cargo.lock` and toolchain pin. Its only path dependency is `goodfetch-protocol`.
- **Clippy.** Clippy reads the root `clippy.toml`, the nearest one in a parent folder, and the root's `[workspace.lints]` are copied into `Cargo.toml`.
- **`just kennel-check`.** It runs Clippy and the tests. The Client's tests run `goodfetch-scripted-engine`, which the root workspace builds, as a process. Run straight from this folder without the test tool manifest that `just kennel-check` writes, those tests say they are skipped and pass.
- **The release scripts' tests.** On macOS, `goodfetch-kennel-app`'s `tests/release.rs` runs `bundle.sh`, `release.sh` and the install script it writes on a stand-in executable, in a home of their own, with `open`, `pgrep` and `pkill` stood in.
- **Which changes run it.** The merge gate runs `just kennel-check` for a change to `kennel/` or `goodfetch-protocol`.
- **On every change.** `just lint` checks the formatting, the lockfile and the `cargo-deny` rules. `just audit` checks the advisories.

## Performance targets

These targets come from ADR 0141. They are measured on the owner's Mac, and no gate runs them. `scripts/measure.sh [seconds]` measures a running Kennel's memory (`phys_footprint`) and CPU, as #1915's throwaway build did.

The first figures are [#1915](https://github.com/goodfetch/goodfetch/issues/1915#issuecomment-6580328695)'s. They were measured on 2026-11-02, on the owner's Mac15,11 under macOS 27.2 (build 26B123), with the throwaway build.

| Target | Figure | Measured |
|---|---|---|
| Tray memory at most 30 MB `phys_footprint` | 13 MB idle (14 MB peak), 20 MB with the window open | 2026-11-02, Mac15,11, macOS 27.2 |
| Idle CPU below 0.1% over 60 s | 0.00% | 2026-11-02, Mac15,11, macOS 27.2 |
| CPU below 0.5% with a live Run | not measured yet: measured on the first build that reaches a Daemon with a live Run | |
| Window at most 100 MB once settled (no frame for 5 s), none when closed | 52 MB settled; 258 MB start-up peak | 2026-11-02, Mac15,11, macOS 27.2 |
| One executable at most 6 MB, stripped, with LTO | 4.77 MB (4,773,024 bytes), glow only | 2026-11-02, Mac15,11, macOS 27.2 |
| One executable, as above | 1.79 MB (1,789,744 bytes): this workspace's first build, the Tray alone, before the window | 2026-11-03, Mac15,11, macOS 27.2 |
| At most one redraw a second, none while hidden | about one a second | 2026-11-02, Mac15,11, macOS 27.2 |
