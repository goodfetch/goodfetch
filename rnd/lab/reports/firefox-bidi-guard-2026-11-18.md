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

  A cell that fails on both browsers fails on the Guard, not on the route.
- **Systems:**
  - Stock Firefox 160.0.1, Mozilla's `linux-x86_64` release tarball (checked against its `SHA256SUMS`), headless, on Linux x86_64 (Ubuntu 26.04.1 LTS).
  - The control used Chrome headless shell 155.0.8059.84.
  - The host was loaded (load average 15 to 24) by other work, so no timing in this report is a measurement.

**Retained runs.**
- `firefox-bidi-guard-2026-11-18T23-34-28-268Z-463ace`: the three campaigns, three blocks, on the code as committed.
- `firefox-bidi-guard-2026-11-18T23-24-06-595Z-35307c`: an earlier full run, made while the action loop moved from `page.ts` to `guard.ts` with the same behaviour, so some of its cells ran the earlier code. It is kept as a second sample of the same route.

**Reproduce.** From `rnd/lab/`:
- `FIREFOX_BIN=<firefox> node src/firefox-bidi-campaign.ts`
- `node adapters/firefox-bidi/probes.ts <firefox>`
- `node adapters/firefox-bidi/chromium-control.ts <chrome> 3`

## Results

| Matrix | Cells | Held, retained run | Held, earlier run |
| --- | --- | --- | --- |
| Actionability | 9 | 27/27 | 25/27 (`moving`, blocks 1 and 2) |
| Primitives | 10 | 29/30 (`moved`, block 1) | 30/30 |
| Navigation, frames and files | 7 | 21/21 | 21/21 |
| **All** | **26** | **77/78** | **76/78** |

Per cell, in the retained run, every cell held in all three blocks except `primitives/moved`, which held 2 of 3:
- actionability: `normal`, `overlay`, `delayed`, `hidden`, `moving`, `rerender`, `never-enabled`, `never-uncovered`, `ambiguous`;
- primitives: `hover`, `dblclick`, `contextmenu`, `drag`, `wheel`, `keyboard`, `contenteditable`, `unicode`, `retarget`;
- navigation: `spa`, `history`, `popup`, `frame`, `cross-frame`, `upload`, `download`.

- **No call into the page's main world.** The 77 cells that report metrics sent 490 commands, 321 of them script calls. Every script call targeted the sandbox: zero went to the main world. No action polled the page from outside. Each check waits in the page and answers once, and the loop calls it again only after an answer.
- **The Guard never let a wrong action through.** No failure, in either run, is an action on the wrong element. Each one is either an action the oracle judged premature, or no action before the deadline.
- **`actionability/moving` fails on the Guard, not on the route.** The fixture moves the target by script every 16 ms until it is ready. `check.js` waits for stability only while a CSS animation or transition runs: ADR 0019's speed target counts on the arm's listener to catch a click that lands elsewhere. So a click can land on the target while it still moves, and the oracle rejects that click as premature ("wrong, premature or untrusted action effect"). The lab's own guard waited for two stable intervals, which is why the October report found no such failure. The control shows that the same scripts fail the same cell on Chromium over CDP. In the control's three blocks on Chrome headless shell 155, `moving` held 1 of 3, with the same reason and the same oracle record (`phaseReady: false`), while the eight other actionability cells held 3 of 3 (25/27). An earlier ten-block run of that cell alone held 4 of 10. Over BiDi, the cell held 3 of 3 in the retained run and 1 of 3 in the earlier one.
- **`primitives/moved` failed once, cause not found.** The target slips away on the pointer's first `pointerover`, uncovering a decoy. Normally the first gesture is cancelled on the decoy and the replay lands, in 2 attempts: that happened in the five other runs of the cell, and in 8 of 8 runs of the cell alone afterwards, seeds 7272 to 7279, with each check's answer logged. In block 1 of the retained run, every replay was cancelled on the decoy until the 1.5 s deadline (`deadline_exceeded: #target: receives_events: <button id="decoy">`). The Guard refused the wrong action. The failure did not recur, and the host's load average was between 24 and 40.

## Commands per click

Read from the session's own log, for the first guarded click of each cell. A click on a ready target is **3 commands**, each one awaited round trip:
1. `script.callFunction` runs `check.js`, which checks and arms;
2. `input.performActions` sends the move, press and release as one burst;
3. `script.callFunction` runs `disarm.js`.

Each replay adds the same 3.

| First click, retained run | Cells |
| --- | --- |
| 1 attempt, 3 commands | 30 |
| 2 attempts, 6 commands (`moving`, `moved`) | 4 |
| 6 attempts, 18 commands (`moving`, block 1) | 1 |

