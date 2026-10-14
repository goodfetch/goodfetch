# SDK schemas come from each language's own schema types, Payloads are typed per Label, and results keep what the SDK does not know

A program describes the data it wants, keeps and passes along in its own language's schema idiom, and Goodfetch carries it as JSON Schema and JSON. The SDKs depend on no single schema library's internals. The program's own validation rules join the model's retry loop. A Dataset may be typed, and a Payload travels with its request in a field of its own, typed per Label and checked when it is enqueued. Goodfetch's own results keep the fields and values an SDK does not know. This answers the research's pain 14 (passing data between handlers) and its schema-coupling half of pain 12 ([research on the leaders' APIs](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md)). Decided in [Typed data: schemas, Records and payloads](https://github.com/goodfetch/automan/issues/469), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), under ADR 0077's principles.

## Decision

### Schemas

- **On the wire, JSON Schema and JSON.** Every schema the engine sees is a JSON Schema document, and every value is JSON (ADR 0038). One schema form serves extraction, a Dataset and a Label's Payload in each language (S1, S2).
- **In code, each language's idiom through a standard interface (G7, S1):**
  - TypeScript: any schema that implements both Standard Schema and Standard JSON Schema, the interfaces Zod (4.2 and later), ArkType (2.1.28 and later) and Valibot (1.2 and later, through its adapter) share. The SDK copies the interfaces' types, as the spec intends, and depends on no schema library. A schema's input and output types reach the SDK API, so a call is typed by what the schema describes.
  - Python: a Pydantic v2 model class, or any type Pydantic's `TypeAdapter` accepts (`list[Book]`, a dataclass, a `TypedDict`).
  - Rust: a type deriving `schemars::JsonSchema` and serde's traits, through the `schemars` the SDK re-exports, so the derive's version always matches the SDK's.
- **The way down (G8).** A JSON Schema document written by hand is accepted in every language, and the values it describes are Goodfetch's own `JsonValue` (`serde_json::Value` in Rust), which the program narrows.
- **Input and output types.** A schema can transform values, as a Zod codec or a Pydantic validator does. What a program sends (an enqueue, a push) takes the schema's input type. What it receives (a handler's Payload, an extraction, a Record read back) is the schema's output type, built by the schema's own parse. Pydantic and serde have one type for both.

### Validation

- **The engine checks the JSON Schema; the program's own rules join the retry loop.** For an extraction, every candidate the model returns is checked against the JSON Schema, and then the engine calls back into the SDK, which runs the schema's native validation: Zod's `.refine()`, a Pydantic `@field_validator`, a custom `Deserialize`. A rejection from either check feeds ADR 0012's bounded retry, which tells the model what was wrong. Only the last failure is reported, and its code and group are [Collie's AI verbs and Plans](https://github.com/goodfetch/automan/issues/473)'s to classify (ADR 0079). The native parse also builds the typed value, so it runs anyway. The callback is one round trip within the program's process per attempt, beside a model call.
- **A validator that throws is the program's bug.** Only the library's own rejection enters the retry loop: Standard Schema issues, Pydantic's `ValidationError`, a serde error. Any other exception stops the call at once with `schema_validator_failed`, a `ProgramError` naming the schema, the field path where known, and the exception's type and message, with the original chained in the process that raised it. A model is never asked to work around a crash, as ADR 0079 keeps a handler's own exception as `handler_failed`. In Rust a `Deserialize` that returns an error is a rejection and a panic stays a panic, so the code never reaches a Rust program.

### Records

- **A Dataset may declare a schema.** A declared Dataset is typed by it (`Dataset<Book>`), so the type checker catches a wrong Record, and the engine validates each Record against the JSON Schema as the authority for every Client and Sink (ADR 0026's declared schema). An undeclared Dataset takes any JSON object. How a Dataset is declared in code, how Records are pushed or read and what becomes of a rejected one are [Crawl output in code](https://github.com/goodfetch/automan/issues/479)'s.

### Payloads

- **A Payload is its own field.** The data a program attaches to a request for its handler never shares a field with the engine's own (the Label, depth, attempts), unlike Crawlee's `userData`, which holds its `label` (crawlee-python #1261).
- **Each Label declares its Payload's schema, or none.**
- **The destination is a typed handle.** An enqueue names its destination by a handle on the Label (`Label<CategoryPayload>`), never by a bare string, so a misspelled Label or a wrong Payload is a type error: scenario 05's fourth failure point. Scrapy's `callback=self.parse_book` is already such a handle. How a crawl declares its Labels and handlers is [Crawl definition and handler context](https://github.com/goodfetch/automan/issues/477)'s, which keeps enqueue's destination typed.
- **Checked at enqueue.** The SDK validates the Payload natively and the engine against the JSON Schema. A failure raises `payload_invalid`, a `ProgramError`, from the enqueue call, naming the Label and each issue's path.
- **Capped in size.** A Payload lives in the Frontier and in every Checkpoint, so its serialized JSON is capped by the setting `crawl.payload.max_bytes`, 64 KiB by default (G5). A larger one is refused with `payload_invalid`, whose `limit` names the setting; large content goes to the Key-value store, and the Payload carries its key.
- **A changed schema is checked before a resume does any work.** The engine keeps a hash of each Label's JSON Schema in the Checkpoint. When a resume finds a Label's schema changed, it checks every pending Payload of that Label against the new schema before any work starts. If any fails, the resume is refused with `payload_schema_changed`, a `ProgramError` naming the Label, how many fail out of how many pending, and the first failure. The program then gives the new field a default, or starts a new Run. A Payload is never migrated silently.

### Only JSON values

- **Records and Payloads hold JSON values.** Anything else (a `datetime`, a `Decimal`, a class instance) is encoded by the program's declared schema: Pydantic writes a `Decimal` as an exact string, a Zod codec or a serde `Serialize` impl chooses its own form. A value that no schema encodes raises a `ProgramError` naming its field path. An SDK never turns a value into a string as a fallback, as crawlee-python did (#524, its most-commented issue), and never pickles one, as Scrapy does with `meta` and `cb_kwargs` (G3).

### Results keep what the SDK does not know

ADR 0079 decided how unknown codes, outcome kinds and enum values are kept. For unknown fields on Goodfetch's own results (G3):

- Python: generated result models allow extra fields, which stay in `model_extra`, and are built without validation (Pydantic's `model_construct`, as Stainless's SDKs build theirs), so drift never raises.
- TypeScript: result objects are passed through as received, never stripped.
- Rust: serde ignores unknown fields when it builds a result's typed view, and each top-level result keeps the JSON it came from, readable through `raw()`. A flattened map of extra fields on every protocol type would cost more for the same reach.
- Inputs stay strict in every language: Pydantic input models forbid extra fields, TypeScript's excess-property checks reject them, and Rust's input builders are non-exhaustive.

### Pinned ecosystem types

- **Widening a pinned major range is a minor; narrowing it follows ADR 0030's rule for runtimes.** Supporting a new major of Pydantic, `schemars` or serde beside the current one is a minor release. Dropping a major happens in a minor only once that major is past its upstream end of life and the drop was announced one minor ahead; before that, it waits for the SDK's next major. Each SDK's release notes state its supported ranges (G7). The TypeScript SDK has no schema library to pin.

### Error codes

`payload_invalid`, `payload_schema_changed` and `schema_validator_failed` join `goodfetch-protocol`'s list in the `program` group (ADR 0079).

## Considered Options

- **One schema library's types as the SDK's input**, as Stagehand takes Zod: rejected by G7. Zod 4 broke Stagehand 2.4–2.5 for about three months (#1029, #938, #1098), and Stagehand v4 still pins Zod to one minor.
- **JSON Schema documents only**, as Skyvern takes: rejected as the main path. Results come back untyped (`dict | list | str | None`), and Skyvern's own TypeScript docs cast them; it stays as the way down.
- **Standard Schema without Standard JSON Schema**: rejected. The base interface validates but cannot describe itself as JSON Schema, which the engine and the model need.
- **Prose shape strings**, as Midscene takes: rejected. Nothing checks them.
- **Ignoring the program's own validation rules**: rejected by G1. A `.refine()` that never runs passes wrong data as valid.
- **Running the native validation once after the engine's check, outside the retry loop**, as Stagehand re-parses with `schema.parse()`: rejected. A failed refinement could not reach the model, though one more attempt is exactly what fixes most of them.
- **Plain JSON objects only for Records**, as Crawlee pushes: rejected. A typed Dataset catches a wrong Record at the type checker, and ADR 0026's Sinks want a declared schema.
- **A free map shared with framework fields**, as Crawlee's `userData` and Scrapy's `meta`: rejected. The research counts four Stack Overflow questions with 168 votes, scrapy#1138 and crawlee-python #524 and #1261 against it.
- **A Label named by string at enqueue**: rejected. A misspelling surfaces in the handler, at run time.
- **Validating a Payload only when its request is dispatched**: rejected. After a schema change, a resumed crawl could run for an hour before its first failure.
- **Migrating stale Payloads with defaults**: rejected. It hides a change the program made, while the fix it would guess, a default in the schema, is one line the program can write.
- **No cap on Payload size**, as Crawlee and Scrapy set none: rejected by G5. A million requests carrying a megabyte each fill a disk silently.
- **Validating results against the SDK's models**: rejected by G3. openai-python's `parse()` raised `ValidationError` when results drifted.
- **Any change to a pinned range as breaking**: rejected. Users would wait on an unsupported Pydantic until the SDK's next major, and runtimes already follow a softer rule.

## Consequences

- **Amends ADR 0012.** The validation its bounded retry feeds back to the model includes the program's own rules, run by the SDK on each candidate.
- **Amends ADR 0030.** A pinned ecosystem type's major range widens in a minor and narrows under its rule for dropping a runtime.
- **Amends ADR 0037's matrix.** RT-07's note says Payload schema validation, since `userData` is an _Avoid_ term of Payload.
- **`STYLE.md`.** Its TypeScript idiom reads Standard Schema with Standard JSON Schema, for extraction, Datasets and Payloads.
- **`GLOSSARY.md`** gains **Label** and **Payload**.
- **Area 6 (Collection)** builds typed Datasets and Payload storage, the size cap and the resume check. **Area 9 (AI and agent)** builds the validation callback inside ADR 0012's retry loop. **Area 11 (Client surfaces)** builds each language's schema input, the typed Label handle, tolerant results and the error classes. Each lists this ADR.
- **The `prototype/sdk-api` sketch** holds this ADR's signatures in its `data` files for each language, with provisional `extract`, `enqueue` and `push` signatures in its snippets until their tickets decide them. The concurrency sketch's placeholder `Record` takes its Dataset's type.
