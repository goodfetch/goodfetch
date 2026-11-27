"""The verification helper of a `webhook` channel's delivery (ADRs 0017, 0027 and 0030).

Pure, with no engine and no Connection, so a receiver calls it on whatever
server takes the call (issue 1391). It checks the Standard Webhooks headers,
`webhook-id`, `webhook-timestamp` and `webhook-signature`, against the
secrets given, the current one and the previous one while a secret rotates,
then reads the body as the `NotificationPayload` the engine sent. A call it
refuses raises `WebhookInvalidError`, whose details name the reason.

A `whsec_` secret verifies a `v1` signature, HMAC-SHA256 over
`<id>.<timestamp>.<body>`; a `whpk_` public key verifies a `v2a` one,
Ed25519 over the SHA-256 of `<endpoint>.<id>.<timestamp>.<body>`, for the
endpoint ID `endpoint` names, with the `cryptography` package the `ed25519`
extra brings. Each signature is compared in constant time, and an entry of a
version no secret has, or that is no Base64, matches nothing, as the
engine's reference verifier does. Rust's `webhook.rs` and TypeScript's
`webhook.ts` are its twins.
"""

from __future__ import annotations

import base64
import binascii
import hashlib
import hmac
import json
import math
import re
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import TYPE_CHECKING, Final, cast

from pydantic import ValidationError

from goodfetch_collie_client._framing import object_members
from goodfetch_collie_client.generated.errors import ProgramError, WebhookInvalidError
from goodfetch_collie_client.generated.models import (
    NotificationPayload,
    WebhookInvalidReason,
)
from goodfetch_collie_client.goodfetch_error import runtime_message

if TYPE_CHECKING:
    from collections.abc import Iterable, Sequence

    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

__all__ = ["verify_webhook"]

_TOLERANCE: Final = timedelta(minutes=5)
"""How far a delivery's timestamp may be from now, either way, by default."""

_FORMAT_MAJOR: Final = 0
"""The major of the Notification payload this SDK reads, every minor of it."""

_ID: Final = "webhook-id"
_TIMESTAMP: Final = "webhook-timestamp"
_SIGNATURE: Final = "webhook-signature"

_OPERATION: Final = "verify_webhook"
"""The name `invalid_params` names for a secret the helper cannot use."""

_FORMAT: Final = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")

_FIXES: Final[Mapping[WebhookInvalidReason, str]] = {
    WebhookInvalidReason.signature: (
        "Answer the call with an error; when every delivery fails, pass the"
        " channel's `webhook.secret`, and the previous one too while it rotates."
    ),
    WebhookInvalidReason.timestamp: (
        "Answer the call with an error; when every delivery fails, check the"
        " receiver's clock."
    ),
    WebhookInvalidReason.malformed: (
        "Answer the call with an error; when every delivery fails, update the"
        " SDK to one that reads the engine's payload."
    ),
    WebhookInvalidReason.missing_header: (
        "Pass the request's headers as received; a call without them is no"
        " `webhook` delivery."
    ),
}


def verify_webhook(
    body: bytes | str,
    headers: Mapping[str, str] | Iterable[tuple[str, str]],
    secret: str | Sequence[str],
    *,
    tolerance: timedelta = _TOLERANCE,
    endpoint: str | None = None,
    at: datetime | None = None,
) -> NotificationPayload:
    """Verifies that a call is a `webhook` channel's delivery, and reads its body.

    The call must be signed with one of `secret`'s secrets less than
    `tolerance` from now, either way.

    Args:
        body: The body as received, before any parsing; text in UTF-8.
        headers: The headers as received, a mapping or name and value pairs,
            whose names are matched in any case.
        secret: The channel's `webhook.secret`, or a list of the current and
            previous ones while it rotates; a `whpk_` public key verifies a
            `v2a` signature.
        tolerance: How far the delivery's timestamp may be from now, either
            way: 5 minutes by default.
        endpoint: The endpoint ID the channel's `whsk_` key signs for, which a
            `whpk_` secret needs; never sent with the call.
        at: The time to verify at in place of the system's, such as a test's.

    Returns:
        The Notification payload the engine sent.

    Raises:
        WebhookInvalidError: The call is no such delivery; its details name
            why.
        ProgramError: `invalid_params` for no secret, a secret that is neither
            a `whsec_` secret nor a `whpk_` public key, or a `whpk_` key with
            no endpoint.
    """
    trusted = _Trusted.read([secret] if isinstance(secret, str) else secret, endpoint)
    call = _Call.read(headers)
    if not (call.timestamp.isascii() and call.timestamp.isdigit()):
        raise _invalid(
            WebhookInvalidReason.timestamp,
            f"`{_TIMESTAMP}` is no Unix time in seconds",
        )
    now = math.floor((datetime.now(UTC) if at is None else at).timestamp())
    seconds = tolerance // timedelta(seconds=1)
    if abs(now - int(call.timestamp)) > seconds:
        raise _invalid(
            WebhookInvalidReason.timestamp,
            f"the call was signed more than {seconds} seconds from now:"
            " a delayed or replayed call",
        )
    raw = body.encode() if isinstance(body, str) else bytes(body)
    message = f"{call.id}.{call.timestamp}.".encode() + raw
    if not trusted.signed(message, call.signature):
        raise _invalid(
            WebhookInvalidReason.signature,
            f"no signature of `{_SIGNATURE}` matches a secret given",
        )
    return _payload_of(raw)


