"""The verification helper through its interface (issue 1391).

The shared Standard Webhooks vectors, which the Rust and TypeScript helpers
pass too, and Notification payloads this test signs as the engine does.
TypeScript's `webhook.test.ts` is its twin.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
from datetime import UTC, datetime, timedelta
from typing import TYPE_CHECKING

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from goodfetch_collie_client.generated.errors import ProgramError, WebhookInvalidError
from goodfetch_collie_client.generated.models import (
    FullNotification,
    NotificationPayload,
    ThinNotification,
    WebhookInvalidReason,
)
from goodfetch_collie_client.webhook import verify_webhook
from goodfetch_test_support.webhook_signatures import (
    SignedVector,
    VerificationCase,
    webhook_signatures,
)

if TYPE_CHECKING:
    from collections.abc import Callable

VECTORS = webhook_signatures()

SIGNED_AT = 1_767_603_600
"""When the test deliveries are signed, in Unix seconds."""
EVENT_ID = "evt_01JDQ8X2Y5"


def signed_vector(name: str) -> SignedVector:
    """The signed vector `name`."""
    return next(vector for vector in VECTORS.signed if vector.name == name)


def verification(name: str) -> VerificationCase:
    """The verification case `name`."""
    return next(case for case in VECTORS.verifications if case.name == name)


def at(seconds: int) -> datetime:
    """`seconds` after the Unix epoch."""
    return datetime.fromtimestamp(seconds, UTC)


def v1(secret: str) -> Callable[[bytes], str]:
    """A signer of `v1` signatures with the `whsec_` secret `secret`."""
    key = base64.b64decode(secret.removeprefix("whsec_"))

    def sign(message: bytes) -> str:
        digest = hmac.new(key, message, hashlib.sha256).digest()
        return f"v1,{base64.b64encode(digest).decode()}"

    return sign


def v2a(signing: str, endpoint: str) -> Callable[[bytes], str]:
    """A signer of `v2a` signatures for `endpoint` with the `whsk_` key `signing`."""
    key = Ed25519PrivateKey.from_private_bytes(
        base64.b64decode(signing.removeprefix("whsk_"))
    )

    def sign(message: bytes) -> str:
        digest = hashlib.sha256(f"{endpoint}.".encode() + message).digest()
        return f"v2a,{base64.b64encode(key.sign(digest)).decode()}"

    return sign


def signed(body: str, *signers: Callable[[bytes], str]) -> dict[str, str]:
    """The headers of `body`'s delivery, signed by each of `signers` at `SIGNED_AT`."""
    message = f"{EVENT_ID}.{SIGNED_AT}.{body}".encode()
    return {
        "webhook-id": EVENT_ID,
        "webhook-timestamp": str(SIGNED_AT),
        "webhook-signature": " ".join(sign(message) for sign in signers),
        "content-type": "application/json",
    }


FULL_DATA: dict[str, object] = {
    "event_id": EVENT_ID,
    "event": "run_failed",
    "run_id": "nightly-prices",
    "schedule_id": None,
    "product": "collie",
    "title": "nightly-prices failed",
    "text": "The Run ended on an error.",
    "link": "http://127.0.0.1:7777/runs/nightly-prices",
}
FULL: dict[str, object] = {
    "type": "notification.run_failed",
    "timestamp": "2026-01-05T08:59:58Z",
    "format": "0.1",
    "data": FULL_DATA,
}
"""A full Notification payload of a failed Run, as the engine writes it."""

CURRENT, PREVIOUS = signed_vector("rotation_two_v1_keys").keys
UNKNOWN = "whsec_c2VjcmV0IG9mIGFub3RoZXIgY2hhbm5lbCEhISE="
"""A secret of no channel this test signs for."""

EXPECTED: dict[str, WebhookInvalidReason] = {
    # A vector's body is no Notification payload: one whose signature
    # verifies is refused only once its body is read.
    "verified": WebhookInvalidReason.malformed,
    "no_matching_signature": WebhookInvalidReason.signature,
    "timestamp_too_old": WebhookInvalidReason.timestamp,
    "timestamp_too_new": WebhookInvalidReason.timestamp,
    "timestamp_invalid": WebhookInvalidReason.timestamp,
    "missing_header": WebhookInvalidReason.missing_header,
}


def reason_of(error: pytest.ExceptionInfo[WebhookInvalidError]) -> object:
    """The reason `error` names, after checking it is `webhook_invalid`."""
    assert error.value.code == "webhook_invalid"
    assert error.value.group == "program"
    assert error.value.retryable is False
    assert error.value.typed_details is not None
    return error.value.typed_details.reason


