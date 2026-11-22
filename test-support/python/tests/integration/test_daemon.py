"""The `daemon` fixture is the Daemon of the test's own home (ADRs 0024 and 0094).

A Client reaches it through the described socket with the described token;
it is run by the executable of the test's `product`, Collie's unless the test
needs Foxhound, and stops at teardown.
"""

import json
import socket
import sys
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import Daemon, Product, TestEnv

if TYPE_CHECKING:
    from conftest import SampleRun

PROTOCOL_VERSION = "0.1"
"""The Protocol version of this commit's executables."""

PRODUCT_VERSION = "0.0.0"
"""The Product version of this commit's executables."""


@pytest.fixture
def with_daemon() -> bool:
    """The module's tests ask for a Daemon, as a product test does."""
    return True


def _connect(address: str) -> socket.socket:
    """Opens the described socket: a Unix socket, or loopback TCP on Windows."""
    if sys.platform == "win32":
        host, port = address.rsplit(":", 1)
        return socket.create_connection((host, int(port)), timeout=10)
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.settimeout(10)
    connection.connect(address)
    return connection


def _hello(daemon: Daemon, token: str, product: Product) -> object:
    """The Daemon's answer to a `hello` with `token`; `None` if closed unanswered."""
    request = {
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": PROTOCOL_VERSION,
            "surface": "sdk_python",
            "token": token,
            "needs": product.value,
            "product_version": PRODUCT_VERSION,
        },
    }
    with _connect(daemon.socket) as connection:
        connection.sendall(json.dumps(request).encode() + b"\n")
        line = connection.makefile("rb").readline()
    return json.loads(line) if line else None


def test_daemon_is_collies_reached_through_its_socket_with_its_token(
    daemon: Daemon,
) -> None:
    """The Daemon answers `hello` with the described token, as Collie."""
    answer = _hello(daemon, daemon.token, Product.COLLIE)

    assert isinstance(answer, dict)
    assert answer["result"]["product"] == "collie", answer


def test_daemon_refuses_a_hello_without_its_token(daemon: Daemon) -> None:
    """A `hello` with another token is closed unanswered."""
    answer = _hello(daemon, daemon.token + "0", Product.COLLIE)

    assert answer is None


def test_daemon_is_the_one_whose_runtime_file_is_in_the_tests_home(
    daemon: Daemon, test_env: TestEnv
) -> None:
    """The runtime file of the test's home names the described token."""
    runtime_file = json.loads(
        (test_env.home / "0.0" / "daemon.json").read_text("utf-8")
    )

    assert runtime_file["token"] == daemon.token


@pytest.mark.parametrize("product", [Product.FOXHOUND])
def test_daemon_of_a_test_that_needs_foxhound_is_run_by_foxhounds_executable(
    daemon: Daemon, product: Product
) -> None:
    """With `product` overridden to Foxhound, Foxhound's executable runs it."""
    answer = _hello(daemon, daemon.token, product)

    assert isinstance(answer, dict)
    assert answer["result"]["product"] == "foxhound", answer


WITHOUT_DAEMON_TEST = """
from goodfetch_test_support import Daemon


def test_reaches(daemon: Daemon) -> None:
    assert daemon.token
"""


def test_daemon_without_with_daemon_fails_the_fixture(
    sample_run: "SampleRun",
) -> None:
    """A test that takes `daemon` but asks for none errors, never runs."""
    outcome = sample_run.run(WITHOUT_DAEMON_TEST)

    assert outcome.results == {"test_reaches": "error"}, outcome.output
    assert "DaemonNotRequested" in outcome.output
