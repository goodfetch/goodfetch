"""SDK handlers conformance tests in Python (ADRs 0033, 0104 and 0132; #1029).

Covers: story:7-72, story:7-73, story:7-114, story:7-76
"""

from __future__ import annotations

import json
import threading
from typing import TYPE_CHECKING, Any

import anyio
import pytest
from anyio.abc import ByteStream
from goodfetch_collie import (
    ExtractContentParams,
    ExtractContentResult,
    ExtractHandler,
    SettingInvalidError,
    ValueInvalidError,
)
from goodfetch_collie.testing import async_connect_over, connect_over
from opentelemetry import trace

if TYPE_CHECKING:
    from anyio.streams.memory import MemoryObjectReceiveStream, MemoryObjectSendStream

COVERS = pytest.mark.covers("story:7-72", "story:7-73", "story:7-114", "story:7-76")


class _MemoryByteStream(ByteStream):
    """Bidirectional in-memory byte stream for scripted engine tests."""

    def __init__(
        self,
        incoming: MemoryObjectReceiveStream[bytes],
        outgoing: MemoryObjectSendStream[bytes],
    ) -> None:
        self._incoming = incoming
        self._outgoing = outgoing

    async def receive(self, max_bytes: int = 65536) -> bytes:
        del max_bytes
        try:
            return await self._incoming.receive()
        except anyio.EndOfStream:
            raise

    async def send(self, item: bytes) -> None:
        await self._outgoing.send(item)

    async def send_eof(self) -> None:
        await self._outgoing.aclose()

    async def aclose(self) -> None:
        await self._incoming.aclose()
        await self._outgoing.aclose()


class _ScriptedPeer:
    """In-memory engine peer reading/writing NDJSON JSON-RPC lines."""

    def __init__(self) -> None:
        c2e_send, c2e_recv = anyio.create_memory_object_stream[bytes](64)
        e2c_send, e2c_recv = anyio.create_memory_object_stream[bytes](64)
        self.client_stream = _MemoryByteStream(e2c_recv, c2e_send)
        self._from_client = c2e_recv
        self._to_client = e2c_send
        self._buf = b""

    async def send_from_engine(self, payload: dict[str, Any]) -> None:
        line = (json.dumps({"jsonrpc": "2.0", **payload}) + "\n").encode()
        await self._to_client.send(line)

    async def next_from_client(self) -> dict[str, Any]:
        while b"\n" not in self._buf:
            chunk = await self._from_client.receive()
            self._buf += chunk
        raw, self._buf = self._buf.split(b"\n", 1)
        result: dict[str, Any] = json.loads(raw.decode())
        return result

    async def close_engine(self) -> None:
        await self._to_client.aclose()


def _hello_result() -> dict[str, Any]:
    return {
        "protocol_version": "0.1",
        "product": "collie",
        "product_version": "0.0.0",
        "engine_instance": "scripted-py-1",
        "timeouts": [],
    }


