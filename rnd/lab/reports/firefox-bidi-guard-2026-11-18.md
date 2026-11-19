# The Guard on stock Firefox over WebDriver BiDi, November 18, 2026

**Purpose.** Issue 2460: before the engine drives stock Firefox directly over WebDriver BiDi (ADR 0019's note of 2026-11-11, issue 2461), prove that the Guard holds there in an isolated world the page cannot see. Collie's own check and disarm scripts, `crates/goodfetch-page/src/guard/check.js` and `disarm.js`, read unchanged, run in a WebDriver BiDi sandbox on the cells of the Guard's conformance matrix. Nothing is called in the page's main world and nothing polls the page.

**Experimental design.**
- **The matrix:** the three cross-engine campaigns of the October report ([`cross-engine-2026-10-09.md`](cross-engine-2026-10-09.md)), with the same variants, seeds, timeouts and three blocks:
  - actionability, 9 variants;
  - primitives, 10;
  - navigation, frames and files, 7.

  That makes 26 cells and 78 runs, each in a fresh worker and a fresh Firefox, each judged by the fixtures' private oracle.
- **The route:** `adapters/firefox-bidi/`. The worker starts the installed Firefox with its remote agent (`--remote-debugging-port`) and opens a WebDriver BiDi session. Every action goes through `guard.ts`, which mirrors the engine's `guard::attempt`:
  - One `script.callFunction` runs `check.js` in the sandbox `goodfetch-guard` with `awaitPromise`. It waits in the page at most a one-second slice, then checks and arms.
  - One `input.performActions` sends the gesture as a single burst of trusted input.
  - One `script.callFunction` runs `disarm.js`.
  - A gesture that landed elsewhere is replayed until the action's window passes.
  - Reads (`evaluate`) run in the same sandbox.
- **The log:** the session records each command before sending it, with its sandbox. The cells' metrics come from that log: commands, script calls, calls into the main world (a `script.*` command with no sandbox), and the commands of each guarded action.
- **The six CDP needs:** `adapters/firefox-bidi/probes.ts` tries each on the same Firefox, against a local server.
- **The control:** `adapters/firefox-bidi/chromium-control.ts` runs the same Guard scripts and the same loop on Chromium over raw CDP, the way the engine drives it, on the actionability cells:
  - `Page.createIsolatedWorld` and `Runtime.callFunctionOn`;
  - three `Input.dispatchMouseEvent`.

  A cell that fails on both browsers fails on the Guard, not on the route. Its seeds (4242 plus the block) differ from the campaign's, which the actionability fixtures do not read.
- **Systems:**
  - Stock Firefox 160.0.1, Mozilla's `linux-x86_64` release tarball (checked against its `SHA256SUMS`), headless, on Linux x86_64 (Ubuntu 26.04.1 LTS).
  - The control used Chrome headless shell 155.0.8059.84, launched by the lab's `launchBrowser` with its flags.
  - The host was loaded by other work (load average 13 to 40 across the runs), so no timing in this report is a measurement.

**Retained runs.** Each is the three campaigns, three blocks, 78 runs.
- `firefox-bidi-guard-2026-11-18T23-54-57-174Z-ad3874`, **the retained run**: on the code as committed, with its sources snapshotted and none changed during the run (`sourceChanges: []`).
- `firefox-bidi-guard-2026-11-18T23-34-28-268Z-463ace`: an earlier run, before the review's fixes (a bound on each action and on each command, a round trip after a key press, and an exception never taken for a gone document). The click loop was the same.
- `firefox-bidi-guard-2026-11-18T23-24-06-595Z-35307c`: an earlier run, made while the action loop moved from `page.ts` to `guard.ts` with the same behaviour, so some of its runs ran the earlier code.

The two earlier runs are kept as more samples of the same route.

**Reproduce.** From `rnd/lab/`:
- `FIREFOX_BIN=<firefox> node src/firefox-bidi-campaign.ts`
- `node adapters/firefox-bidi/probes.ts <firefox>`
- `node adapters/firefox-bidi/chromium-control.ts <chrome> 3`

## Results

| Matrix | Cells | Held, retained run (`ad3874`) | Held, `463ace` | Held, `35307c` |
| --- | --- | --- | --- | --- |
| Actionability | 9 | 24/27 (`moving`, blocks 0, 1 and 2) | 27/27 | 25/27 (`moving`, blocks 1 and 2) |
| Primitives | 10 | 30/30 | 29/30 (`moved`, block 1) | 30/30 |
| Navigation, frames and files | 7 | 21/21 | 21/21 | 21/21 |
| **All** | **26** | **75/78** | **77/78** | **76/78** |

Per cell, in the retained run, every cell held in all three blocks except `actionability/moving`, which held 0 of 3:
- actionability: `normal`, `overlay`, `delayed`, `hidden`, `moving`, `rerender`, `never-enabled`, `never-uncovered`, `ambiguous`;
- primitives: `hover`, `dblclick`, `contextmenu`, `drag`, `wheel`, `keyboard`, `contenteditable`, `unicode`, `retarget`;
- navigation: `spa`, `history`, `popup`, `frame`, `cross-frame`, `upload`, `download`.

- **No call into the page's main world.** The 78 runs of the retained run sent 498 commands, 328 of them script calls. Every script call targeted the sandbox: zero went to the main world. The same holds in every run of the two earlier ones that reports metrics (all but the failed `moved` run of `463ace`). No action polled the page from outside. Each check waits in the page and answers once, and the loop calls it again only after an answer.
- **The Guard never let a wrong action through.** No failure, in either run, is an action on the wrong element. Each one is either an action the oracle judged premature, or no action before the deadline.
- **`actionability/moving` fails on the Guard, not on the route.** The fixture moves the target by script every 16 ms until it is ready. `check.js` waits for stability only while a CSS animation or transition runs: ADR 0019's speed target counts on the arm's listener to catch a click that lands elsewhere. So a click can land on the target while it still moves, and the oracle rejects that click as premature ("wrong, premature or untrusted action effect"). The lab's own guard waited for two stable intervals, which is why the October report found no such failure. The control shows that the same scripts fail the same cell on Chromium over CDP. The control, run on the code as committed right after the retained run, shows that the same scripts fail the same cell on Chromium over CDP: `moving` held 0 of 3, with the same reason and the same oracle record (`phaseReady: false`), while the eight other actionability cells held 3 of 3 (24/27). Before the review's fixes, the control held `moving` 1 of 3 (25/27), and a ten-block run of that cell alone held 4 of 10. Over BiDi, the cell held 0 of 3 in the retained run, 3 of 3 in `463ace` and 1 of 3 in `35307c`. How often it fails moves with the host's load on both browsers, as a race between the click and the script's next move would.
- **`primitives/moved` failed once, in `463ace`, cause not found.** The target slips away on the pointer's first `pointerover`, uncovering a decoy. Normally the first gesture is cancelled on the decoy and the replay lands, in 2 attempts: that happened in the eight other runs of the cell across the three runs, and in 8 of 8 runs of the cell alone afterwards, seeds 7272 to 7279, with each check's answer logged. In block 1 of `463ace`, every replay was cancelled on the decoy until the 1.5 s deadline (`deadline_exceeded: #target: receives_events: <button id="decoy">`). The Guard refused the wrong action. The failure did not recur, and the host's load average was then between 24 and 40.

## Commands per click

Read from the session's own log, for the first guarded click of each cell. A click on a ready target is **3 commands**, each one awaited round trip:
1. `script.callFunction` runs `check.js`, which checks and arms;
2. `input.performActions` sends the move, press and release as one burst;
3. `script.callFunction` runs `disarm.js`.

Each replay adds the same 3.

| First click, retained run | Cells |
| --- | --- |
| 1 attempt, 3 commands | 30 |
| 2 attempts, 6 commands (`moved` 3, `moving` 1) | 4 |
| 3 attempts, 9 commands (`moving`) | 2 |

A key press adds one `script.callFunction` after its input, the engine's `settled` round trip.

The engine's click on CDP is 5 awaited commands: `Runtime.callFunctionOn` for the check, three `Input.dispatchMouseEvent`, and `Runtime.callFunctionOn` for the disarm. The control sends exactly these. BiDi's single `input.performActions` saves two round trips per attempt. The pointer's events are dispatched before the command answers, so the disarm that follows reads a settled outcome.

## The six things CDP gives that WebDriver BiDi has no command for

Firefox 160.0.1, headless, Linux x86_64; `probes.ts` run twice, before and after the review's fixes, with the same results. None of the six is fully available. Hiding works, but in a way the page can see.

| Need (the engine's use) | What was tried | Result |
| --- | --- | --- |
| Holding a new page, frame or worker until it is prepared (auto-attach with `waitForDebuggerOnStart`, then `Runtime.runIfWaitingForDebugger`) | `script.addPreloadScript` in the sandbox, which marks its window and reports on a channel (`script.message`); then a page with a frame and a dedicated worker | **Page: the sandbox's script, yes.** The preload ran in the new document at `readyState` `loading`, with no script run yet and the title not set: a script in the Guard's sandbox runs before the page's own, with no hold. What was tested is that script only. The engine's preparation also sends browser-side commands while the target is held (focus emulation, its identity, `Page.enable`, `Network.enable`); whether BiDi's equivalents reach a new page before it runs was not probed. **Frame: unavailable.** The frame's first document (same origin) reuses the Window of its initial `about:blank`. The preload ran on that `about:blank` only. In the loaded frame document the sandbox had no mark (`prepared: false`), while the frame's own script had already run. A cross-origin frame was not run. **Worker: unavailable.** A `dedicated-worker` realm was created and the worker ran its `fetch` with nothing held; preload scripts do not reach workers, and BiDi has no hold. |
| A streamed read of a paused response's body (`Fetch.takeResponseBodyAsStream`) | `network.addDataCollector`, then `network.addIntercept` at `responseStarted` on a 2,000,000-byte response, then `network.getData` while paused, and again after `network.continueResponse` | **Unavailable.** While the response was paused, `getData` gave no answer within 5 s. Once the response completed, it returned the whole 2,000,000 bytes. So only a completed body can be read, whole, and only after the response was released. |
| An inspector style sheet for live hiding (`CSS.createStyleSheet`, then `CSS.setStyleSheetText`) | From the sandbox, a constructed `CSSStyleSheet` pushed onto `document.adoptedStyleSheets` | **Works, but the page sees it.** The element went from `display: block` to `none` with no reload. The page's own script then counted `document.adoptedStyleSheets.length` 1 (it was 0); `document.styleSheets` stayed 0. |
| WebSocket frames (`Network.webSocketFrameSent` and `…Received`) | A subscription to the whole `network` module while the page opens a WebSocket, sends `ping` and receives the echo | **Unavailable.** The session got the handshake only (`beforeRequestSent`, `responseStarted`, `responseCompleted` for `/echo`), and no event for either frame, though the page received the echo. |
| Bypassing service workers for a fetch (`Network.setBypassServiceWorker`) | A page controlled by a service worker that answers `/probe`; a `fetch("/probe")` from the sandbox, then again under `network.setCacheBehavior` `bypass` | **Unavailable.** Both fetches were answered by the service worker (`from-service-worker`). BiDi's cache bypass does not bypass the worker. A profile-wide pref that turns service workers off for every page was not tried, since it is not a per-fetch bypass. |
| Focus emulation (`Emulation.setFocusEmulationEnabled`) | `document.hasFocus()` and `visibilityState` in two tabs, before and after `browsingContext.activate`; then again in a Firefox started with `focusmanager.testmode` | **Unavailable.** Only the active tab reported focus and `visible`; the other reported `hasFocus: false` and `hidden`. `activate` moved focus and visibility from one tab to the other. `focusmanager.testmode` changed nothing. |

## Systems not run

- **macOS 27:** not run. No macOS machine was at hand to this run: it ran on the gate host.
- **Windows:** not run.
- **Firefox:** other versions and channels were not run.

## Defect found along the way

- **The engine's Guard fails the matrix's `moving` cell on every route.** This is the Chromium result above: its control held 0 of 3, and 1 of 3 and 4 of 10 before. ADR 0019 asks the Guard to stay at 390 of 390 while waiting for stability only during an animation or a transition. That is not met for a target a script moves, the case the October report credited the guard with ("its own checks are not always enough on a moving target"). This is a defect of `check.js`, not of WebDriver BiDi.

## Verdict

The route does not hold as the first criterion reads it: the retained run held 75 of 78, because `actionability/moving` failed in all three blocks. It fails on the Guard's own `check.js`, which fails it on Chromium over CDP as well (0 of 3 in the control run beside it), not on WebDriver BiDi. Every other cell held in every block of the retained run, with no call into the main world and no polling. Across the two earlier runs, `moving` failed twice more and `primitives/moved` failed once, with no cause found and the Guard refusing the wrong action.
