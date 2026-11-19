"""The Collie client package, which both Products' Python SDKs build on (ADR 0095).

Area 11 adds its hand-written runtime; `generated` holds the layer `xtask
codegen` writes from Collie's entries of the catalog, whose operations take
`caller`'s callers and whose exception classes extend `goodfetch_error`'s base.

Its loggers sit under `goodfetch`, which gets a `NullHandler` and nothing
else (ADR 0078): a host that configures no logging sees none of the engine's
stderr lines or log events, which Python's last-resort handler would
otherwise print. `host_logging` names them.
"""

import logging

logging.getLogger("goodfetch").addHandler(logging.NullHandler())
