# Failures raise grouped, typed errors, and expected outcomes are values

Every SDK reports what happened in two ways that never mix: a failure raises a typed error, and an expected outcome is a value in what the call returns. No option turns one into the other. Errors are one class per Error code under seven groups named for their remedy, carry the fields G6 asks for, and show the target as the caller wrote it; a crawl request ends in one of four outcomes; AI work reports how it ended, its Verdict and what the model claimed as three separate fields. Competitors fail here in both directions: Browser Use, Firecrawl and Skyvern returned failures that looked like results, while Stagehand v4 both returns and throws failures and its docs match on message text, as Playwright for Python's users are told to do ([#1425](https://github.com/microsoft/playwright-python/issues/1425)). Decided in [Results, outcomes and errors](https://github.com/goodfetch/automan/issues/466), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), on the evidence of the [research on competitors' APIs](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) (pains 3 and 7; [AI browser automation](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) §6 rows 2 and 8; [browser drivers](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/browser-drivers.md) P8; [crawl frameworks](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §6.4 and §6.15) and of the [SDK design guidelines research](https://github.com/goodfetch/automan/blob/research/sdk-design-guidelines/rnd/research/2026-10-14-sdk-design-guidelines.md) (§1.3, §2.2, §3, §6, §8). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Failures raise, expected outcomes are values

- **The rule.** A call raises when it could not do what its name promises. It returns an outcome when its promise includes "no": a question (is it there, how many), one request among many (a crawl's requests), or work judged by checks (a Verdict). TypeScript and Python throw; in Rust every call that can fail returns `Result<T, goodfetch::Error>` and outcomes live in `T`. No option, setting or overload changes which (S2), and Python's sync and async APIs raise the same classes.
- **How G1 reads.** G1 counts "blocked by policy" among expected outcomes. Under the rule, it is an outcome where the call's promise includes "no" (a crawl request, enqueueing a link) and a failure where a program asks outright for what its own settings forbid (`goto` to a domain outside the run's policy). Azure draws the same line per method, returning `false` from `exists()` and raising `ResourceNotFoundError` from `get()` for the same missing resource; Playwright's `goto` returns a 404 as a `Response`, while `click` raises when its target never appears. ADR 0077 is unchanged.
- **Cases:**

  | Case | Result |
  |---|---|
  | `locator.count()` is 0; `isVisible()` is false | a value: the answer |
  | `click()`'s target never appears before the deadline | raises a `DeadlineExceededError` |
  | the target matches two elements | raises a `PageError` listing the matches |
  | `goto()` answers 404 or 500 and no Page check is declared | a value: the response and its status (S3) |
  | `goto()` reaches a page that fails its Page check, or meets a Block cause it cannot get past | raises a `PageError` (`page_check_failed` under ADR 0006, or a code naming the Block cause) |
  | `goto()` to a domain the run's own policy forbids | raises a `RefusedError` |
  | a crawl request fails, is blocked or is skipped | a value: its outcome |
  | AI work stopped by a Limit, or a Success check fails | a value: `ended` is `stopped`, or `verdict` is `rejected` (ADR 0029) |
  | a call refused because a Limit is already used up | raises a `RefusedError` |
  | the engine is unreachable; an option is unknown | raises an `UnavailableError`; a `ProgramError` |

  Each verb's ticket classifies its own codes by this rule, such as drift with no model, an uncertain Effect after `act`, or an extraction missing a field.

### Error classes

- **Names.** The base is `GoodfetchError` in TypeScript and Python and `goodfetch::Error` in Rust. Each Error code has one generated class (ADR 0038), named the code in PascalCase plus `Error`: `page_check_failed` gives `PageCheckFailedError`. Every error carries `code` and `name`, and TypeScript's `code` is a literal union, so a program can check them without `instanceof`, which fails when a bundle holds two copies of the package ([azure-sdk-for-js#28059](https://github.com/Azure/azure-sdk-for-js/issues/28059)). A code named like its group (`deadline_exceeded`, `internal`) shares the group's class.
- **No shadowing.** `xtask codegen` refuses a code whose class name would collide with a built-in or standard name of any SDK language (`TimeoutError`, `ConnectionError`, `PermissionError`, `AbortError`, `TypeError`), and the catalog names it differently on every surface, with no per-language alias (S1). Playwright for Python's `TimeoutError` shadows Python's own and cannot be renamed without breaking its API ([playwright-python#1495](https://github.com/microsoft/playwright-python/issues/1495)).
- **Seven groups by remedy.** Each code belongs to exactly one, declared in `goodfetch-protocol` beside its retryable flag and carried as its JSON-RPC numeric class (ADR 0038), so a program catches a remedy without listing codes. The codes JSON-RPC 2.0 predefines, which ADR 0038 keeps for protocol failures, map to groups too: invalid params and method not found to `ProgramError`; parse error, invalid request and internal error to `InternalError`.

  | Group | Meaning | Remedy |
  |---|---|---|
  | `ProgramError` | The program is wrong: its call, its settings or its own handler code | Fix the program (G3) |
  | `RefusedError` | A policy, a Limit, a lock or a person refused | Change a setting, raise the Limit, ask the operator |
  | `PageError` | The page or site never reached the state the call needed: a target found absent without waiting out a deadline (as an AI verb that finds none) or not unique, a failed Page check, a block, AI work not verified | Read the Run trace; change the step, the wait or the Access mode |
  | `ModelError` | The Model provider's answer was unusable: a refusal, structured output that does not validate | Change the model or the strategy |
  | `DeadlineExceededError` | The deadline passed, including a call that waited for its target until then; it carries the target and what was observed | Raise it, or wait for the right thing |
  | `UnavailableError` | Something Goodfetch depends on is unreachable or gone: the engine, the connection, the browser, the Model provider, a Browser host, a Sink | Retry, or fix the setup |
  | `InternalError` | A bug in Goodfetch | Report it |

  The set is closed and under semver: a new group or a code moved between groups is a breaking change (ADR 0030). This is Azure's rule, a new type only where the remedy differs, as the guidelines research recommends.
- **Cancellation.** A cancellation the caller asked for keeps its language's own form; that form and the `cancelled` code's group are [Time, waits, deadlines, cancellation and budgets](https://github.com/goodfetch/automan/issues/467)'s.
- **A handler's own exception.** An exception from the user's code inside an SDK handler that is not a `GoodfetchError` becomes `handler_failed`, a `ProgramError`. Its details carry the exception's type name, its message and the stack frames of the user's code as text, since the outcome outlives the process and the Console reads it; in the process that raised it, the original is also its cause. A `GoodfetchError` raised inside a handler keeps its own code and retryable flag. ADR 0033's `hook_failed` carries a Hook's own exception with the same details; a Goodfetch code raised inside a Hook stays wrapped in `hook_failed`, naming that code and carrying its retryable flag, as ADR 0051 does for Script Hooks. Whether `handler_failed` is retried by default is [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477)'s.

### What an error carries

- **Always:** `code`, `group`, `retryable`, `message`, `operation` (the SDK call, such as `locator.click`), `callId` (to find the call in the Run trace) and, inside a run, `runId`.
- **When they apply:** `target`; `url`; `observed`, what was there instead; `limit`, the setting key, its value and the settings layer that set it (ADR 0028); `fix`, what would change the outcome, naming setting keys or operations; `callLog`, the engine's steps for the call, as Playwright's call log; the cause, through the language's own chaining; and typed `details` per code.
- **Never** a Secret's value: a Secret reference shows as the reference (ADR 0015).
- **One message**, rendered from the fields: the operation and what failed, then the target, URL, observed, limit and fix, then the call log. It never repeats the text of a cause chained in the same process, which each language's own reporting prints from the chain; `handler_failed` and `hook_failed` name the user's exception type and message in theirs, since outside the process that raised it their details are all a reader has. The engine renders the message of the errors it sends, and the SDK runtime renders the errors it raises itself (`closed`, a dropped connection, an Attached engine's exit, `not_verified`) in the same layout. Its text is outside semver; codes, groups and fields are inside it (ADR 0030). There are no per-audience message modes, unlike Notte's process-wide `set_error_mode` (G2).
- **The target in the caller's language.** The engine renders it in the language the Client declares in `hello`, as Playwright's server renders locators for its `SDKLanguage`, and sends the structured target with it, so Python shows `page.get_by_role("button", name="Next")` and TypeScript `page.getByRole('button', { name: 'Next' })`. One renderer is tested once instead of three that drift; each new SDK language adds a renderer to the engine. The CLI's and MCP's renderings belong to their own designs.

### Outcomes are values

- **A crawl request** ends in exactly one of:
  - `succeeded`;
  - `failed`, with its error;
  - `skipped`, with a reason, such as out of scope, a duplicate, Site preferences, a Hook or the handler;
  - `blocked`, with a Block cause.

  Retries are a count, not an outcome: every outcome carries `attempts` and the errors of the failed attempts, so a request retried and then failed counts once. Live states (pending, in flight, a Human pause) are [The Run in code](https://github.com/goodfetch/automan/issues/478)'s, dropped and rejected Records (ADR 0033) are [Crawl output in code](https://github.com/goodfetch/automan/issues/479)'s, and the handler's `skip` and `retry` are Crawl definition and handler context's, which also settles the noun; until then the type is `RequestOutcome`.
- **AI work that can claim success** (an Agent's task, a Plan replayed toward a goal) carries three separate fields:
  - `ended`: `finished`, or `stopped` with its reason (a Limit by name, an abort); a failed ending raises instead;
  - `verdict`: `verified`, `rejected` or `needs-review`, given by checks under ADR 0029's rules (a declared Success check must pass, ADR 0013) and never by the model's claim;
  - `reported`: what the model said (done, gave up, its message), a signal and never proof.

  `requireVerified()` in TypeScript and `require_verified()` in Python and Rust return the output, or raise `NotVerifiedError` (`not_verified`, a `PageError`) carrying the Verdict, the reasons it is not `verified` (failed, missing or switched-off checks, uncertain Effects) and the model's claim. They exist on these results only, as httpx's `raise_for_status()` turns an outcome into an error only when asked. [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474) and [Collie's AI verbs and Plans](https://github.com/goodfetch/automan/issues/473) build their result types on these fields.
- **Partial results.** A value of a declared type is always complete. Data gathered before a failure travels on the error, in its code's details, such as the fields an extraction found and the problem with each missing one. Data gathered before an expected early end travels on the outcome, in a field typed as partial. A call over many items reports one outcome per item, never all or nothing and never an `ExceptionGroup`. Records a crawl committed stay in its Dataset, and its Completeness says what is missing (ADR 0029).

### A newer engine's codes and values

- An Error code an SDK does not know raises the class of the group its numeric class names, or the base class, with the code kept as received.
- Outcome kinds, skip reasons, Block causes, claims and Verdicts it does not know are kept as strings: `Known | (string & {})` in TypeScript, `Literal[...] | str` in Python, and in Rust `#[non_exhaustive]` enums with an `Unknown(String)` variant, so that adding a value in a minor breaks no build (ADR 0030). A tagged union gets an `unknown` member that keeps the received tag. An unknown Verdict never equals `verified`.
- A Daemon or Goodfetch Cloud can run a newer minor than the SDK (ADR 0024, G3); openai-python's strict parsing raised on results that had drifted ([#1763](https://github.com/openai/openai-python/issues/1763)).

### Retries

- Apart from the two cases below, an SDK never retries a call on its own; `retryable` tells the program that trying again is safe and may help. The engine already retries work items under settings (ADR 0028), Sink deliveries (ADR 0008) and its model calls (ADR 0012). ADR 0024's `run_moved` redirect, which retries once, is unchanged.
- After the runtime reconnects a dropped connection (ADR 0038), it re-sends an in-flight call only when the catalog classes the operation read or idempotent write. A call that is an Effect raises an `UnavailableError`, and the Effect journal tells whether it happened or is uncertain (ADRs 0010 and 0029).

### Rust

- `goodfetch::Error` is one opaque struct implementing `std::error::Error + Send + Sync + 'static` (C-GOOD-ERR), with an accessor for each field above. `code()` returns an `ErrorCode`, `#[non_exhaustive]` with `Unknown(String)` and `as_str()`; `group()` returns an `ErrorGroup`; `details()` returns an `Option<&ErrorDetails>`, a `#[non_exhaustive]` enum holding `goodfetch-protocol`'s own structs, so nothing is generated. Its cause is `source()`. Its `Display` is the message the other SDKs show, longer than C-GOOD-ERR's usual concise line because its fields are what make it diagnosable (G6), and without the cause's text, which reporters such as anyhow print from `source()`. `Error::fake(code, message)`, behind the crate's `test-util` feature (STYLE.md), builds one for a program's own tests.
- Results that carry an outcome, such as a request's outcome and AI work's result, are `#[must_use]`.
- TypeScript and Python carry the same `details`, typed per class (S1).

## Considered Options

- **A raise switch**, as Notte's `raise_on_failure` per session or call: rejected. `scrape`'s return type then depends on an option, which S2 forbids and Crawl4AI's `arun()` shows, and every reader must know the default.
- **Values only**, as Browser Use's `ActionResult(error=…)`, Skyvern's run status and Crawl4AI's `success`: rejected by G1. Several of Browser Use's built-in actions returned "element not found" as a non-error, so its failure limit never counted it ([#5361](https://github.com/browser-use/browser-use/issues/5361), [#5438](https://github.com/browser-use/browser-use/issues/5438)), and Firecrawl returned failed jobs like successful ones ([#4212](https://github.com/firecrawl/firecrawl/issues/4212)).
- **Exceptions for everything**: rejected. Azure's users complain of expected outcomes thrown ([azure-sdk-for-python#9507](https://github.com/Azure/azure-sdk-for-python/issues/9507)), and ADR 0029 already keeps `stopped` and `rejected` apart from `failed`.
- **Leaf classes only**, or Playwright's three classes: rejected. A program would list codes to catch one remedy. **Groups by subsystem** (engine, browser, model, crawl): rejected, since they do not tell the caller what to do. A base that misses its siblings fails the same way: Notte's `NotteAPIError` does not catch `AuthenticationError` or `InvalidRequestError`, so the class a program reaches for misses failures.
- **Per-language renames of colliding names**: rejected by S1; the CLI and MCP would show yet another name.
- **Minimal errors with a free data map**, as Stagehand's `RPCError(code, data)`: rejected, since programs fall back to the message text. **Messages per audience**, as Notte's: rejected by G2.
- **Each SDK rendering the target**: rejected; three renderers to keep in agreement, the drift Selenium's and Spider's bindings showed. **One neutral rendering** (JSON or CSS): rejected, since people and agents would read a target that is not in their code.
- **`retried` as a fifth request outcome**, as the research listed: rejected; a request retried and then failed would count twice. **Only succeeded and failed**, as Crawlee v3, where skipping meant throwing and then lowering `requestsFailed` by hand, and failed requests counted as handled: rejected, since blocks and skips would read as bugs.
- **One status enum for AI work**, as Skyvern's `completed`, `terminated` and `timed_out`: rejected, as ADR 0029 rejected it for runs. **Booleans**, as Browser Use's `is_done()` and `is_successful()`: rejected; the agent's own flag reads as proof, and with an output schema it defaulted to success ([#5955](https://github.com/browser-use/browser-use/issues/5955)).
- **Rejecting unknown codes and values**: rejected by G3. **Mapping them to `InternalError`**: rejected, since a valid outcome would read as a bug.
- **Automatic retries in the SDK**, as Stainless's SDKs retry twice: rejected. They would double the engine's retries, and an Effect is never redone blindly (ADR 0010).
- **One public Rust enum of every error**, as chromiumoxide and thirtyfour: rejected; each variant repeats the common fields, and adding one breaks callers. **AWS's `SdkError<E, R>`**: rejected for its three levels of nesting ([aws-sdk-rust#572](https://github.com/awslabs/aws-sdk-rust/issues/572)) and its terse `Display`, which left users asking for an error-handling guide ([#191](https://github.com/awslabs/aws-sdk-rust/issues/191)).

## Consequences

- **Adds to ADR 0038.** Each Error code also has a group, carried as its JSON-RPC numeric class; codegen generates the seven group classes and refuses a class name that shadows a built-in.
- **Adds to ADR 0029.** An AI task has a Verdict too, by the same rules; `GLOSSARY.md`'s Verdict says so.
- **Adds to ADR 0024.** `hello` carries the Client's language, in which the engine renders targets.
- **Adds to ADR 0030.** Error groups and error fields join what semver covers, beside Error codes; error message text, like log text, does not.
- **Adds to ADRs 0033 and 0051.** `hook_failed` carries a Hook's own exception with the details `handler_failed` carries for a handler's, and the wrapping ADR 0051 gives Script Hooks, naming the Goodfetch code and carrying its retryable flag, extends to every Hook.
- **Adds to ADR 0078.** Its `closed` code is a `ProgramError`, and an Attached engine's unexpected exit raises an `UnavailableError` carrying the stderr tail.
- **Error codes.** `handler_failed` and `not_verified` join `goodfetch-protocol`'s list. The case table's other codes are examples: each area's spec and the ticket that owns the call name its codes, their groups and their details.
- **Area 11 (Client surfaces)** builds these rules and lists this ADR. Client conformance cases cover them in every SDK ([TESTING.md](../../TESTING.md#sdks)).
- **Tickets that build on it.** The Agent and AI verbs tickets shape their results and partial types on these fields. Crawl definition and handler context settles the request's noun, `skip`, `retry` and the default for `handler_failed`. The Run in code decides how a program reads outcomes and explains a call that did nothing. Time, waits, deadlines, cancellation and budgets decides cancellation and the deadline codes. [Collie's page API](https://github.com/goodfetch/automan/issues/472) decides whether a response offers `ok()`.
- **The `prototype/sdk-api` sketch** holds this ADR's signatures in `results` files for each language.
