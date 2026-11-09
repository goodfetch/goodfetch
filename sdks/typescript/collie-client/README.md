# @goodfetch/collie-client

The TypeScript Collie client package, which both Products' SDKs share (ADR 0095). It has no executable.

- `.`: what a program holds: `Run`, the `Connection` interface `Collie` and `Foxhound` implement, `EngineInfo`, the error classes (`GoodfetchError`, the seven group classes and one per Collie Error code, each with `code`, `group`, `retryable`, `fix` and `details`) and the Run's types.
- `./runtime`: what each Product's package builds its Connection on, never for programs: `AttachedEngine` (the engine's process, its stderr piped and kept), `ClientConnection` (`hello`, request correlation, `$/cancel` on an aborted call, `closed`, `engine_exited`, `engine_output_invalid` for a line that is not JSON-RPC), `COLLIE_ENGINE_PATH_VARIABLE`, `COLLIE_EXECUTABLE` and `PRODUCT_VERSION`.
- `./generated`: the layer `xtask codegen` writes from Collie's catalog, with the Connection's messages, `PROTOCOL_VERSION` and `PRODUCT_VERSION`, the workspace's version; never edited by hand. It re-exports what its methods and classes build on, which is hand-written beside it: `Caller` and `CallOptions` (`caller.ts`) and `GoodfetchError` (`goodfetch-error.ts`).

Engine answers are not validated beyond `hello`'s protocol version (STYLE.md). Each module imports its siblings with `.ts`, so Node runs the source as it is.
