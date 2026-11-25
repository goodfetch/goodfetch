"""The shared Standard Webhooks vectors (ADR 0017) hold in Python too.

The standard library's HMAC-SHA256 and `cryptography`'s ed25519 make every
signed vector's signatures and check every verification case's, so the
SDKs' helpers can share the file.
"""

import base64
import binascii
import hashlib
import hmac

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from goodfetch_test_support import webhook_signatures


def _signed(identifier: str, timestamp: str, body: str) -> bytes:
    return f"{identifier}.{timestamp}.{body}".encode()


def _digest(endpoint: str | None, signed: bytes) -> bytes:
    """The SHA-256 a `v2a` signature covers: the endpoint ID, then the signed text."""
    return hashlib.sha256(f"{endpoint}.".encode() + signed).digest()


def _raw(key: str, prefix: str) -> bytes:
    return base64.b64decode(key.removeprefix(prefix))


def _hmac(key: str, signed: bytes) -> bytes:
    return hmac.digest(_raw(key, "whsec_"), signed, "sha256")


def _signature(key: str, endpoint: str | None, signed: bytes) -> str:
    """One key's `webhook-signature` entry."""
    if key.startswith("whsk_"):
        private = Ed25519PrivateKey.from_private_bytes(_raw(key, "whsk_"))
        made = private.sign(_digest(endpoint, signed))
        return f"v2a,{base64.b64encode(made).decode()}"
    return f"v1,{base64.b64encode(_hmac(key, signed)).decode()}"


def _verifies(key: str, entry: str, endpoint: str | None, signed: bytes) -> bool:
    """Whether a trusted key verifies one entry of the same version."""
    version, _, encoded = entry.partition(",")
    try:
        received = base64.b64decode(encoded, validate=True)
    except binascii.Error:
        return False
    if key.startswith("whpk_"):
        if version != "v2a":
            return False
        public = Ed25519PublicKey.from_public_bytes(_raw(key, "whpk_"))
        try:
            public.verify(received, _digest(endpoint, signed))
        except InvalidSignature:
            return False
        return True
    return version == "v1" and hmac.compare_digest(received, _hmac(key, signed))


def test_the_vectors_hold_the_specifications_published_v1_and_v2a_ones() -> None:
    """The published vectors are the two that name their source."""
    vectors = webhook_signatures().signed

    published = [vector.name for vector in vectors if vector.source is not None]

    assert published == ["v1_published", "v2a_published"]


def test_each_signed_vector_gives_its_signatures_in_python() -> None:
    """HMAC-SHA256 and ed25519 give every signed vector's `webhook-signature`."""
    vectors = webhook_signatures().signed

    misses: list[str] = []
    for vector in vectors:
        signed = _signed(vector.id, str(vector.timestamp), vector.body)
        sent = " ".join(_signature(key, vector.endpoint, signed) for key in vector.keys)
        if sent != vector.signature:
            misses.append(f"{vector.name}: {sent}")

    assert misses == []


def test_a_case_verifies_under_a_trusted_key_exactly_when_it_is_verified() -> None:
    """A trusted key verifies a signature entry in exactly the verified cases."""
    cases = webhook_signatures().verifications

    misses: list[str] = []
    for case in cases:
        if case.outcome not in {"verified", "no_matching_signature"}:
            continue
        signed = _signed(case.id or "", case.timestamp or "", case.body)
        entries = (case.signature or "").split(" ")
        verified = any(
            _verifies(key, entry, case.endpoint, signed)
            for key in case.trusted
            for entry in entries
        )
        if verified != (case.outcome == "verified"):
            misses.append(case.name)

    assert misses == []
