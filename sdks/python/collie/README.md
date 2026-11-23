# goodfetch-collie

Collie's Python SDK (ADRs 0090 and 0095): `AsyncCollie` and the sync `Collie`, built on the Collie client package. Its platform wheels hold Collie's executable in `goodfetch_collie/bin/`.

Nothing is read, patched or logged at import, and the SDK writes nothing to stdout or stderr: its loggers sit under `goodfetch`, which gets a `NullHandler` and nothing else (ADR 0078). The engine's stderr lines go to `goodfetch.client` at warning. The engine sends its log events from the effective level of `goodfetch.engine` (`CRITICAL` and `ERROR` give `error`, `WARNING` `warn`, `INFO` `info`, `DEBUG` `debug`, anything lower `trace`), read at `connect()` and again when a Run starts, and told to the engine only when it changed; each goes to `goodfetch.engine.<area>`, its target as a dotted path without a leading `goodfetch` (`goodfetch::rpc::dispatch` to `goodfetch.engine.rpc.dispatch`), at its level, `trace` at 5, with `target`, `fields` and `dropped` as the record's attributes. The logger names are listed in [`../manifest.toml`](../manifest.toml).

The engine runs in a process group of its own and the SDK installs no signal handler, so a terminal's Ctrl+C reaches only your program, as `KeyboardInterrupt`: open the Connection and the Run with `with` (`async with` for `AsyncCollie`), and the interrupt unwinds through them, closing the Run, which the engine stops with a Checkpoint, then the Connection. SIGTERM's default ends the program without unwinding, so turn it into an exception too, for example `signal.signal(signal.SIGTERM, signal.default_int_handler)`, or close the Run in your own handler (ADRs 0078 and 0093). A call cut short because the engine ended raises `EngineExitedError` (`engine_exited`), whose `details` hold the engine's `exit_code`, the `signal` that ended it and `stderr_tail`, the last 64 KiB of its stderr.

The first use in a process of an Experimental member, one outside semver until it is promoted, warns once through `warnings`, as an `ExperimentalWarning`, a `UserWarning` (ADR 0030). A program that accepts its Experimental members hides the warning with a filter on the category this package exports:

```python
import warnings

from goodfetch_collie import ExperimentalWarning

warnings.filterwarnings("ignore", category=ExperimentalWarning)
```

A Connection, and every Run, Session or page under it, belongs to the process that opened it: a child created by `fork` that uses one gets `ConnectionForkedError` (`connection_forked`) before anything is written to the parent's engine, and connects on its own (ADR 0078).

An `AsyncCollie`, and every Run, Session or page under it, also belongs to the event loop that opened it: a call from another loop or thread gets `EventLoopMismatchError` (`event_loop_mismatch`) before anything is sent. The sync `Collie` runs its own loop, so its values may be called from any thread (ADR 0081).
