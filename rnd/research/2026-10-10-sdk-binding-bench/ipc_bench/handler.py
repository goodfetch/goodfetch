"""Python handler host: newline-delimited JSON-RPC 2.0 on stdin/stdout (asyncio)."""
import asyncio
import json
import sys


async def handler(params):
    return {"n": len(params["html"])}


async def serve(msg, writer):
    result = await handler(msg["params"])
    writer.write((json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}) + "\n").encode())


async def main():
    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader(limit=64 * 1024 * 1024)
    await loop.connect_read_pipe(lambda: asyncio.StreamReaderProtocol(reader), sys.stdin)
    transport, protocol = await loop.connect_write_pipe(asyncio.streams.FlowControlMixin, sys.stdout)
    writer = asyncio.StreamWriter(transport, protocol, reader, loop)
    tasks = set()
    while True:
        line = await reader.readline()
        if not line:
            break
        t = asyncio.create_task(serve(json.loads(line), writer))
        tasks.add(t)
        t.add_done_callback(tasks.discard)


asyncio.run(main())