The engine's click on CDP is 5 awaited commands: `Runtime.callFunctionOn` for the check, three `Input.dispatchMouseEvent`, and `Runtime.callFunctionOn` for the disarm. The control sends exactly these. BiDi's single `input.performActions` saves two round trips per attempt. The pointer's events are dispatched before the command answers, so the disarm that follows reads a settled outcome.

## The six things CDP gives that WebDriver BiDi has no command for

Firefox 160.0.1, headless, Linux x86_64; one run of `probes.ts`. None of the six is fully available. Hiding works, but in a way the page can see.

| Need (the engine's use) | What was tried | Result |
| --- | --- | --- |
| Holding a new page, frame or worker until it is prepared (auto-attach with `waitForDebuggerOnStart`, then `Runtime.runIfWaitingForDebugger`) | `script.addPreloadScript` in the sandbox, which marks its window and reports on a channel (`script.message`); then a page with a frame and a dedicated worker | **Page: works.** The preload ran in the new document at `readyState` `loading`, with no script run yet and the title not set: it prepares before the page runs, with no hold. **Frame: unavailable.** The frame's first document (same origin) reuses the Window of its initial `about:blank`. The preload ran on that `about:blank` only. In the loaded frame document the sandbox had no mark (`prepared: false`), while the frame's own script had already run. A cross-origin frame was not run. **Worker: unavailable.** A `dedicated-worker` realm was created and the worker ran its `fetch` with nothing held; preload scripts do not reach workers, and BiDi has no hold. |
| A streamed read of a paused response's body (`Fetch.takeResponseBodyAsStream`) | `network.addDataCollector`, then `network.addIntercept` at `responseStarted` on a 2,000,000-byte response, then `network.getData` while paused, and again after `network.continueResponse` | **Unavailable.** While the response was paused, `getData` gave no answer within 5 s. Once the response completed, it returned the whole 2,000,000 bytes. So only a completed body can be read, whole, and only after the response was released. |
| An inspector style sheet for live hiding (`CSS.createStyleSheet`, then `CSS.setStyleSheetText`) | From the sandbox, a constructed `CSSStyleSheet` pushed onto `document.adoptedStyleSheets` | **Works, but the page sees it.** The element went from `display: block` to `none` with no reload. The page's own script then counted `document.adoptedStyleSheets.length` 1 (it was 0); `document.styleSheets` stayed 0. |
| WebSocket frames (`Network.webSocketFrameSent` and `…Received`) | Every `network` event while the page opens a WebSocket, sends `ping` and receives the echo | **Unavailable.** The session got the handshake only (`beforeRequestSent`, `responseStarted`, `responseCompleted` for `/echo`), and no event for either frame, though the page received the echo. |
| Bypassing service workers for a fetch (`Network.setBypassServiceWorker`) | A page controlled by a service worker that answers `/probe`; a `fetch("/probe")` from the sandbox, then again under `network.setCacheBehavior` `bypass` | **Unavailable.** Both fetches were answered by the service worker (`from-service-worker`). BiDi's cache bypass does not bypass the worker. A profile-wide pref that turns service workers off for every page was not tried, since it is not a per-fetch bypass. |
| Focus emulation (`Emulation.setFocusEmulationEnabled`) | `document.hasFocus()` and `visibilityState` in two tabs, before and after `browsingContext.activate`; then again in a Firefox started with `focusmanager.testmode` | **Unavailable.** Only the active tab reported focus and `visible`; the other reported `hasFocus: false` and `hidden`. `activate` moved focus and visibility from one tab to the other. `focusmanager.testmode` changed nothing. |

## Systems not run

- **macOS 27:** not run. No macOS machine was at hand to this run: it ran on the gate host.
- **Windows:** not run.
- **Firefox:** other versions and channels were not run.

## Defect found along the way

- **The engine's Guard fails the matrix's `moving` cell on every route.** This is the Chromium result above: its control held 1 of 3 and 4 of 10. ADR 0019 asks the Guard to stay at 390 of 390 while waiting for stability only during an animation or a transition. That is not met for a target a script moves, the case the October report credited the guard with ("its own checks are not always enough on a moving target"). This is a defect of `check.js`, not of WebDriver BiDi.

## Verdict

The route does not hold as the first criterion reads it: 77 of 78 runs held (76 of 78 in the earlier run), because `actionability/moving` fails intermittently on the Guard's own `check.js`, which fails it on Chromium over CDP as well, and `primitives/moved` failed once with no cause found; every other cell held in every block, with no call into the main world and no polling.
