"""The shared Standard Webhooks vectors (ADR 0017).

They are read from `test-support/fixtures/webhook-signatures.json`, the one
file Rust and TypeScript read too, so the engine's `webhook_signing` and the
SDKs' verification helpers sign and verify alike.
"""

from dataclasses import dataclass
from typing import Any, cast

from goodfetch_test_support.schemas import shared_fixture


@dataclass(frozen=True)
class SignedVector:
    """A message, the keys that sign it and the `webhook-signature` they give."""

    name: str
    """The case's name."""
    source: str | None
    """Where the vector is published, for one the specification publishes."""
    keys: list[str]
    """The signing keys, current first: `whsec_` or `whsk_`."""
    endpoint: str | None
    """The endpoint ID a `v2a` key signs for."""
    id: str
    """`webhook-id`."""
    timestamp: int
    """`webhook-timestamp`, in Unix seconds."""
    body: str
    """The body, as sent."""
    signature: str
    """`webhook-signature`: each key's signature, in order."""


@dataclass(frozen=True)
class VerificationCase:
    """A delivery as received, the keys trusted and the verifier's answer."""

    name: str
    """The case's name."""
    trusted: list[str]
    """The trusted keys: `whsec_` or `whpk_`."""
    endpoint: str | None
    """The endpoint ID a `v2a` signature is made for."""
    id: str | None
    """`webhook-id`, `None` when missing."""
    timestamp: str | None
    """`webhook-timestamp` as received, `None` when missing."""
    signature: str | None
    """`webhook-signature`, `None` when missing."""
    body: str
    """The body, as received."""
    now: int
    """The receiver's clock, in Unix seconds."""
    outcome: str
    """The verifier's answer: `verified`, `missing_header`,
    `timestamp_invalid`, `timestamp_too_old`, `timestamp_too_new` or
    `no_matching_signature`."""
    run_epoch: int | None = None
    """The run epoch a verified delivery's body gives, when the case reads it."""


@dataclass(frozen=True)
class WebhookSignatures:
    """The committed vectors."""

    signed: list[SignedVector]
    """What a signer sends."""
    verifications: list[VerificationCase]
    """What the verifier answers."""


def webhook_signatures() -> WebhookSignatures:
    """The committed vectors.

    Returns:
        The specification's published `v1` and `v2a` vectors, Goodfetch's
        rotations, and the verification cases.
    """
    document = cast(
        "dict[str, list[dict[str, object]]]",
        shared_fixture("webhook-signatures.json", "webhook-signatures.schema.json"),
    )
    return WebhookSignatures(
        [
            SignedVector(**cast("dict[str, Any]", vector))
            for vector in document["signed"]
        ],
        [
            VerificationCase(**cast("dict[str, Any]", case))
            for case in document["verifications"]
        ],
    )
