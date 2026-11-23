"""The Proving ground `goodfetch-test-env --proving-ground <seed>` starts (ADR 0063).

It holds what a test needs to reach the Proving ground with no setup of its
own (ADR 0059): its address, version and seed, the `resolve` mapping of every
host name it serves to loopback, the file of its certificate authority, and
its network fixtures. Python reads what the Rust setup started, and starts
nothing. Its routes come from `test-support/fixtures/proving-ground/routes.json`,
which `xtask codegen` writes from the Proving ground's own interface.
"""

import json
import urllib.request
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Self, cast

from goodfetch_test_support.schemas import shared_fixture


class ProvingGroundUnseeded(Exception):
    """A test took `proving_ground` without overriding `proving_ground_seed`."""


@dataclass(frozen=True)
class ProxyCredentials:
    """A forward proxy's Basic credentials, derived from the seed."""

    username: str
    password: str


@dataclass(frozen=True)
class ProvingGroundFixtures:
    """The network fixtures `serve` starts beside its pages."""

    proxies: Mapping[str, str]
    """Each forward proxy's address, by name: `open`, `refusing_tunnels`,
    `down` and one per proxy kind."""
    proxy_credentials: ProxyCredentials
    """The credentials the authenticated proxies accept."""
    dns: str
    """The DNS fixture's UDP address."""
    target: str
    """The watched target's address, which answers `200 reached`."""
    connections: str
    """The URL of the connection record: every connection accepted and every
    relay a proxy made."""


@dataclass(frozen=True)
class ProvingGround:
    """A started Proving ground, as `goodfetch-test-env` describes it."""

    address: str
    """The address it answers on, such as `127.0.0.1:51234`."""
    version: str
    """Its version, `<workspace version>+<commit>`."""
    seed: int
    """The seed every page and expected result comes from."""
    resolve: Mapping[str, str]
    """Every host name it serves, mapped to the IP of its address."""
    ca: Path
    """The file of its certificate authority, as PEM, in the test's folder."""
    fixtures: ProvingGroundFixtures
    """The addresses of its network fixtures."""

    @classmethod
    def from_json(cls, value: object) -> Self:
        """Reads the description's `proving_ground`, already checked by its schema."""
        fields = cast("dict[str, object]", value)
        fixtures = cast("dict[str, object]", fields["fixtures"])
        credentials = cast("dict[str, str]", fixtures["proxy_credentials"])
        return cls(
            address=cast("str", fields["address"]),
            version=cast("str", fields["version"]),
            seed=cast("int", fields["seed"]),
            resolve=dict(cast("dict[str, str]", fields["resolve"])),
            ca=Path(cast("str", fields["ca"])),
            fixtures=ProvingGroundFixtures(
                proxies=dict(cast("dict[str, str]", fixtures["proxies"])),
                proxy_credentials=ProxyCredentials(
                    username=credentials["username"],
                    password=credentials["password"],
                ),
                dns=cast("str", fixtures["dns"]),
                target=cast("str", fixtures["target"]),
                connections=cast("str", fixtures["connections"]),
            ),
        )

    def url(self, host: str, path: str) -> str:
        """The HTTP URL of `path` on its page host `host`, such as `fetch-edge.test`."""
        port = self.address.rpartition(":")[2]
        return f"http://{host}:{port}{path}"

    def start_url(self, task: str) -> str:
        """The URL its task `task` starts at, read from its task list; it blocks."""
        tasks = f"http://{self.address}{proving_ground_routes().tasks}"
        with urllib.request.urlopen(tasks) as answer:
            listed = cast("dict[str, list[dict[str, str]]]", json.load(answer))
        return next(
            entry["start_url"] for entry in listed["tasks"] if entry["id"] == task
        )


@dataclass(frozen=True)
class ProvingGroundRoutes:
    """The paths of the Proving ground's own routes.

    They are served on its bound address rather than on a page family's host.
    """

    tasks: str
    """`GET`: the task list."""
    oracle: str
    """`POST`: judges a tool's result for one task."""
    time_step: str
    """`PUT`: the time step its pages answer at from now on."""
    observations: str
    """`GET`: every request its page hosts received, and the connection record."""
    client_certificate: str
    """`GET`: the client certificate its mutual-TLS hosts accept."""
    client_key: str
    """`GET`: that certificate's private key."""
    article_answers: str
    """`GET <path>/<size>`: the right answer of each page of the article site."""


def proving_ground_routes() -> ProvingGroundRoutes:
    """The Proving ground's routes, read from the committed shared fixture."""
    routes = cast(
        "dict[str, str]",
        shared_fixture(
            "proving-ground/routes.json", "proving-ground-routes.schema.json"
        ),
    )
    return ProvingGroundRoutes(**routes)
