# goodfetch-collie

Collie's Python SDK (ADRs 0090 and 0095): `AsyncCollie` and the sync `Collie`, built on the Collie client package. Its platform wheels hold Collie's executable in `goodfetch_collie/bin/`.

Nothing is read, patched or logged at import, and the SDK writes nothing to stdout or stderr: its loggers sit under `goodfetch`, which gets a `NullHandler` and nothing else (ADR 0078). The engine's stderr lines go to `goodfetch.client` at warning. The engine sends its log events from the effective level of `goodfetch.engine` (`CRITICAL` and `ERROR` give `error`, `WARNING` `warn`, `INFO` `info`, `DEBUG` `debug`, anything lower `trace`), read at `connect()` and again when a Run starts, and told to the engine only when it changed; each goes to `goodfetch.engine.<area>`, the second segment of its target, at its level, `trace` at 5, with `target`, `fields` and `dropped` as the record's attributes. The logger names are listed in [`../manifest.toml`](../manifest.toml).
