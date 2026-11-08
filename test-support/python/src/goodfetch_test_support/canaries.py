"""The Secret canary search (ADR 0067), as Rust's `Canaries::search` does it.

It finds every place an output holds a canary, raw and in Goodfetch's
encodings (JSON-escaped, URL-encoded, base64 standard or URL-safe at any
alignment, hex in either case), with the output and the byte offset of each
hit. A test searches what the SDK shows, its `repr`, `str`, errors and logs,
for canaries it planted through `goodfetch-test-env` or the Proving ground.
Hits name a canary by ID, never by value.
"""

import base64
import json
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

from goodfetch_test_support.generated.secret_kinds import (
    SECRET_KINDS as SECRET_KINDS,
)
from goodfetch_test_support.generated.secret_kinds import (
    SecretKind as SecretKind,
)

Encoding = Literal["raw", "json_escaped", "url_encoded", "base64", "hex"]
"""The encodings a value is searched in."""

Placement = Literal["planted", "leaked"]
"""Whether a hit is where the canary was planted, or a leak."""

_ALPHANUMERIC = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
_UNRESERVED = frozenset(_ALPHANUMERIC + b"-._~")
"""The letters, digits and marks RFC 3986 leaves unreserved."""
_FORM_KEPT = frozenset(_ALPHANUMERIC + b"*-._")
"""What a browser keeps as it encodes a form, which writes a space `+`."""
_URI_COMPONENT_KEPT = frozenset(_ALPHANUMERIC + b"-_.!~*'()")
"""What JavaScript's `encodeURIComponent` keeps."""


class CanaryOutputUnreadable(Exception):
    """A Secret canary search could not read one of its outputs."""


@dataclass(frozen=True)
class Canary:
    """A planted canary."""

    id: str
    """Its ID, which hits name it by."""
    kind: SecretKind
    """Its kind."""
    value: str = field(repr=False)
    """Its value, never shown in its `repr`."""
    planted_in: tuple[str, ...] = ()
    """The outputs the test planted it in, whose hits are planted copies."""


@dataclass(frozen=True)
class NamedOutput:
    """Bytes or text the test holds, such as a captured `repr`, named for hits."""

    name: str
    """The name hits give it."""
    data: bytes | str
    """Its bytes, or text read as UTF-8."""


@dataclass(frozen=True)
class CanaryHit:
    """One place an output holds a canary."""

    canary: str
    """The canary's ID."""
    kind: SecretKind
    """Its kind."""
    encoding: Encoding
    """The encoding it was found in."""
    output: str
    """The output's name, or the file's path."""
    offset: int
    """The byte offset where the encoded value starts."""
    placement: Placement
    """Whether the output is where the canary was planted."""


def _url_encoded(value: bytes, kept: frozenset[int]) -> str:
    """`value` percent-encoded, every byte but those in `kept`."""
    return "".join(chr(byte) if byte in kept else f"%{byte:02X}" for byte in value)


def _base64_at(value: bytes, alignment: int, *, url_safe: bool) -> bytes:
    """The base64 characters encoding `value` alone, `alignment` bytes into a group."""
    padded = bytes(alignment) + value
    encode = base64.urlsafe_b64encode if url_safe else base64.b64encode
    encoded = encode(padded).rstrip(b"=")
    start = (0, 2, 3)[alignment]
    end = len(padded) // 3 * 4 + len(padded) % 3
    return encoded[start:end] if start < end else b""


def _forms(canary: Canary) -> list[tuple[Encoding, bytes]]:
    """Each searched form of the value, a form an earlier encoding gives left out."""
    value = canary.value.encode("utf-8")
    found: list[tuple[Encoding, bytes]] = []

    def add(encoding: Encoding, form: bytes) -> None:
        if form and all(earlier != form for _, earlier in found):
            found.append((encoding, form))

    add("raw", value)
    add("json_escaped", json.dumps(canary.value, ensure_ascii=False)[1:-1].encode())
    add("url_encoded", _url_encoded(value, _UNRESERVED).encode())
    add("url_encoded", _url_encoded(value, _FORM_KEPT).replace("%20", "+").encode())
    add("url_encoded", _url_encoded(value, _URI_COMPONENT_KEPT).encode())
    for url_safe in (False, True):
        for alignment in range(3):
            add("base64", _base64_at(value, alignment, url_safe=url_safe))
    add("hex", value.hex().encode())
    add("hex", value.hex().upper().encode())
    return found


def _files(path: Path) -> list[Path]:
    """`path` itself when it is a file, else every file under it, in path order."""
    if not path.is_dir():
        return [path]
    return [file for entry in sorted(path.iterdir()) for file in _files(entry)]


def _read(output: Path | NamedOutput) -> list[tuple[str, bytes]]:
    if isinstance(output, NamedOutput):
        data = output.data
        return [(output.name, data.encode("utf-8") if isinstance(data, str) else data)]
    try:
        return [(str(file), file.read_bytes()) for file in _files(output)]
    except OSError as error:
        message = f"reading {output} for Secret canaries"
        raise CanaryOutputUnreadable(message) from error


def _occurrences(data: bytes, form: bytes) -> list[int]:
    offsets: list[int] = []
    at = data.find(form)
    while at != -1:
        offsets.append(at)
        at = data.find(form, at + 1)
    return offsets


def search_canaries(
    canaries: Sequence[Canary], outputs: Sequence[Path | NamedOutput]
) -> list[CanaryHit]:
    """Every place `outputs` hold one of `canaries`.

    Hits come in output order, then by offset, then in canary and encoding
    order.

    Args:
        canaries: The canaries to search for.
        outputs: Files or folders (every file under one, in path order), or
            named bytes or text.

    Returns:
        Each hit.

    Raises:
        CanaryOutputUnreadable: A path cannot be read.
    """
    searched = [(canary, _forms(canary)) for canary in canaries]
    hits: list[CanaryHit] = []
    for output in outputs:
        for name, data in _read(output):
            found: list[tuple[int, int, int, CanaryHit]] = []
            for order, (canary, forms) in enumerate(searched):
                placement: Placement = (
                    "planted" if name in canary.planted_in else "leaked"
                )
                for index, (encoding, form) in enumerate(forms):
                    found.extend(
                        (
                            offset,
                            order,
                            index,
                            CanaryHit(
                                canary.id,
                                canary.kind,
                                encoding,
                                name,
                                offset,
                                placement,
                            ),
                        )
                        for offset in _occurrences(data, form)
                    )
            found.sort(key=lambda entry: entry[:3])
            for *_, hit in found:
                if not hits or hits[-1] != hit:
                    hits.append(hit)
    return hits
