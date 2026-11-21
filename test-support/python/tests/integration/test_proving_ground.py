"""The `proving_ground` fixture is the Proving ground started on a seed (ADR 0063).

A test reaches its pages through the `resolve` mapping, its forward proxies
and its connection record with no setup of its own (ADR 0059).
"""

import http.client
import json
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import ProvingGround

if TYPE_CHECKING:
    from conftest import SampleRun

SEED = 341


@pytest.fixture
def proving_ground_seed() -> int:
    """The seed the module's Proving ground serves, as a product test sets it."""
    return SEED


def _get(address: str, target: str, host: str) -> tuple[int, str]:
    """Sends `GET <target>` with `Host: <host>` to `address`, as `<ip>:<port>`."""
    ip, port = address.rsplit(":", 1)
    connection = http.client.HTTPConnection(ip, int(port), timeout=10)
    try:
        connection.request("GET", target, headers={"Host": host})
        response = connection.getresponse()
        return response.status, response.read().decode()
    finally:
        connection.close()


def test_proving_ground_answers_on_its_address_with_the_seed_asked_for(
    proving_ground: ProvingGround,
) -> None:
    """Its task list names the seed the test asked for."""
    status, body = _get(proving_ground.address, "/tasks", proving_ground.address)

    assert proving_ground.seed == SEED
    assert (status, json.loads(body)["proving_ground"]["seed"]) == (200, SEED)


def test_proving_ground_reaches_catalog_test_through_its_resolve_mapping(
    proving_ground: ProvingGround,
) -> None:
    """`catalog.test` maps to the address's IP, on the bound port."""
    port = proving_ground.address.rsplit(":", 1)[1]
    ip = proving_ground.resolve["catalog.test"]

    status, body = _get(f"{ip}:{port}", "/catalog/1", f"catalog.test:{port}")

    assert status == 200
    assert "/products/P-0001" in body


def test_proving_ground_reaches_catalog_test_through_the_open_forward_proxy(
    proving_ground: ProvingGround,
) -> None:
    """The open proxy relays the request, and the connection record holds the relay."""
    port = proving_ground.address.rsplit(":", 1)[1]
    url = f"http://catalog.test:{port}/catalog/1"

    status, _ = _get(
        proving_ground.fixtures.proxies["open"], url, f"catalog.test:{port}"
    )

    record_address, record_path = proving_ground.fixtures.connections.removeprefix(
        "http://"
    ).split("/", 1)
    _, record = _get(record_address, f"/{record_path}", record_address)
    relay = json.loads(record)["relays"][0]
    assert status == 200
    assert (relay["proxy"], relay["target"], relay["outcome"]) == (
        "open",
        url,
        "relayed",
    )


def test_proving_ground_names_the_file_of_its_certificate_authority(
    proving_ground: ProvingGround,
) -> None:
    """The file holds the certificate authority as PEM."""
    authority = proving_ground.ca.read_text("utf-8")

    assert authority.startswith("-----BEGIN CERTIFICATE-----\n")


UNSEEDED_TEST = """
from goodfetch_test_support import ProvingGround


def test_pages(proving_ground: ProvingGround) -> None:
    assert proving_ground.seed >= 0
"""


def test_proving_ground_without_a_seed_fails_the_fixture(
    sample_run: "SampleRun",
) -> None:
    """A test that takes `proving_ground` but sets no seed errors, never runs."""
    outcome = sample_run.run(UNSEEDED_TEST)

    assert outcome.results == {"test_pages": "error"}, outcome.output
    assert "ProvingGroundUnseeded" in outcome.output
