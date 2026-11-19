"""The Collie client package, which both Products' Python SDKs build on (ADR 0095).

Area 11 adds its hand-written runtime; `generated` holds the layer `xtask
codegen` writes from Collie's entries of the catalog, whose operations take
`caller`'s callers, whose times are `point_in_time`'s and whose exception
classes extend `goodfetch_error`'s base.

Its loggers sit under `goodfetch`, which gets a `NullHandler` and nothing
else (ADR 0078): a host that configures no logging sees none of the engine's
stderr lines, which Python's last-resort handler would otherwise print.
"""

import logging

logging.getLogger("goodfetch").addHandler(logging.NullHandler())
