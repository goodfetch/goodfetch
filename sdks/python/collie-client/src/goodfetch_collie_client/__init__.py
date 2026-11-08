"""The Collie client package, which both Products' Python SDKs build on (ADR 0095).

Area 11 adds its hand-written runtime; `generated` holds the layer `xtask
codegen` writes from Collie's entries of the catalog, whose operations take
`caller`'s callers and whose exception classes extend `goodfetch_error`'s base.
"""