@dataclass(frozen=True)
class _Call:
    """The three Standard Webhooks headers of a call."""

    id: str
    timestamp: str
    signature: str

    @staticmethod
    def read(headers: Mapping[str, str] | Iterable[tuple[str, str]]) -> _Call:
        """The first value of each header, whatever its name's case."""
        found: dict[str, str] = {}
        pairs: Iterable[tuple[str, str]] = (
            cast("Mapping[str, str]", headers).items()
            if isinstance(headers, Mapping)
            else headers
        )
        for name, value in pairs:
            found.setdefault(name.lower(), value)

        def read(name: str) -> str:
            value = found.get(name)
            if value is None:
                raise _invalid(
                    WebhookInvalidReason.missing_header,
                    f"the call has no `{name}` header",
                )
            return value

        return _Call(read(_ID), read(_TIMESTAMP), read(_SIGNATURE))


@dataclass(frozen=True)
class _Trusted:
    """The secrets a call may be signed with, and the endpoint `v2a` signs."""

    v1: tuple[bytes, ...]
    """Each `whsec_` secret's bytes, which verify a `v1` signature."""
    v2a: tuple[Ed25519PublicKey, ...]
    """Each `whpk_` public key, which verifies a `v2a` signature."""
    endpoint: str

    @staticmethod
    def read(secrets: Sequence[str], endpoint: str | None) -> _Trusted:
        """Reads `secrets`; refuses a non-secret, or `whpk_` without `endpoint`."""
        if not secrets:
            raise _refused(
                "no secret was given",
                "Pass the channel's `webhook.secret`, and the previous one too"
                " while it rotates.",
            )
        v1: list[bytes] = []
        v2a: list[Ed25519PublicKey] = []
        for secret in secrets:
            if secret.startswith("whsec_"):
                v1.append(_secret_of(secret.removeprefix("whsec_")))
            else:
                v2a.append(_public_key_of(secret))
        if endpoint is not None and (not endpoint or "." in endpoint):
            raise _refused(
                "the endpoint ID is empty or holds a `.`",
                "Pass the endpoint ID the channel's `whsk_` key signs for.",
            )
        if endpoint is None and v2a:
            raise _refused(
                "a `whpk_` key verifies a `v2a` signature, which names an"
                " endpoint, and none was given",
                "Pass the endpoint ID the channel's `whsk_` key signs for.",
            )
        return _Trusted(tuple(v1), tuple(v2a), endpoint or "")

    def signed(self, message: bytes, signatures: str) -> bool:
        """Whether an entry of `signatures` is a key's signature of `message`.

        `message` is the ID and timestamp, followed by the body.
        """
        digest = hashlib.sha256(f"{self.endpoint}.".encode() + message).digest()
        for entry in signatures.split(" "):
            version, comma, encoded = entry.partition(",")
            signature = _base64(encoded) if comma else None
            if signature is None:
                continue
            if version == "v1" and any(
                hmac.compare_digest(
                    hmac.new(secret, message, hashlib.sha256).digest(), signature
                )
                for secret in self.v1
            ):
                return True
            if version == "v2a" and any(
                _verifies(key, signature, digest) for key in self.v2a
            ):
                return True
        return False


