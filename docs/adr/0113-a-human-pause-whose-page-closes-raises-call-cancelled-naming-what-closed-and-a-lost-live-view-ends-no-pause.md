# A Human pause whose page closes raises `call_cancelled` naming what closed, and a lost Live view ends no pause

ADR 0092 says how `page.pause()` ends: it returns when its condition holds or a person says "Done", it raises `human_pause_declined` on a person's "Decline", it raises `deadline_exceeded` past `timeout.human_pause`, and the program ends it by cancelling the call. It does not say what happens when the page goes away first. With `browser.headless: false` the person acts in a real window and can close it or quit the browser, and the page's own script can close the window. Golden scenario 04 ("Log in once with a person, reuse the login headless") lists this as failure point 1, "the person never finishes, or closes the window". Its scoring relied on the wording of `CallCancelledError`, which does not name the window (G6). The leaders blur the cases. Playwright 1.64.1's `page.pause()` fails with one `TargetClosedError`, "Target page, context or browser has been closed", for a closed page and a crashed one alike ([`client/page.ts` L179–180, L902](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/client/page.ts#L179-L180)). Closing its Inspector window resumes the script as if the person had pressed Resume ([`recorderApp.ts` L115–116](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/recorder/recorderApp.ts#L115-L116), [`recorder.ts` L341–342](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/recorder.ts#L341-L342)). Puppeteer raises `TargetCloseError` and WebDriver `no such window`, and neither has a pause. Browser Use Cloud has no call that waits: a person uses its live view and a later run carries on, and a client going away stops nothing there, since "a completed run or closed CDP connection does not immediately stop its cloud browser" ([Human in the loop](https://docs.browser-use.com/cloud/agent/human-in-the-loop.md)). Decided in [Ending a Human pause by closing the window](https://github.com/goodfetch/goodfetch/issues/532), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), by the agent on the owner's delegation ([grilling record](https://github.com/goodfetch/goodfetch/issues/532#issuecomment-6194565395)). It extends ADR 0092's owner-confirmed decisions and reverses none of them. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### A page closed under a call

- **`call_cancelled`, with who and what.** A page whose window or tab is closed in the browser, outside every Client, raises `call_cancelled` (ADR 0080) in each call in flight on it. The cause can be a person on the desktop or the page's own `window.close()`. `page.pause()` is one such call, and so are the Gestures a paused page holds back (ADR 0092) and any wait or read. Its details say `by: "browser"` and `closed: "page"`. The engine renders `observed`: "the page's window was closed in the browser, outside every Client". Its `fix` names the two ways on: open a page and pause again, or, to refuse the step, have the person use "Decline". The engine cannot tell a person's click from the page's own script, so it names the browser, never a person (G1).
- **The browser quit.** A headful browser that quits ends every Session in it, including a browser that quits by itself when its last window closes, as Chromium does on some desktops. Calls in flight on any of their pages raise `call_cancelled` with `by: "browser"` and `closed: "browser"`, and each of those Sessions ends.
- **What stays.** When only the page closed, its Session stays open, with its cookies and its other pages. The page emits `close`, and later calls on it raise ADR 0078's `closed`, as for a page a program closed (ADR 0085) or one a crawler closed (ADR 0089). A value never reopens (ADR 0078). The engine opens no new page to carry the pause on.
- **Typed details for every `call_cancelled`.** `CallCancelledDetails { by, closed, person }` applies to every case ADR 0080 lists:
  - `by` says who stopped the call: `person` (a person in the Console), `client` (another Client), `program` (the program closing a parent value from another task) or `browser` (above). It is the vocabulary of an Agent's `abort` (ADR 0100), plus `browser`.
  - `closed` says what was closed or stopped under the call: `page`, `session`, `browser`, `run` or `connection`.
  - `person` names whom the stopping Client acts for (ADR 0031), when known, and is otherwise null.

  Values a newer engine adds are kept as received (ADR 0079). Rust reads them through `Error::details::<CallCancelled>()` (ADR 0099). Golden scenario 04 can now branch on `closed === "page"` without reading text (G6).
- **An Agent's task** whose page is closed in the browser returns `ended: stopped` with the reason `abort` and `by: "browser"`, as ADR 0100 has every task do that someone other than the caller stops.

### What does not end a pause

- **A lost Live view ends nothing.** A person who closes the Console's tab, loses the network or lets the device sleep leaves the pause waiting, under `timeout.human_pause`. They come back through "Needs you" or the Notification's link. A control lock held through a lost viewer goes back to *waiting* under ADR 0031's idle-holder rule, never to automation. Ending the pause would report a step as done that nobody did, as closing Playwright's Inspector does (G1).
- **The Live view and the Console close no page that a Human pause holds.** They offer "Done" and "Decline" instead. A grant holder therefore cannot end a program's pause by closing its page from the Live view, and still answers one only through `until` (ADR 0092, G5); a page its own script closes is reported as `by: "browser"`, as above.
- **A renderer crash stays `page_crashed`** (ADR 0085), an `UnavailableError`. A crash is not a close, and the two stay apart where Playwright joins them. How a browser process that exits abnormally is told apart from a quit browser is left to area 5's spec below.

### What is recorded

- **The Run trace** records how each Human pause ended:
  - by its condition;
  - by a person's "Done" or "Hand back";
  - by a "Decline";
  - past its timeout;
  - by the program's own cancel;
  - or stopped, with `by` and `closed`.

  This extends the start and end that ADR 0102 already records.
- **The Audit log** records each pause that a person's answer ended ("Done", "Hand back", "Decline", with the person) and each pause that its page or browser closing ended (`by: "browser"`, with no person). This joins ADR 0031's record of taking control and handing back. An end caused by the condition, the timeout or the program's own cancel is no one's act, so only the Run trace keeps it.
- **The Run report gains nothing.** A pause is not an outcome of the Run. An error that escapes the program ends the Run through its close (ADR 0078), as any error does (S2). ADR 0102 already rejected a sequence of pauses beside `progress()`. The pause leaves `progress().pauses` and the Console's "Needs you", and no Notification is sent for its end.

## Considered Options

- **`human_pause_declined` with a `how` detail:** rejected. A decline is a person's answer, with their name, their note and a page still usable afterwards. A closed page has no known person, and the program's next call on that page would fail with `closed`. A grant holder, or the page itself, would also gain a way to decline that ADR 0092 keeps for a person (G1, ADR 0031).
- **A new code (`page_closed`, `human_pause_abandoned`):** rejected by S2. One event would get two codes, depending on which call was in flight, or else the new code would replace ADR 0080's for every call. Typed details on `call_cancelled` name the window as clearly.
- **ADR 0078's `closed` for the call in flight:** rejected. It is a `ProgramError`, "the program used a value it closed", which names the wrong remedy (G6). It stays the code for later calls on the closed page.
- **A lost Live view, or a closed headful window, counting as "Done",** as closing Playwright's Inspector resumes the script: rejected by G1.
- **Reopening the page and keeping the pause:** rejected. A value never reopens (ADR 0078), and the person may have closed it on purpose.
- **A list of pauses in the Run report:** rejected. A pause is not a Run outcome. `progress()` and the Run trace already hold pauses (ADR 0102), and the Audit log holds who answered them.

## Consequences

- **Amends ADR 0092.** A pause whose page or browser closes raises `call_cancelled` with `by: "browser"`. A lost Live view ends no pause, and the Live view closes no paused page. Notes sit under "What it returns" and "Where the person acts".
- **Amends ADR 0080.** `call_cancelled` carries `CallCancelledDetails { by, closed, person }`, and a page or browser closed outside every Client is one more way someone else stops a call. A note sits under "`call_cancelled` is never the caller's own cancellation".
- **Amends ADR 0085.** A page closed in the browser raises `call_cancelled` in each call in flight on it and `closed` in later calls, so "closed in the browser" is a fourth case beside the three ADR 0085 tells apart. A note sits under its crash and close rules.
- **Amends ADR 0100.** `abort`'s `by` gains `browser`. A note sits under its stop reasons.
- **Amends ADR 0031.** The Audit log also records each Human pause that a person's answer or a closed page ended, and the Live view closes no page that a Human pause holds. A note sits under "Audit".
- **Adds to ADR 0102.** The Run trace records how each pause ended. A note sits under `pauses`.
- **Error codes.** No new code. `call_cancelled` gains typed details in `goodfetch-protocol`, through `xtask codegen` (ADR 0079), and `browser` joins the values of `by`.
- **`GLOSSARY.md`** is unchanged: the Human pause, the Live view and the Audit log keep their meanings.
- **`TESTING.md`** gains the Human pause ending cases.
- **Area 5 (Site access)** builds how the engine detects a page or browser closed outside every Client, and decides how a browser process that exits abnormally is reported apart from a quit.
  > Note (agent decision, pending owner review), 2026-10-23, [#731](https://github.com/goodfetch/goodfetch/issues/731): the engine follows each page's target. A target destroyed with no close the engine sent is closed in the browser, unless the browser's connection ends within 2 s, when it is read with the rest of the quit or crash: the browser's windows close before its process exits. A launched browser whose process exits with status 0 quit (as on `SIGTERM`, the signal a desktop's quit sends); one that exits non-zero, by a signal, or with no exit seen within 2 s of its connection ending crashed. A Borrowed browser, whose exit the engine never sees, quit when its connection ends once its Session is open (spec #594's decision 11; recorded 2026-10-28 after the deep review of M1, D16): its calls raise `call_cancelled` naming the browser, and no crash counts in Usage. Every Session seated in a browser that quit or crashed takes its end, one that holds no page included, and the browser leaves its Run's pool at once (S8, S9). `CallCancelledDetails` carries `by` (`client`, `program` or `browser`) and `closed` (`page`, `session` or `browser`); `person`, `run` and `connection` join with the tickets that raise them. A page's `crash` and `close` are the events `page.crash` and `page.close`, published while `page.on` listens to them.

  **Area 3 (Engine runtime)** decides the code of a call in flight that a Limit's stop ends, a pause included (ADR 0092 has the Limit stop the Run). **Area 12 (Console)** builds the Live view's refusal to close a paused page and the Audit log entries. **Area 11 (Client surfaces)** builds `CallCancelledDetails` per language. Each lists this ADR.
- **ADR 0037's matrix:** pain 18 (`agent-human-in-loop`) cites this ADR.
- **Other tickets of the map.** [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485) receives the P40 and P08 rows as filled here.
- **The `prototype/sdk-api` sketch** holds `CallCancelledDetails` in its `results` files for each language, `browser` in the Agent's `abort`, and the pause's doc text in its `sessions` files. Golden scenario 04 scores failure point 1 on these details. The pain coverage matrix adds these endings to P40 and P08.