@COVERS
@pytest.mark.anyio
async def test_async_sdk_handler_routes_errors_cancel_and_trace_context() -> None:
    """Async ExtractHandler serves calls, maps errors, cancels, and nests spans."""
    peer = _ScriptedPeer()
    seen_traces: list[tuple[int, int, str]] = []
    cancelled_event = anyio.Event()
    started_slow = anyio.Event()

    class MyExtractor(ExtractHandler):
        async def content(self, params: ExtractContentParams) -> ExtractContentResult:
            ctx = trace.get_current_span().get_span_context()
            if ctx.is_valid:
                seen_traces.append(
                    (ctx.trace_id, ctx.span_id, ctx.trace_state.to_header())
                )
            if "goodfetch-error" in params.url:
                raise ValueInvalidError("bad html", details={"value": "html"})
            if "boom" in params.url:
                msg = "py extractor exploded"
                raise RuntimeError(msg)
            if "slow" in params.url:
                started_slow.set()
                try:
                    await anyio.sleep(60)
                except anyio.get_cancelled_exc_class():
                    cancelled_event.set()
                    raise
            return ExtractContentResult(
                title="Title",
                text=f"text:{params.html}",
                markdown=f"# {params.html}",
                metadata={"source_kind": "sdk"},
            )

    async def engine_driver() -> None:
        hello = await peer.next_from_client()
        assert hello["method"] == "hello"
        assert hello["params"]["handlers"] == {"extract": ["custom"]}
        await peer.send_from_engine({"id": hello["id"], "result": _hello_result()})

        # Trigger client call so reader loop is active while engine sends requests.
        req = await peer.next_from_client()
        assert req["method"] == "config.explain"

        # 1. Happy path + incoming Trace context.
        traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        await peer.send_from_engine(
            {
                "id": 201,
                "method": "extract.content",
                "params": {
                    "url": "https://example.test/ok",
                    "html": "<p>hi</p>",
                    "handler": "custom",
                    "_meta": {
                        "traceparent": traceparent,
                        "tracestate": "vendor=py",
                    },
                },
            }
        )
        ok_reply = await peer.next_from_client()
        assert ok_reply["id"] == 201
        assert ok_reply["result"] == {
            "title": "Title",
            "text": "text:<p>hi</p>",
            "markdown": "# <p>hi</p>",
            "metadata": {"source_kind": "sdk"},
        }

        # 2. GoodfetchError keeps its code.
        await peer.send_from_engine(
            {
                "id": 202,
                "method": "extract.content",
                "params": {
                    "url": "https://example.test/goodfetch-error",
                    "html": "<p>x</p>",
                    "handler": "custom",
                },
            }
        )
        gf_reply = await peer.next_from_client()
        assert gf_reply["id"] == 202
        assert gf_reply["error"]["data"]["code"] == "value_invalid"

        # 3. Other exception becomes extract_handler_failed with type, message, stack.
        await peer.send_from_engine(
            {
                "id": 203,
                "method": "extract.content",
                "params": {
                    "url": "https://example.test/boom",
                    "html": "<p>x</p>",
                    "handler": "custom",
                },
            }
        )
        boom_reply = await peer.next_from_client()
        assert boom_reply["id"] == 203
        assert boom_reply["error"]["data"]["code"] == "extract_handler_failed"
        assert boom_reply["error"]["data"]["handler"] == "custom"
        assert boom_reply["error"]["data"]["exception_type"] == "RuntimeError"
        expected_msg = "py extractor exploded"
        assert boom_reply["error"]["data"]["exception_message"] == expected_msg
        assert "RuntimeError" in boom_reply["error"]["data"]["exception_stack"]

        # 4. $/cancel cancels the async task and answers call_cancelled.
        await peer.send_from_engine(
            {
                "id": 204,
                "method": "extract.content",
                "params": {
                    "url": "https://example.test/slow",
                    "html": "<p>slow</p>",
                    "handler": "custom",
                },
            }
        )
        await started_slow.wait()
        await peer.send_from_engine({"method": "$/cancel", "params": {"id": 204}})
        cancel_reply = await peer.next_from_client()
        assert cancel_reply["id"] == 204
        assert cancel_reply["error"]["data"]["code"] == "call_cancelled"
        await cancelled_event.wait()

        # Answer outer config.explain.
        await peer.send_from_engine(
            {"id": req["id"], "result": {"settings": [], "other_product": []}}
        )
        await peer.close_engine()

    async with anyio.create_task_group() as tg:
        tg.start_soon(engine_driver)
        async with async_connect_over(
            peer.client_stream, extractors={"custom": MyExtractor()}
        ) as collie:
            await collie.explain_settings()

    assert seen_traces == [
        (0x4BF92F3577B34DA6A3CE929D0E0E4736, 0x00F067AA0BA902B7, "vendor=py")
    ]


@COVERS
@pytest.mark.anyio
async def test_async_connect_fails_with_setting_invalid_for_two_handlers() -> None:
    """Two handlers of a single-key kind fail connect() with SettingInvalidError."""
    peer = _ScriptedPeer()

    async def dummy(params: ExtractContentParams) -> ExtractContentResult:
        return ExtractContentResult(
            title="", text=params.html, markdown="", metadata={}
        )

    async def engine_driver() -> None:
        hello = await peer.next_from_client()
        assert hello["params"]["handlers"] == {"extract": ["a", "b"]}
        await peer.send_from_engine(
            {
                "id": hello["id"],
                "error": {
                    "code": -32001,
                    "message": "`content.extractor` is invalid: two handlers",
                    "data": {
                        "code": "setting_invalid",
                        "retryable": False,
                        "key": "content.extractor",
                        "reason": "ambiguous",
                    },
                },
            }
        )
        await peer.close_engine()

    async with anyio.create_task_group() as tg:
        tg.start_soon(engine_driver)
        with pytest.raises(SettingInvalidError):
            await async_connect_over(
                peer.client_stream, extractors={"a": dummy, "b": dummy}
            )