def assert_full(verified: NotificationPayload) -> None:
    """`verified` is `FULL`, read."""
    assert verified.model_dump(mode="json", exclude_none=True) == {
        **FULL,
        "data": {key: value for key, value in FULL_DATA.items() if value is not None},
    }
    assert verified.timestamp == datetime(2026, 1, 5, 8, 59, 58, tzinfo=UTC)
    assert isinstance(verified.data.root, FullNotification)


@pytest.mark.covers("story:11-271", "error:webhook_invalid")
@pytest.mark.parametrize(
    "vector", VECTORS.verifications, ids=[each.name for each in VECTORS.verifications]
)
def test_refuses_each_vector_for_its_reason_or_passes_its_signature(
    vector: VerificationCase,
) -> None:
    """Each vector fails for its outcome's reason: a verified one for its body."""
    headers = {
        name: value
        for name, value in (
            ("webhook-id", vector.id),
            ("webhook-timestamp", vector.timestamp),
            ("webhook-signature", vector.signature),
        )
        if value is not None
    }

    with pytest.raises(WebhookInvalidError) as error:
        verify_webhook(
            vector.body,
            headers,
            vector.trusted,
            endpoint=vector.endpoint,
            at=at(vector.now),
        )

    assert reason_of(error) == EXPECTED[vector.outcome]


@pytest.mark.covers("story:11-271")
def test_reads_back_a_signed_full_payload() -> None:
    """A signed full payload reads back, its time in UTC."""
    body = json.dumps(FULL)
    headers = signed(body, v1(CURRENT))

    verified = verify_webhook(body, headers, CURRENT, at=at(SIGNED_AT + 1))

    assert_full(verified)
    assert verified.format.root == "0.1"


@pytest.mark.covers("story:11-271")
def test_reads_back_a_signed_thin_payload() -> None:
    """A thin payload reads back with its IDs only."""
    body = json.dumps(
        {**FULL, "data": {"event_id": EVENT_ID, "run_id": "nightly-prices"}}
    )
    headers = signed(body, v1(CURRENT))

    verified = verify_webhook(body, headers, CURRENT, at=at(SIGNED_AT))

    assert isinstance(verified.data.root, ThinNotification)
    assert verified.data.root.event_id == EVENT_ID


@pytest.mark.covers("story:11-271")
def test_reads_a_body_given_as_bytes_and_headers_given_as_pairs() -> None:
    """Bytes and a list of name and value pairs read as text and a mapping do."""
    body = json.dumps(FULL)
    headers = list(signed(body, v1(CURRENT)).items())

    verified = verify_webhook(body.encode(), headers, CURRENT, at=at(SIGNED_AT))

    assert_full(verified)


@pytest.mark.covers("story:11-271")
@pytest.mark.parametrize("trusted", [CURRENT, PREVIOUS], ids=["current", "previous"])
def test_accepts_a_rotation_signed_with_both_secrets(trusted: str) -> None:
    """While a secret rotates, either secret verifies, beside one of no channel."""
    body = json.dumps(FULL)
    headers = signed(body, v1(CURRENT), v1(PREVIOUS))

    assert_full(verify_webhook(body, headers, trusted, at=at(SIGNED_AT)))
    assert_full(verify_webhook(body, headers, [UNKNOWN, trusted], at=at(SIGNED_AT)))


@pytest.mark.covers("story:11-271")
def test_verifies_a_v2a_signature_with_the_public_key_for_its_endpoint_only() -> None:
    """A `whpk_` key verifies the `v2a` signature made for its endpoint, no other."""
    signing = signed_vector("v2a_published")
    (public_key,) = verification("v2a_published").trusted
    endpoint = signing.endpoint or ""
    body = json.dumps(FULL)
    headers = signed(body, v2a(signing.keys[0], endpoint))

    verified = verify_webhook(
        body, headers, public_key, endpoint=endpoint, at=at(SIGNED_AT)
    )
    with pytest.raises(WebhookInvalidError) as elsewhere:
        verify_webhook(
            body, headers, public_key, endpoint="ep_another", at=at(SIGNED_AT)
        )

    assert_full(verified)
    assert reason_of(elsewhere) == WebhookInvalidReason.signature


@pytest.mark.covers("story:11-271", "error:webhook_invalid")
@pytest.mark.parametrize(
    ("forged", "now", "expected"),
    [
        (
            json.dumps({"type": "notification.run_finished"}),
            SIGNED_AT,
            WebhookInvalidReason.signature,
        ),
        (None, SIGNED_AT + 6 * 60, WebhookInvalidReason.timestamp),
        (None, SIGNED_AT - 6 * 60, WebhookInvalidReason.timestamp),
    ],
    ids=["a forged body", "a call replayed six minutes later", "a call dated ahead"],
)
def test_refuses_a_forged_or_replayed_call(
    forged: str | None, now: int, expected: WebhookInvalidReason
) -> None:
    """A body the signature is not of, or a call outside the tolerance, fails."""
    body = json.dumps(FULL)
    headers = signed(body, v1(CURRENT))

    with pytest.raises(WebhookInvalidError) as error:
        verify_webhook(forged or body, headers, CURRENT, at=at(now))

    assert reason_of(error) == expected


