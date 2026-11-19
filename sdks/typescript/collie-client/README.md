# @goodfetch/collie-client

The TypeScript Collie client package, which both Products' SDKs share (ADR 0095). It has no executable.

- `.`: what a program holds: `Run`, `Session`, `Page` and `Locator`, whose members `xtask codegen` writes from the catalog, each in the form its kind gives it (ADR 0099), the `Connection` interface `Collie` and `Foxhound` implement, `EngineInfo`, `ConnectOptions` and `ConnectOverOptions`, the error classes (`GoodfetchError`, the seven group classes and one per Collie Error code, each with `code`, `group`, `retryable`, `fix` and `details`) and the Run's types.
- `./runtime`: what each Product's package builds its Connection on, never for programs: `openEngine` and `openOver` (a Product's `connect()` and `connectOver`) and `locateEngine`, each given what tells the Product's package from the other's (the Product `hello` names, its executable, its one engine-path variable and its platform packages), the `ClientConnection` type they answer, `holdInRealm`, `COLLIE_ENGINE_PATH_VARIABLE`, `COLLIE_EXECUTABLE` and `PRODUCT_VERSION`. Behind them, inside the package: the engine choice (`engine-choice.ts`: `auto`, `attached`, `daemon` or a remote Daemon's `{ url, token }`, the Daemon's runtime file and the bytes to it), `AttachedEngine` (the engine's process, its stderr piped and kept) and `ClientConnection`'s work (`hello`, request correlation, `$/cancel` on an aborted call, `closed`, `engine_exited`, also for a write the engine's side rejected, `engine_output_invalid` for a line that is not JSON-RPC, `connection_overloaded` past its bounds on calls waiting and bytes waiting to be written).
- `./generated`: the layer `xtask codegen` writes from Collie's catalog, with the Connection's messages, `PROTOCOL_VERSION` and `PRODUCT_VERSION`, the workspace's version; never edited by hand. Its `handles.ts` holds the members of `Run`, `Session`, `Page` and `Locator`, each in the form its operation's kind gives it (ADR 0099). It re-exports what its methods and classes build on, which is hand-written beside it: `Caller` and `CallOptions` (`caller.ts`) and `GoodfetchError` (`goodfetch-error.ts`); its handles reach the engine through `HandleHost` (`handle.ts`).

Engine answers are not validated beyond `hello`'s protocol version (STYLE.md). Each module imports its siblings with `.ts`, so Node runs the source as it is.

## Schemas

A program types data with a schema in its own types (ADR 0082): any schema that implements both Standard Schema and Standard JSON Schema, or a JSON Schema written by hand (`JSONSchema`), whose values are `JSONValue` and `JSONObject`. `SchemaInput<S>` is what a program sends under `S` and `SchemaOutput<S>` what it receives, built by the schema's own parse. The interfaces' types are copied from `@standard-schema/spec` 1.1.0, so the package depends on no schema library. Only JSON crosses: a value that is not JSON fails from the call that sent it with `value_invalid`, naming its path as a JSON Pointer.

Supported ranges, each tested at its floor:

| Library | Versions |
|---|---|
| Zod | 4.2 and later 4.x |
| ArkType | 2.1.28 and later 2.x |
| Valibot | 1.2 and later 1.x, through `toStandardJsonSchema` of `@valibot/to-json-schema` 1.5 and later |
