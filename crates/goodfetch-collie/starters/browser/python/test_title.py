"""The program's test, on Collie's testing kit.

A test engine's Runs reach only the hosts `network.resolve` maps to
loopback, here the page this test serves itself, so it needs no network.
"""

import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import TYPE_CHECKING

from goodfetch_collie.testing import TestEngine
from main import read_title

if TYPE_CHECKING:
    from goodfetch_collie import Settings


class Page(BaseHTTPRequestHandler):
    """Serves one page, whatever the path."""

    def do_GET(self) -> None:
        """Answers the page."""
        body = b"<!doctype html><title>Hello from the starter</title>"
        self.send_response(200)
        self.send_header("content-type", "text/html; charset=utf-8")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def test_reads_the_pages_title() -> None:
    """The program reads the title of a page the test serves."""
    server = ThreadingHTTPServer(("127.0.0.1", 0), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    try:
        settings: Settings = {"network": {"resolve": {"starter.test": ["127.0.0.1"]}}}
        with TestEngine.start(settings=settings) as engine:
            title = read_title(engine.connection, f"http://starter.test:{port}/")

        assert title == "Hello from the starter"
    finally:
        server.shutdown()
