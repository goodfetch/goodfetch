# Kennel on macOS: signing, token, login start, sandbox and notifications

**Date:** 2026-11-01. **Ticket:** [#1826](https://github.com/goodfetch/goodfetch/issues/1826), on the map [#1823](https://github.com/goodfetch/goodfetch/issues/1823).

**Scope:** a tray tool one person builds and runs on their own Mac, never distributed. These are facts and trade-offs. The decision is the owner's.

**Not tested.** This research ran on a Linux server, so nothing here was run on macOS. [What remains unverified](#what-remains-unverified) lists what needs a Mac.

**Versions.** Apple's security releases page lists [macOS Golden Gate 27.2, Tahoe 26.7.2 and Sequoia 15.8.2, all dated 26 October 2026](https://support.apple.com/en-us/100100). Each claim below names the versions its source names. Where a source names none, it is a statement of Apple's current documentation (copyright 2026), not a test on macOS 27.

**How far to trust each source:**

- **Apple doc:** Apple's documentation, technotes or support pages. Technotes on code signing warn that [their details "aren't considered API"](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements.md).
- **Apple DTS:** a post by Apple's Developer Technical Support on the forums. Informed, but not a contract.
- **Apple source:** Apple's published `Security` code. It may lag or differ from the shipping system.
- **Third party:** a field report or another project's documentation.
- **Inference:** my own reasoning from the cited facts, marked as such.

## Summary

| Question | Short answer | Fee needed? |
|---|---|---|
| Can a local build run? | Yes. Apple silicon needs a signature, and an ad hoc one is enough. A build that was never downloaded should not meet Gatekeeper (inference). | No |
| Is notarization needed? | No. It serves distribution, and it requires Developer ID. | Would need one |
| Does the menu-bar icon need a bundle? | No Apple document requires one, but this is not confirmed by a test. | No |
| Do notifications need a bundle? | `UNUserNotificationCenter` does: a bare executable crashes. | No (ad hoc is reported to be enough) |
| Keychain or `0600` file? | The Daemon already writes the token to a `0600` file, so a Keychain copy hides nothing until the Daemon stops writing that file. An ad hoc build also meets prompts after each rebuild. | No, except for the data protection keychain without its 7-day limit |
| Login start? | A launchd user agent needs no bundle and no identity. `SMAppService` needs a bundle and has reported problems under ad hoc signing. | No |
| App Sandbox? | Not needed, and it would block the Goodfetch home and the Daemon's socket. | No |
| Hardened runtime? | Not needed. It is required only for notarization, but it is available as a hardening option. | No |

## 1. Gatekeeper, notarization and signing

### What must be true to run

- **Apple silicon needs a signature (macOS 11 and later).** ["The operating system enforces that any executable must be signed before it's allowed to run... a simple ad-hoc signature is sufficient."](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes) The rule does not cover x86 binaries under Rosetta 2 or Intel Macs. (Apple doc)
- **The toolchain signs at link time.** Since Xcode 12 beta 4, [`clang` and `ld` apply an ad hoc signature automatically](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes). (Apple doc) A `cargo build` that links through them should inherit this (inference; no Rust primary source read).
- **Changing the binary after linking can break it.** The same notes say tools such as `strip` or `install_name_tool` [may require a manual `codesign`](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes). (Apple doc) A Rust forum thread reports [bundles that would not run until re-signed with `codesign --force --deep -s -`](https://users.rust-lang.org/t/how-to-create-macos-bundles-for-apple-silicon-they-always-seem-broken/132678), read only through a search summary. (Third party)
- **Never overwrite an installed executable in place.** [macOS caches signature information in the kernel, so writing over the same file can cause a `Code Signature Invalid` crash that a restart clears.](https://developer.apple.com/documentation/security/updating-mac-software.md) `cp` keeps the inode and is exposed. `ditto`, or a write to a temporary file followed by `rename`, is not. (Apple doc) This applies directly to a rebuild-and-install step.

### Gatekeeper

- **Gatekeeper follows quarantine.** ["Gatekeeper usually only kicks in the app is quarantined."](https://developer.apple.com/forums/thread/666452) [Safari, AirDrop and similar user-level routes quarantine what they fetch](https://developer.apple.com/forums/thread/777189), and [curl and scp do not](https://developer.apple.com/forums/thread/666452). (Apple DTS, 2020 and 2025)
- **A local build is therefore not expected to meet it (inference).** Nothing downloads it, so nothing quarantines it. Apple's documents do not state this case directly.
- **If it is quarantined, an ad hoc build fails the check.** Ad hoc signatures ["cannot pass through Gatekeeper"](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes). (Apple doc) That would apply to a bundle AirDropped from another Mac, for example.
- **The override moved in macOS 15.** [Control-click no longer overrides Gatekeeper; the user goes to System Settings > Privacy & Security.](https://developer.apple.com/news/?id=saqachfa) [Apple's support page describes the Open Anyway button there.](https://support.apple.com/en-us/102445) (Apple doc)
- **macOS 27 refuses quarantined launchd files.** [`launchd` no longer loads property lists that carry the quarantine attribute.](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes.md) (Apple doc) A plist written locally is unaffected (inference); a downloaded one is not.

### Notarization

- It [requires a Developer ID signature, and Apple names ad hoc among the signatures not to use](https://developer.apple.com/tutorials/data/documentation/security/notarizing-macos-software-before-distribution.json).
- It also [requires the hardened runtime](https://developer.apple.com/tutorials/data/documentation/security/hardened-runtime.json). (Apple doc)
- Its purpose is [telling Gatekeeper and users that Apple checked Developer ID software you distribute](https://developer.apple.com/tutorials/data/documentation/security/notarizing-macos-software-before-distribution.json). A tool that is never distributed gets nothing from it.

### Signature kinds and what each gives

| Signature | Cost | Identity across rebuilds | Team ID |
|---|---|---|---|
| Ad hoc (`codesign -s -`, or the linker's) | Free | None: [its designated requirement (DR) "is tied to that specific version of the code"](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements.md) | No |
| Self-signed certificate | Free | [Lets the system verify that two versions came from the same source](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html). DTS adds that a non-Apple certificate is ["not really an options these days"](https://developer.apple.com/forums/thread/809954), and TN3161 says it has [no practical use because Gatekeeper blocks it](https://developer.apple.com/documentation/technotes/tn3161-inside-code-signing-certificates.md) | No |
| Personal Team (free Apple Account, Xcode) | Free | [Apple-issued, stable, certificate valid for a year](https://developer.apple.com/forums/thread/809954) (Apple DTS). [Its provisioning profiles last 7 days](https://developer.apple.com/support/compare-memberships/) | Yes |
| Apple Development or Developer ID (paid Program) | **Annual fee, declined under ADR 0076** | Stable | Yes |

DTS [tested that macOS 15.6 runs a Personal Team app with an expired certificate when no restricted entitlement is involved](https://developer.apple.com/forums/thread/809954). The same app with a restricted entitlement was blocked.

### Bare executable and bundle

The minimum bundle is [`Contents/Info.plist` and the executable in `Contents/MacOS/`](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle.md). (Apple doc)

| Need | Bare executable | `.app` bundle |
|---|---|---|
| Run on Apple silicon | Yes, ad hoc | Yes, ad hoc |
| Menu-bar icon | Apple's [`NSStatusItem` page](https://developer.apple.com/documentation/appkit/nsstatusitem.md) and the Rust [`tray-icon` documentation](https://docs.rs/tray-icon/latest/tray_icon/) state no bundle requirement. `tray-icon` does require the [event loop and icon on the main thread](https://docs.rs/tray-icon/latest/tray_icon/) | Yes |
| No Dock icon | [`.accessory` activation policy, which Apple says corresponds to `LSUIElement` = 1](https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy-swift.enum/accessory.md), set in code | [`LSUIElement`](https://developer.apple.com/documentation/bundleresources/information-property-list/lsuielement.md) in `Info.plist`, or the same call |
| Bundle identifier | No, unless an [`Info.plist` section is embedded in the binary](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html), which Apple describes for code signing purposes | Yes |
| `UNUserNotificationCenter` | **No: it crashes** (section 5) | Yes, signed |
| `SMAppService` | No: the service must [live inside an app bundle](https://developer.apple.com/tutorials/data/documentation/servicemanagement/smappservice.json) | Yes |
| launchd user agent | Yes (section 3) | Yes |
| Data protection keychain | No: [a standalone executable has nowhere to embed the profile](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles.md) | Only with a profile |

Privacy grants share the identity problem. [With no stable DR, macOS repeats its prompt after each change to the code.](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements.md) (Apple doc) DTS says [most reports of repeated prompts come from unsigned or ad hoc code](https://developer.apple.com/forums/thread/125794). This reaches the Kennel only if its files sit in [Desktop, Documents or Downloads](https://support.apple.com/guide/mac-help/control-access-to-files-and-folders-on-mac-mchld5a35146/mac). The default Goodfetch home on macOS is under `~/Library/Application Support` (`dirs::data_dir` in `crates/goodfetch-collie/src/executable.rs`). A `GOODFETCH_HOME` in one of those three folders would trigger it (inference).

## 2. Keychain or `0600` file for the token

### Project facts

- The Daemon's runtime file is [`daemon.json`, mode `0600`, holding the endpoint, token, version and PID](../../docs/adr/0024-engine-binary-with-thin-json-rpc-clients.md).
- Under socket activation and a foreground `daemon run`, the token is [`daemon.token`, a persistent `0600` file](../../docs/adr/0031-console-is-an-ordinary-client-over-a-run-index.md).
- `daemon rotate-token` [replaces that token and stops the running Daemon](../../docs/adr/0031-console-is-an-ordinary-client-over-a-run-index.md).
- ADR 0137 says a local tool [reaches the Daemon as the CLI and the SDKs do](https://github.com/goodfetch/goodfetch/blob/main/docs/adr/0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md). That ADR is on `origin/main` and not yet in this worktree.

### What each protects against

| Threat | `0600` file | Keychain copy, while the file still exists | Keychain only (Daemon no longer writes the file) |
|---|---|---|---|
| Another user account on the Mac | Blocked by [BSD permissions](https://developer.apple.com/forums/thread/678819) | Same | Blocked |
| A program running as the owner | **Not blocked** (inference: BSD permissions check the user, and no mandatory control covers this folder by default) | **Not blocked**: it reads the file | Depends on the item's access control and the signing identity (below) |
| Reading the disk while logged out or from a backup | Depends on FileVault and on whether backups include the folder (inference) | The file is still there | [Items are encrypted](https://support.apple.com/guide/security/keychain-data-protection-secb0694df1a/web) |
| Root | Not blocked | Not blocked | Not assessed |
| A staleness failure | None: the file is the source of truth | **New risk**: a per-launch token changes at each Daemon start and rotation changes the persistent one, so the copy can be wrong (inference) | None, but see the cost below |

The main fact: **a Keychain copy adds no secrecy while the Daemon writes the same token to a `0600` file** (inference from ADRs 0024 and 0031). Real protection requires the Daemon to stop writing it. That is an engine change for every Client, and it would need a keychain that works for a Daemon run under launchd and for signed Product builds. The Kennel also has to read `daemon.json` for the endpoint regardless.

### Which keychain

[macOS has two implementations, and `SecItem` defaults to the file-based one.](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains.md) (Apple doc, revised 2026-09-24)

| | File-based keychain | Data protection keychain |
|---|---|---|
| Access control | [Access control lists](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains.md) plus a partition list | [Access groups from entitlements](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains.md) |
| Requirement | None to call it | [A restricted entitlement, authorized by a provisioning profile, embedded in an app-like bundle](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains.md) |
| Status | ["On the road to deprecation"](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains.md); [significant changes began in macOS 26.4](https://developer.apple.com/forums/thread/844097) | Apple's default recommendation |
| Reachable without a fee? | Yes | Personal Team only, whose [profile lasts 7 days](https://developer.apple.com/support/compare-memberships/) and [whose expiry blocks launch](https://developer.apple.com/forums/thread/809954). Otherwise it needs the paid Program |

### Prompts after a rebuild

- **The mechanism has two layers in the file-based keychain.** The default ACL [identifies the app by its DR](https://developer.apple.com/forums/thread/69841). (Apple DTS) The partition check [compares the caller's partition identifier with the item's list and, on a mismatch during a read, asks the user and adds the identifier if approved](https://github.com/apple-oss-distributions/Security/blob/main/securityd/src/acls.cpp). (Apple source)
- **The partition identifier depends on the signer.** [Apple code gets `apple:`, `security(1)` gets `apple-tool:`, Apple-issued developer certificates get `teamid:<OU>`, and any other signed code gets `cdhash:<hash>`](https://github.com/apple-oss-distributions/Security/blob/main/securityd/src/clientid.cpp). (Apple source; DTS confirms [the Team ID is "critical for the keychain partition mechanism"](https://developer.apple.com/forums/thread/809954))

| Build signed with | Expected behaviour after each rebuild | Basis |
|---|---|---|
| Nothing | Not available on Apple silicon: [unsigned native code does not run](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes) | Apple doc |
| Ad hoc | **A prompt each time.** The cdhash changes, so both layers miss. "Always Allow" holds only until the next rebuild | [DTS](https://developer.apple.com/forums/thread/809954) names ad hoc as a cause of such trouble; [a project reports a prompt per item after each rebuild](https://github.com/4LAU/codex-profile-switcher/releases/tag/v0.4.1) (third party); per-build behaviour from [source](https://github.com/apple-oss-distributions/Security/blob/main/securityd/src/clientid.cpp) |
| Self-signed | **Sources conflict.** The stable DR should satisfy the ACL, but [the source gives such code a `cdhash:` partition](https://github.com/apple-oss-distributions/Security/blob/main/securityd/src/clientid.cpp), which predicts a partition prompt each time. [A project using a local self-signed identity reports no further clicks after updates](https://github.com/GokhanTurk/gukab/wiki/Security) | Needs a test on a Mac |
| Personal Team | **Probably none**, once the item exists: stable identity and a Team ID. DTS: ["I think that'll all work"](https://developer.apple.com/forums/thread/809954), a prediction | Not confirmed by anyone in the sources read |
| Paid Apple identity | None expected | Same mechanism; declined under ADR 0076 |

**Reading through `/usr/bin/security`.** That tool has [its own `apple-tool:` partition](https://github.com/apple-oss-distributions/Security/blob/main/securityd/src/clientid.cpp), which does not change when the Kennel is rebuilt. Once it is allowed on the item, any program of the same user can run it to read the item, so the protection falls towards that of the file (inference).

**Other projects' workaround.** One project [keeps unsigned development builds out of the Keychain and uses a file-based vault](https://github.com/4LAU/codex-profile-switcher/releases/tag/v0.4.1). (Third party)

**A live bug to note.** On macOS 26.6.x, [`securityd` crashes and repeated login-keychain prompts were reported with `SecKeychain` code; DTS confirmed the crash as a bug (FB24637565) and the reporter fixed the prompts by leaving that API](https://developer.apple.com/forums/thread/844097). This argues for `SecItem` if a keychain is used at all.

## 3. Starting at login

| | launchd user agent | `SMAppService` | Login Items (user-added) |
|---|---|---|---|
| macOS | [All current versions](https://keith.github.io/xcode-man-pages/launchd.plist.5.html) | [13 and later](https://developer.apple.com/tutorials/data/documentation/servicemanagement/smappservice.json) | [Current; System Settings > General > Login Items & Extensions](https://support.apple.com/guide/mac-help/open-items-automatically-when-you-log-in-mh15189/mac) |
| Needs a bundle | No | **Yes** | No Apple statement for a bare executable; [the list accepts apps, documents, folders and other items](https://support.apple.com/guide/mac-help/open-items-automatically-when-you-log-in-mh15189/mac) |
| Needs an identity | None stated | [DTS: helper and app should share an Apple-issued identity; ad hoc "will see problems"](https://developer.apple.com/forums/thread/799910) | None stated |
| Set up by | Writing a plist to [`~/Library/LaunchAgents`](https://keith.github.io/xcode-man-pages/launchd.plist.5.html), then [`launchctl bootstrap gui/<uid>`](https://keith.github.io/xcode-man-pages/launchctl.1.html) | Calling [`register()`](https://developer.apple.com/documentation/servicemanagement/smappservice/register().md) from the app, through Objective-C bindings in Rust | The owner, by hand |
| Restart on crash | [`KeepAlive`](https://keith.github.io/xcode-man-pages/launchd.plist.5.html), optional | [A `LoginItem` helper restarts after a crash or non-zero exit](https://developer.apple.com/documentation/servicemanagement/smappservice/register().md); [`mainApp`](https://developer.apple.com/documentation/servicemanagement/smappservice/mainapp.md) only launches at login | No |
| In Rust from a bare `cargo` build | Yes, writes a text file | No, needs a bundle step | Not verified |

**Launchd user agent details:**

- Apple's own article says [a `LaunchAgent` can provide "menu bar extras"](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos.md).
- [An agent with no `LimitLoadToSessionType` defaults to `Aqua`, which "has access to all GUI services; much like a login item."](https://developer.apple.com/library/archive/technotes/tn2083/_index.html) That technote is an old archive document.
- It appears in Settings: [an unsigned executable no app claims shows under its executable name](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos.md). The owner can switch it off there.
- [`AssociatedBundleIdentifiers` needs a Team ID matching the app's](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos.md), so an ad hoc build cannot use it (inference).
- The project already installs one for the Daemon: [`goodfetch daemon install` registers a launch agent](../../docs/adr/0031-console-is-an-ordinary-client-over-a-run-index.md).

**`SMAppService` under rebuilds:**

- [`.requiresApproval` means the user must act in System Settings.](https://developer.apple.com/documentation/servicemanagement/smappservice/status-swift.enum/requiresapproval.md) (Apple doc)
- A developer with no local signing reports that [each new build shows `.notFound` and adds another login item, with over 100 accumulated](https://www.github.com/sindresorhus/LaunchAtLogin/issues/100). (Third party, December 2023, macOS version not stated, closed unresolved)
- Another reports [`Operation not permitted` from `register()`, with "just a free developer profile" on macOS 13.1](https://developer.apple.com/forums/thread/799910). That thread does not isolate the cause.

**A related prompt in macOS 26.** [On macOS 26 and later, background tasks an app starts that stay active after it quits lead to a prompt to allow or not allow them.](https://support.apple.com/guide/deployment/manage-login-items-background-tasks-mac-depdca572563/web) (Apple doc) Whether a Daemon started by the Kennel counts is not verified.

The map says the Kennel is [started on request](https://github.com/goodfetch/goodfetch/issues/1823), so login start would be an optional setting with a free off state.

## 4. App Sandbox and hardened runtime

### App Sandbox

- **Not needed.** Apple requires it [to distribute through the Mac App Store](https://developer.apple.com/tutorials/data/documentation/security/app-sandbox.json). This tool is not.
- **It would deny the Goodfetch home.** The sandbox [denies all file system locations except an allowlist and the app's containers](https://developer.apple.com/forums/thread/678819). (Apple DTS) `~/Library/Application Support/goodfetch` is neither, so `daemon.json` and `daemon.token` would need an exception (inference).
- **It would block the Daemon's socket.** Unix domain sockets ["are blocked by the sandbox as part of its general policy of blocking unmediated IPC between code from different teams"](https://developer.apple.com/forums/thread/788364), except in an app group container. (Apple DTS, 2025) The `/tmp/goodfetch-daemon-<digest>/` fallback for long paths would be blocked likewise (inference).
- **Whether an exception helps is contradictory in Apple's own thread.** The first reply says temporary exception entitlements ["only work for files and directories; they don't work for Unix domain sockets"](https://developer.apple.com/forums/thread/788364). A later reply in the same thread says a temporary exception ["applies to both files, directories, and sockets"](https://developer.apple.com/forums/thread/788364), while an open-panel grant does not. Only a test settles this.
- **The clean routes change other things.** An app group container [means the server creating its socket there](https://developer.apple.com/forums/thread/788364), which is an engine change. A [non-sandboxed XPC service](https://developer.apple.com/forums/thread/776609) carries the blocked work outside the sandbox, which adds a component and takes back much of the benefit (inference). [`sun_path` is 104 bytes](https://developer.apple.com/forums/thread/756756), so container paths are tight.
- **What it would give.** [Contains damage if the app is compromised](https://developer.apple.com/tutorials/data/documentation/security/app-sandbox.json), and [enables app container protection](https://developer.apple.com/forums/thread/776609). A token-holding tool gains something, but only by giving up the access it exists for.
- **Signing.** Sandbox entitlements are [unrestricted, so no profile is needed](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles.md). Behaviour under an ad hoc signature is not verified.

### Hardened runtime

- **Not needed to run.** It is [required to upload for notarization](https://developer.apple.com/tutorials/data/documentation/security/hardened-runtime.json), and this tool does not notarize. ADR 0039 requires it for Product binaries, and that does not apply here (ADR 0137).
- **What it does.** It [protects against code injection, library hijacking and memory tampering, and "doesn't affect the operation of most apps"](https://developer.apple.com/tutorials/data/documentation/security/hardened-runtime.json). `codesign -o runtime` [turns it on from macOS 10.14](https://keith.github.io/xcode-man-pages/codesign.1.html).
- **Costs.** It disallows [JIT without an entitlement](https://developer.apple.com/tutorials/data/documentation/security/hardened-runtime.json), and the man page lists [debugging restrictions](https://keith.github.io/xcode-man-pages/codesign.1.html).
- **With ad hoc.** Its entitlements are [unrestricted](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles.md), but the man page warns of [significant restrictions on ad hoc code](https://keith.github.io/xcode-man-pages/codesign.1.html), and the combination is not verified here.

## 5. User notifications from Rust

| | `UNUserNotificationCenter` | `NSUserNotification` |
|---|---|---|
| Status | [Current, macOS 10.14 and later](https://developer.apple.com/tutorials/data/documentation/usernotifications/unusernotificationcenter.json) | [Deprecated since macOS 11: "should be replaced with UserNotifications.frameworks API"](https://developer.apple.com/tutorials/data/documentation/foundation/nsusernotification.json) |
| Bundle | **Required in practice.** From a bare executable, `current()` [raises `bundleProxyForCurrentProcess is nil`](https://developer.apple.com/forums/thread/724249), also [from an unbundled launch agent](https://developer.apple.com/forums/thread/679326). (Community reports; Apple's pages do not state it) | [Community report: "The main bundle doesn't seems to be on the critical path"](https://developer.apple.com/forums/thread/679326) (2022) |
| Signature | [Must be bundled and signed, ad hoc is sufficient](https://docs.rs/mac-usernotifications) ([`mac-usernotifications`](https://docs.rs/mac-usernotifications), third party); [a Bitcoin Core developer confirms `codesign -s -` then `open` works](https://mirror.b10c.me/bitcoin-core-gui/112/) (2024) | No requirement found |
| Permission | [One system prompt the first time, then remembered](https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications.md). [Provisional authorization sends quietly with no prompt](https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications.md) | None found |
| Failure | [`UNErrorDomain` Code=1, "Notifications are not allowed for this application"](https://mirror.b10c.me/bitcoin-core-gui/112/), reported for unbundled or unsigned builds | Not verified |
| Rust | [`mac-usernotifications` 0.3.1, designed for `notify-rust`](https://docs.rs/mac-usernotifications), with a `check_bundle` guard | [`mac-notification-sys` 0.6.15](https://docs.rs/mac-notification-sys/latest/mac_notification_sys/), which [calls `NSUserNotificationCenter`](https://github.com/h4llow3En/mac-notification-sys/blob/master/objc/notify.m) |

**Further points:**

- `mac-notification-sys` [substitutes a bundle identifier and falls back to `com.apple.Finder`](https://docs.rs/mac-notification-sys/latest/mac_notification_sys/), so a notification may be attributed to another app (inference on how it displays).
- Apple documents no continued support for the deprecated API; whether it still delivers on macOS 27 is not verified. The [macOS 27 notes](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes.md) mention neither API.
- Without a bundle, `par-term` [falls back to `osascript`](https://docs.rs/crate/par-term/0.40.0/source/src/platform/notify_macos.rs). (Third party)
- A click [launches the app that sent the notification and calls its delegate](https://developer.apple.com/forums/thread/679326) (community report). This requires a bundle that macOS can launch (inference), and the app [must assign a delegate before interacting](https://developer.apple.com/tutorials/data/documentation/usernotifications/unusernotificationcenter.json). (Apple doc)

## What needs a fee

ADR 0076 declines the Apple Developer Program for this tool unless already paid for elsewhere. Apple's [membership page](https://developer.apple.com/support/compare-memberships/) did not show its price in the text read.

- **Needs the Program:** Developer ID and so notarization. Membership is implied by the [certificate access it lists](https://developer.apple.com/support/compare-memberships/), and the page's check marks did not survive extraction.
- **Needs the Program in practice:** the data protection keychain without a 7-day profile.
- **Free:** ad hoc signing, a self-signed certificate, a Personal Team, a launchd agent, Login Items, a bundle, `UNUserNotificationCenter` under ad hoc (third-party reports), and `0600` file reads.

## Combinations open with no fee

These are facts collected from above, not recommendations.

| Combination | Gives | Gives up |
|---|---|---|
| Bare executable, ad hoc | Simplest build; icon (unconfirmed); launchd agent; `0600` read | `UNUserNotificationCenter` |
| `.app`, ad hoc | All of the above, plus `UNUserNotificationCenter` and `LSUIElement` | A bundle step in the install; identity across rebuilds |
| `.app`, Personal Team | Above, plus a stable identity for privacy grants, the file-based keychain and perhaps `SMAppService` | Needs Xcode and an Apple Account, and a yearly certificate |

## What remains unverified

None of this could be tried without a Mac. Each point names the check, on the current macOS release (27.2).

1. **Tray icon from a bare binary.** Does an ad hoc `tray-icon` binary show its icon, from a terminal and from a launchd agent? Does `.accessory` set in code flash a Dock icon?
2. **`UNUserNotificationCenter` under ad hoc.** Do authorization and delivery work on macOS 27 from `open Kennel.app`, and when the bundled executable is started directly from a terminal? Does the grant survive a rebuild, or does each build prompt again?
3. **`NSUserNotification`.** Does it still deliver on macOS 27 from a bare binary, and who does the banner name?
4. **Keychain prompts per rebuild,** for ad hoc, self-signed and Personal Team, especially the self-signed conflict. The published `Security` source may also not match macOS 27, since [macOS 26.4 began changing the file-based keychain](https://developer.apple.com/forums/thread/844097).
5. **`SMAppService`.** Does `mainApp.register()` work under ad hoc and under a Personal Team, and do rebuilds add entries? `sfltool dumpbtm` [prints the state](https://support.apple.com/guide/deployment/manage-login-items-background-tasks-mac-depdca572563/web).
6. **launchd agent across rebuilds.** Does replacing the binary at the same path keep the Login Items switch? Does macOS repeat its "added" notice for an ad hoc build whose hash changed? How does it show on macOS 27?
7. **User-added Login Item for a bare executable.** Does it start directly, or open a Terminal window?
8. **Sandbox and sockets.** Does `temporary-exception.files.home-relative-path` reach a socket, given the contradiction in [thread 788364](https://developer.apple.com/forums/thread/788364)? This matters only if the sandbox is wanted.
9. **Hardened runtime with ad hoc.** Does a static Rust tray binary run under `codesign -s - -o runtime`?
10. **The macOS 26 background-task prompt.** Does a Daemon started by the Kennel trigger it?
11. **Post-link signing under Cargo.** After `cargo build --release`, including its `strip` step, does `codesign -v` report a valid signature? No Rust primary source was read.
12. **Intel Macs.** macOS 27's [Apple-silicon-only requirement](https://www.macworld.com/article/672681/list-of-all-macos-versions-including-the-latest-macos.html) comes from secondary sources only, read through a search summary, so Intel behaviour is not excluded here.

## Limits of this research

- Several forum dates appear only as month and year.
- `launchd.plist`, `launchctl` and `codesign` man pages were read through [a mirror of Xcode's man pages](https://keith.github.io/xcode-man-pages/), not on a Mac.
- [Thread 724249](https://developer.apple.com/forums/thread/724249) and [thread 679326](https://developer.apple.com/forums/thread/679326) on bundles and notifications carry no Apple reply. The bundle requirement rests on community reports, consistent with third-party crates, and Apple's reference pages do not state it.
- The Rust crates were not assessed for maintenance. That belongs to the toolkit research on this map.
