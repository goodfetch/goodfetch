# A program and Collie take turns on a page other code drives, and an AI verb fails when a navigation it did not start replaces its document

`run.borrow(page)` (ADRs 0056, 0078, 0089) hands Collie a page that the program's own Playwright, Puppeteer or crawler code keeps driving. Each holds its own CDP session on the tab. [ADR 0085](0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md) runs a page's Gestures one at a time, but that order covers only the calls that reach the engine: the program's driver sends its commands straight to the browser. [ADR 0056](0056-a-login-is-brought-in-through-one-session-source-read-by-the-browser-that-owns-it.md) only marks results on such a browser unguarded. So an `extract` replayed from a Plan, or an `act`, would read or act on whatever document the program's driver had moved the page to while the call ran. Golden scenario 09 scores this failure point ("the two libraries act on the page at the same time") silent in TypeScript and Python, which G1 never accepts.

The browser decides what is possible here:
- **No exclusive attach exists.** Chrome has accepted several CDP clients on one tab since Chrome 63: "Two separate WebSocket protocol clients, such as Puppeteer or chrome-remote-interface, can now connect to the same tab simultaneously" ([New in DevTools 63](https://developer.chrome.com/blog/new-in-devtools-63)). `Target.attachToTarget` has no exclusive mode ([`browser_protocol.json`](https://github.com/ChromeDevTools/devtools-protocol/blob/master/json/browser_protocol.json)).
- **No client sees another's commands.** No client is told about another client's `Input.*` commands. The one input switch, `Input.setIgnoreInputEvents`, makes the page ignore all input, the engine's own included.
- **Every client sees every navigation.** `Page.frameStartedNavigating` "is fired for both renderer-initiated and browser-initiated navigations", and `Page.frameNavigated` fires once the frame "is now associated with the new loader". None of these events says who started the navigation.

The leaders notice a navigation that lands under a call but report it as text, "Execution context was destroyed, most likely because of a navigation" (Playwright's `server/frames.ts`, Puppeteer's `cdp/ExecutionContext.ts`). WebDriver has a typed `stale element reference` for an element whose document is gone ([W3C WebDriver](https://w3c.github.io/webdriver/)).

Decided in [The program's own driver acting during a Collie call on a borrowed page](https://github.com/goodfetch/goodfetch/issues/533) ([grilling record](https://github.com/goodfetch/goodfetch/issues/533#issuecomment-6194584047)), raised by [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/goodfetch/issues/483), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Taking turns, with no hold

- **The program and Collie take turns on a page other code drives.** The program awaits each Collie call before its own driver acts on that page again, as golden scenario 09 does. This rule is stated in the doc text of `run.borrow()` and `page.lend()`, in the crawler and lending guides (ADR 0089), and in the error below.
- **Nothing holds the page for a Collie call.** The engine cannot hold it: CDP has no exclusive attach, and ignoring input would block the engine's own. The SDK does not wrap or patch the program's Playwright or Puppeteer page either. That page is a third-party object the program lent (G4, G7), and wrapping it would miss the crawler's own code, a second connection, DevTools and extensions.
- **`run.borrow()` gains no `exclusive` option.** The option means "nothing else drives it". On a page the program borrowed from its own driver, that is false by construction. It would also put the engine back on the guarded path, where it types Secrets, on a browser other code reads (ADR 0056, G5). `exclusive` stays honoured only on a `cdp_url`, as ADR 0056 says.

### Which pages

A page other code also drives is any of these:
- a page of a Borrowed browser that ADR 0056 treats as shared: `client_browser`, `running_browser`, or a `cdp_url` not declared `exclusive`;
- a Lent page while a library is attached ([ADR 0089](0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md));
- a page with an open CDP session from `page.newCDPSession()` (ADR 0085).

These are the pages whose results those ADRs already mark unguarded because other code drives them. On a page only the engine drives, nothing here changes. Any navigation the engine did not start there comes from the page itself, which [ADR 0091](0091-collies-ai-verbs-live-on-the-page-replay-from-a-plan-file-the-run-names-and-expect-runs-a-success-check.md)'s Target fingerprint check and the guard already handle.

### What the engine detects

- **An AI verb fails when a navigation it did not start replaces its document.** On a page other code drives, the engine watches its main frame during each `act`, `observe` and `extract`, on a page or a locator, compiled or replayed from the Plan. The call fails with `page_navigated_during_call` if the frame commits a navigation to a new document between the call's first read of the page and its return. An `act` is watched until its input is sent; after that, a navigation counts as the action's own, as ADR 0085 has an action wait for the navigation it starts. An AI verb's answer or action is made from a document it observed earlier, so a document replaced in between is what it must not return or act on silently (G1).
- **An Agent's task gets it as a step's outcome.** During `page.agent` ([ADR 0100](0100-collies-agent-is-one-bounded-call-on-a-page-judged-by-the-programs-success-check-and-replayed-whole-from-the-plan.md)), the same navigation is given back to the model as a step's outcome, naming both addresses, as ADR 0100 does for a failed page action. The task's steps record it, and its loop observes the page again. A task replayed from the Plan meets it as Drift, and ADR 0100's live loop takes over.
- **Other calls keep ADR 0085's rules.** A plain locator call resolves its target again on the new document, as Playwright's does when another driver moves the page (S3). A pointer Gesture that lands elsewhere is cancelled and replayed by the guard. Waits and Success checks (`waitForURL`, `expect(page).toHaveURL()`) are unchanged: waiting for a navigation the program's own driver starts is what they are for.
- **What CDP never shows stays outside the guarantee.** The other driver's keystrokes, pointer moves and DOM writes within one document are invisible to the engine, and so are same-document navigations, which a page's own script also makes. The turn-taking rule, the doc text and the guides name these limits, and Provenance keeps ADR 0056's mark that the results are unguarded.

### The failure

- **`page_navigated_during_call`, a `PageError`:** "the page or site never reached the state the call needed" (ADR 0079). It is retryable, since the page may now be where the call needs it. In a crawl handler, ADR 0093's retry policy decides. The class is `PageNavigatedDuringCallError` under ADR 0079's naming rule.
- **What it carries** (G6):
  - `url`, the document the call read, and `navigatedTo`, the document that replaced it;
  - a `fix` saying to await the Collie call before the program's own driver acts on the page, and that the page may also have navigated by itself;
  - the call's `usage` when a compile made model calls (ADR 0101).
- **Why not `page_changed_by_other_client`.** The engine cannot tell the program's navigation from the page's own script, a person in a headful window or an extension, so it does not claim a cause it cannot know (G1). "Client" is also `GLOSSARY.md`'s term for a program that drives the engine, and speaking CDP to a browser does not make a program one.
- **What a failed call leaves.**
  - No Plan entry is written, and no Repair runs on the new document.
  - An `act` fails before its input, so nothing was sent.
  - The Run trace records the navigation beside the failed call.

### Golden scenario 09

The added step's `extract` runs on the borrowed Travel page. If the program's own driver clicked a book while the step ran, the step would fail with `page_navigated_during_call`, which the program's `catch` (`except GoodfetchError`) reports before it carries on. Failure point 3 is therefore typed in TypeScript and Python, and Goodfetch's totals become 72 typed and 1 generic of 73, with none silent. The Rust program drives the browser through Goodfetch's Session alone, so its failure point 3 stays typed through ADR 0085's order.

## Considered Options

- **The engine holds the page during a call:** rejected. CDP has no exclusive attach, `Input.setIgnoreInputEvents` would also block the engine's own input, and a navigation could be stopped only by failing navigation requests whose sender the engine cannot tell.
- **The SDK wraps or patches the program's Playwright page** so its calls wait for Collie's: rejected by G4 and G7. The SDK would patch a third-party object it was lent, tie itself to that library's internals, and still miss every other driver of the page.
- **An `exclusive` option on `run.borrow()`:** rejected. On a page the program's own driver holds, the declaration is false by construction, and it would let the engine type Secrets on a browser other code reads (G5).
- **Failing every call during which a navigation it did not start commits:** rejected. Waits and Success checks exist to wait for the navigation the program's own driver starts, and a plain locator call that names its own target is right to resolve again on the new document (ADR 0085, S3).
- **Attributing the change to another driver** (`page_changed_by_other_client`): rejected. The engine cannot know the cause, and "Client" means something else in `GLOSSARY.md` (G1).
- **Documenting the rule only, scored generic:** rejected by G1. A replayed `extract` and an `act` would still act on whatever page they found, with no error.
- **Failing an Agent's task:** rejected. ADR 0100 gives a task's page events back to the model, and its loop observes the page again at every step.
- **Applying the rule on a page only the engine drives:** rejected. There every navigation the engine did not start is the page's own, which ADR 0091's Target fingerprint check and the guard already handle, so failing the call would only make ordinary redirects fail.

## Consequences

- **Amends ADR 0056.** On a browser other code also drives, an AI verb whose document is replaced by a navigation it did not start fails with `page_navigated_during_call`, and the program and Collie take turns on a borrowed page. A note sits under "The person's code is a raw CDP client".
- **Amends ADR 0085.** The Gesture order covers only the calls that reach the engine. Another driver's commands on the same page are outside it, and what the engine detects of them is decided here. A note sits under "Gestures on one page".
- **Amends ADR 0089.** The program's own AI verbs on a Lent page while a library is attached, and Collie's on a page borrowed inside another crawler, follow this ADR. Notes sit under the table of what a Lent page keeps and loses, and under what Collie loses inside other crawlers.
- **Amends ADR 0091.** `page_navigated_during_call` joins its error codes. A note sits under its table.
- **Amends ADR 0100.** On a page other code drives, a navigation the task did not start is given back to the model as a step's outcome. A note sits under its Failures table.
- **Error codes.** `page_navigated_during_call` joins `goodfetch-protocol` in the `page` group, retryable, through `xtask codegen`'s naming checks (ADR 0079).
- **`TESTING.md`** gains the borrowed page turn-taking conformance cases.
- **Area 5 (Site access)** builds the main-frame watch during AI verbs on the pages listed here. **Area 9 (AI and agent)** builds the failure in the AI verbs and the Agent's step outcome. **Area 11 (Client surfaces)** builds `PageNavigatedDuringCallError` per language and the doc text of `run.borrow()` and `page.lend()`. Each lists this ADR.
- **ADR 0037's matrix:** attaching to a running or remote browser (SA-08) and pain 12 (`integ-sdk-llm-runtime`) cite this ADR.
- **The `prototype/sdk-api` sketch** holds `PageNavigatedDuringCallError` in each language and the doc text on `borrow` and `lend`. Golden scenario 09 scores its failure point 3 typed, and the pain coverage matrix's rows P20 and P41 name this answer.
