"""RPC SDK: newline-delimited JSON-RPC 2.0 to `engined` (stdio sidecar or daemon Unix socket)."""
import asyncio
import itertools
import json
import os
import traceback

PROTOCOL_VERSION = "1.0"
_seq = itertools.count()
LIMIT = 1 << 28  # one JSON line can carry a 10 MB inline page


class RpcError(Exception):
    def __init__(self, code, message, data=None):
        super().__init__(message)
        self.code, self.data = code, data


class RunHandle:
    def __init__(self, engine, req_id, fut):
        self._engine, self._id, self._fut = engine, req_id, fut

    def cancel(self):
        self._engine._send({"method": "$/cancelRequest", "params": {"id": self._id}})

    def __await__(self):
        return self._fut.__await__()


class RpcEngine:
    def __init__(self, reader, writer, proc=None):
        self.reader, self.writer, self.proc = reader, writer, proc
        self.next_id = itertools.count(1)
        self.pending, self.runs, self.inflight = {}, {}, {}
        self.info = None
        self._task = asyncio.get_running_loop().create_task(self._read_loop())

    @classmethod
    async def sidecar(cls, binary, protocol_version=PROTOCOL_VERSION):
        proc = await asyncio.create_subprocess_exec(binary, "stdio", stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, limit=LIMIT)
        e = cls(proc.stdout, proc.stdin, proc)
        e.info = await e.request("initialize", {"protocolVersion": protocol_version, "sdk": "python"})
        return e

    @classmethod
    async def connect(cls, sock, protocol_version=PROTOCOL_VERSION):
        r, w = await asyncio.open_unix_connection(sock, limit=LIMIT)
        e = cls(r, w)
        e.info = await e.request("initialize", {"protocolVersion": protocol_version, "sdk": "python"})
        return e

    def _send(self, msg):
        self.writer.write((json.dumps({"jsonrpc": "2.0", **msg}) + "\n").encode())

    def request(self, method, params, req_id=None):
        req_id = req_id or next(self.next_id)
        fut = asyncio.get_running_loop().create_future()
        self.pending[req_id] = fut
        self._send({"id": req_id, "method": method, "params": params})
        return fut

    async def _read_loop(self):
        while line := await self.reader.readline():
            m = json.loads(line)
            method = m.get("method")
            if method == "handle":
                t = asyncio.create_task(self._serve(m))
                self.inflight[m["id"]] = t
            elif method == "events":
                if m["params"]:
                    hooks = self.runs.get(m["params"][0]["run_id"])
                    if hooks and hooks[1]:
                        hooks[1](m["params"])  # synchronous: a slow consumer stops this loop reading
            elif method == "$/cancelRequest":
                t = self.inflight.get(m["params"]["id"])
                if t:
                    t.cancel()
            else:
                fut = self.pending.pop(m.get("id"), None)
                if fut and not fut.done():
                    if "error" in m:
                        e = m["error"]
                        fut.set_exception(RpcError(e["code"], e["message"], e.get("data")))
                    else:
                        fut.set_result(m.get("result"))
        for fut in self.pending.values():
            if not fut.done():
                fut.set_exception(ConnectionError("engine connection closed"))

    async def _serve(self, m):
        try:
            handler = self.runs[m["params"]["run_id"]][0]
            res = await handler(m["params"])
            self._send({"id": m["id"], "result": res})
        except asyncio.CancelledError:
            self._send({"id": m["id"], "error": {"code": -32800, "message": "cancelled"}})
        except Exception as e:  # noqa: BLE001 - forwarded to the engine
            self._send({"id": m["id"], "error": {"code": -32000, "message": str(e), "data": {"stack": traceback.format_exc()}}})
        finally:
            self.inflight.pop(m["id"], None)

    def start(self, opts, handler, on_event=None):
        opts = {**opts, "run_id": opts.get("run_id") or f"run-{os.getpid()}-{next(_seq)}"}
        req_id = next(self.next_id)
        self.runs[opts["run_id"]] = (handler, on_event)
        fut = self.request("run", opts, req_id)
        fut.add_done_callback(lambda _f: self.runs.pop(opts["run_id"], None))
        return RunHandle(self, req_id, fut)

    async def run(self, opts, handler, on_event=None):
        h = self.start(opts, handler, on_event)
        try:
            return await h
        except asyncio.CancelledError:
            h.cancel()
            raise

    async def close(self):
        self.writer.close()
        if self.proc:
            await self.proc.wait()
        self._task.cancel()