@pytest.mark.covers("story:11-271")
def test_accepts_an_older_call_within_a_wider_tolerance() -> None:
    """A tolerance of ten minutes accepts a call six minutes old."""
    body = json.dumps(FULL)
    headers = signed(body, v1(CURRENT))

    verified = verify_webhook(
        body,
        headers,
        CURRENT,
        tolerance=timedelta(minutes=10),
        at=at(SIGNED_AT + 6 * 60),
    )

    assert_full(verified)


@pytest.mark.covers("story:11-271")
def test_reads_a_newer_minor_and_keeps_an_unknown_event_as_received() -> None:
    """A newer minor's fields and an event this SDK does not know read as received."""
    body = json.dumps(
        {
            "type": "notification.crawl_drained",
            "timestamp": "2026-01-05T09:00:00Z",
            "format": "0.7",
            "data": {
                "event_id": EVENT_ID,
                "event": "crawl_drained",
                "run_id": "nightly-prices",
                "product": "foxhound",
                "title": "nightly-prices drained",
                "text": "The crawl drained.",
                "severity": "info",
            },
            "delivery_attempt": 2,
        }
    )
    headers = signed(body, v1(CURRENT))

    verified = verify_webhook(body, headers, CURRENT, at=at(SIGNED_AT))

    assert verified.type == "notification.crawl_drained"
    assert verified.format.root == "0.7"
    assert isinstance(verified.data.root, FullNotification)
    assert verified.data.root.event == "crawl_drained"


@pytest.mark.covers("story:11-271", "error:webhook_invalid")
@pytest.mark.parametrize(
    "body",
    [json.dumps({**FULL, "format": "1.0"}), '{"test": 2432232314}', "ok"],
    ids=["another major", "no payload", "no JSON"],
)
def test_refuses_a_body_that_is_no_payload_this_sdk_reads_as_malformed(
    body: str,
) -> None:
    """A signed body of another major, or no payload, is malformed."""
    headers = signed(body, v1(CURRENT))

    with pytest.raises(WebhookInvalidError) as error:
        verify_webhook(body, headers, CURRENT, at=at(SIGNED_AT))

    assert reason_of(error) == WebhookInvalidReason.malformed


@pytest.mark.covers("story:11-271")
def test_matches_header_names_in_any_case() -> None:
    """Header names are matched in any case."""
    body = json.dumps(FULL)
    headers = {name.upper(): value for name, value in signed(body, v1(CURRENT)).items()}

    assert_full(verify_webhook(body, headers, CURRENT, at=at(SIGNED_AT)))


@pytest.mark.covers("story:11-271")
def test_refuses_a_call_signed_long_ago_by_the_systems_clock() -> None:
    """Without `at`, the system's clock is the time verified at."""
    body = json.dumps(FULL)
    headers = signed(body, v1(CURRENT))

    with pytest.raises(WebhookInvalidError) as error:
        verify_webhook(body, headers, CURRENT)

    assert reason_of(error) == WebhookInvalidReason.timestamp


@pytest.mark.covers("story:11-271")
@pytest.mark.parametrize(
    ("secrets", "endpoint"),
    [
        ([], None),
        (["MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"], None),
        (["whsec_"], None),
        (["whsk_VQMJMCF2TCBiITJzbZB7z+0sHSkvbmqQaNgo84ExILU="], "ep_1"),
        (["whpk_2zPwzAhd9xyyTX6hydEXQjvUWwvpqA/DmAUXdzvZGyU="], None),
        (["whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"], "ep.1"),
    ],
    ids=[
        "no secret",
        "no prefix",
        "an empty secret",
        "a signing key",
        "a public key with no endpoint",
        "an endpoint with a dot",
    ],
)
def test_refuses_an_unusable_secret_before_reading_the_call(
    secrets: list[str], endpoint: str | None
) -> None:
    """A secret the helper cannot use is the program's error, never the call's."""
    with pytest.raises(ProgramError) as error:
        verify_webhook("", {}, secrets, endpoint=endpoint)

    assert not isinstance(error.value, WebhookInvalidError)
    assert error.value.code == "invalid_params"
    assert error.value.details == {"operation": "verify_webhook"}
