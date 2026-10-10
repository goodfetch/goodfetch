"""In-process SDK: the engine is the PyO3 extension loaded into this interpreter."""
import asyncio
import itertools
import json
import os

import automan_engine as _e

_seq = itertools.count()


class RunHandle:
    def __init__(self, fut, token):
        self._fut, self._token = fut, token

    def cancel(self):
        self._token.cancel()

    def __await__(self):
        return self._fut.__await__()


class NativeEngine:
    def __init__(self, event_delivery="direct", body="bytes"):
        # event_delivery: "direct" (callback on the engine thread, under the GIL) or "loop"
        # (call_soon_threadsafe). body: "bytes" (one copy) or "memoryview" (zero-copy).
        self.event_delivery, self.body = event_delivery, body

    def start(self, opts, handler, on_event=None):
        loop = asyncio.get_running_loop()
        fut = loop.create_future()
        token = _e.CancelToken()
        opts = {**opts, "run_id": opts.get("run_id") or f"run-{os.getpid()}-{next(_seq)}"}

        def done(js):
            if not fut.done():
                fut.set_result(json.loads(js))

        _e.start_run(json.dumps(opts), handler, on_event, loop, done, token, self.event_delivery, self.body)
        return RunHandle(fut, token)

    async def run(self, opts, handler, on_event=None):
        h = self.start(opts, handler, on_event)
        try:
            return await h
        except asyncio.CancelledError:
            h.cancel()
            raise

    async def close(self):
        pass
