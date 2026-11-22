"""Collie's testing kit (ADRs 0069 and 0090).

Opens Collie's Connection over a transport the test holds, such as the
Scripted engine's in-memory player, so a program's tests run its code with
no engine process: the same Connection as `connect()`'s. Importing it reads,
patches and logs nothing (ADR 0078). TypeScript's `@goodfetch/collie/testing`
is its twin.
"""

from goodfetch_collie.collie import async_connect_over, connect_over

__all__ = ["async_connect_over", "connect_over"]