@COVERS
def test_sync_sdk_handler_discards_late_answer_on_cancel_and_nests_trace() -> None:
    """Sync handler runs on worker thread, nests trace, discards late answer."""
    seen_traces: list[tuple[int, int, str]] = []
    slow_started = threading.Event()
    release_slow = threading.Event()
    slow_finished = threading.Event()
    client_replies: list[dict[str, Any]] = []

    class _SyncScriptStream(ByteStream):
        """Queue-backed stream for driving sync Collie from a scripted engine."""

        def __init__(self) -> None:
            self._to_client: list[bytes] = []
            self._to_client_cond = threading.Condition()
            self._closed = False
            self._engine_buf = b""
            self._phase = 0

        def _push_engine(self, payload: dict[str, Any]) -> None:
            raw = (json.dumps({"jsonrpc": "2.0", **payload}) + "\n").encode()
            with self._to_client_cond:
                self._to_client.append(raw)
                self._to_client_cond.notify_all()

        def _close_engine(self) -> None:
            with self._to_client_cond:
                self._closed = True
                self._to_client_cond.notify_all()

        async def receive(self, max_bytes: int = 65536) -> bytes:
            del max_bytes

            def pop() -> bytes | None:
                with self._to_client_cond:
                    while not self._to_client and not self._closed:
                        self._to_client_cond.wait(timeout=0.05)
                    if self._to_client:
                        return self._to_client.pop(0)
                    return None

            chunk = await anyio.to_thread.run_sync(pop)
            if chunk is None:
                raise anyio.EndOfStream
            return chunk

        async def send(self, item: bytes) -> None:
            self._engine_buf += item
            while b"\n" in self._engine_buf:
                raw, self._engine_buf = self._engine_buf.split(b"\n", 1)
                msg: dict[str, Any] = json.loads(raw.decode())
                client_replies.append(msg)
                self._on_client_message(msg)

        def _on_client_message(self, msg: dict[str, Any]) -> None:
            if msg.get("method") == "hello":
                assert msg["params"]["handlers"] == {"extract": ["sync_ext"]}
                self._push_engine({"id": msg["id"], "result": _hello_result()})
            elif msg.get("method") == "config.explain":
                self._explain_id = msg["id"]
                sync_tp = "00-4bf92f3577b34da6a3ce929d0e0e4736-1122334455667788-01"
                self._push_engine(
                    {
                        "id": 301,
                        "method": "extract.content",
                        "params": {
                            "url": "https://example.test/ok",
                            "html": "<p>sync</p>",
                            "handler": "sync_ext",
                            "_meta": {
                                "traceparent": sync_tp,
                                "tracestate": "vendor=sync",
                            },
                        },
                    }
                )
            elif msg.get("id") == 301:
                assert msg["result"]["text"] == "sync:<p>sync</p>"
                self._push_engine(
                    {
                        "id": 302,
                        "method": "extract.content",
                        "params": {
                            "url": "https://example.test/slow",
                            "html": "<p>slow</p>",
                            "handler": "sync_ext",
                        },
                    }
                )

                def send_cancel() -> None:
                    slow_started.wait(timeout=5.0)
                    self._push_engine({"method": "$/cancel", "params": {"id": 302}})

                threading.Thread(target=send_cancel, daemon=True).start()
            elif msg.get("id") == 302:
                assert msg["error"]["data"]["code"] == "call_cancelled"
                # Release slow sync handler AFTER $/cancel; late answer is discarded.
                release_slow.set()
                slow_finished.wait(timeout=5.0)
                self._push_engine(
                    {
                        "id": self._explain_id,
                        "result": {"settings": [], "other_product": []},
                    }
                )

        async def send_eof(self) -> None:
            self._close_engine()

        async def aclose(self) -> None:
            self._close_engine()

    def sync_handler(params: ExtractContentParams) -> ExtractContentResult:
        ctx = trace.get_current_span().get_span_context()
        if ctx.is_valid:
            seen_traces.append((ctx.trace_id, ctx.span_id, ctx.trace_state.to_header()))
        if "slow" in params.url:
            slow_started.set()
            release_slow.wait(timeout=5.0)
            slow_finished.set()
            return ExtractContentResult(
                title="Late", text="late", markdown="late", metadata={}
            )
        return ExtractContentResult(
            title="Sync",
            text=f"sync:{params.html}",
            markdown=f"# {params.html}",
            metadata={},
        )

    stream = _SyncScriptStream()
    with connect_over(stream, extractors={"sync_ext": sync_handler}) as collie:
        collie.explain_settings()

    assert seen_traces == [
        (0x4BF92F3577B34DA6A3CE929D0E0E4736, 0x1122334455667788, "vendor=sync")
    ]
    # Only one reply for id 302 (the call_cancelled), never a second late result!
    replies_302 = [msg for msg in client_replies if msg.get("id") == 302]
    assert len(replies_302) == 1
    assert replies_302[0]["error"]["data"]["code"] == "call_cancelled"