def _secret_of(encoded: str) -> bytes:
    """A `whsec_` secret's bytes; an empty one is refused, as WebCrypto refuses it."""
    secret = _base64(encoded)
    if not secret:
        raise _unreadable()
    return secret


def _public_key_of(secret: str) -> Ed25519PublicKey:
    """A `whpk_` public key, with the `cryptography` package."""
    key = _base64(secret.removeprefix("whpk_")) if secret.startswith("whpk_") else None
    if key is None or len(key) != 32:
        raise _unreadable()
    # `cryptography` is the `ed25519` extra's, which only a `whpk_` key
    # needs, so it is imported here and never with the module.
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PublicKey,
        )
    except ImportError:
        raise _refused(
            "a `whpk_` key needs the `cryptography` package, which is not installed",
            "Install `goodfetch-collie-client[ed25519]`, or pass the channel's"
            " `whsec_` secret.",
        ) from None
    try:
        return Ed25519PublicKey.from_public_bytes(key)
    except ValueError:
        raise _unreadable() from None


def _verifies(key: Ed25519PublicKey, signature: bytes, digest: bytes) -> bool:
    """Whether `signature` is `key`'s signature of `digest`."""
    from cryptography.exceptions import InvalidSignature  # As `_public_key_of`.

    if len(signature) != 64:
        return False
    try:
        key.verify(signature, digest)
    except InvalidSignature:
        return False
    return True


def _payload_of(body: bytes) -> NotificationPayload:
    """Reads the body as a Notification payload of the major this SDK reads."""
    try:
        value: object = json.loads(
            body.decode("utf-8"), parse_constant=_refuse_constant
        )
    except ValueError:
        raise _invalid(WebhookInvalidReason.malformed, "the body is no JSON") from None
    no_payload = _invalid(
        WebhookInvalidReason.malformed, "the body is no Notification payload"
    )
    members = object_members(value)
    if members is None:
        raise no_payload
    data = object_members(members.get("data"))
    form = members.get("format")
    if not (
        isinstance(members.get("type"), str)
        and isinstance(members.get("timestamp"), str)
        and isinstance(form, str)
        and data is not None
        and isinstance(data.get("event_id"), str)
    ):
        raise no_payload
    version = _FORMAT.fullmatch(form)
    if version is None:
        raise no_payload
    if int(version[1]) != _FORMAT_MAJOR:
        raise _invalid(
            WebhookInvalidReason.malformed,
            f"the body's `format` is {form}, a major this SDK does not read",
        )
    try:
        return NotificationPayload.model_validate(value)
    except ValidationError:
        raise no_payload from None


def _refuse_constant(name: str) -> object:
    # JSON has no NaN or Infinity, which Python's parser takes by default.
    message = f"{name} is no JSON value"
    raise ValueError(message)


def _base64(text: str) -> bytes | None:
    """The bytes of `text`, strict Base64 in its canonical form; `None` otherwise."""
    try:
        decoded = base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError):
        return None
    # A non-canonical last character, whose unused bits are not zero, reads
    # as another text; the engine's Base64 refuses it too.
    return decoded if base64.b64encode(decoded).decode() == text else None


def _unreadable() -> ProgramError:
    """The refusal of a secret that is neither a `whsec_` secret nor a `whpk_` key."""
    return _refused(
        "a secret is neither a `whsec_` secret nor a `whpk_` public key in Base64",
        "Pass the channel's `webhook.secret` as the engine has it, or the"
        " `whpk_` public key of its `whsk_` key.",
    )


def _refused(reason: str, fix: str) -> ProgramError:
    """A call refused before it is checked, as a raw call's params are."""
    return ProgramError(
        f"{_OPERATION}: {reason}",
        code="invalid_params",
        group="program",
        retryable=False,
        fix=fix,
        details={"operation": _OPERATION},
    )


def _invalid(reason: WebhookInvalidReason, what_failed: str) -> WebhookInvalidError:
    """`webhook_invalid` for `reason`, saying what failed."""
    fix = _FIXES[reason]
    return WebhookInvalidError(
        runtime_message(
            f"{what_failed}, so it is no verified `webhook` delivery", fix=fix
        ),
        fix=fix,
        details={"reason": reason.value},
    )
