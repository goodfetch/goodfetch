"""The base of an open tagged union's `unknown` member (ADR 0079).

A member of an open tagged union this package does not know, which a newer
engine may send, reads as the union's `unknown` member: its tag field says
`unknown`, `tag` keeps the tag the engine sent, and what else it carries is
kept as sent in `model_extra`. Each union's member is generated on this base,
which `caller.py` builds such a member as. Hand-written, since no catalog entry
shapes it.
"""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict


class UnknownMember(BaseModel):
    """A member of an open tagged union this package does not know."""

    model_config = ConfigDict(extra="allow")
    tag: str
    """The tag the engine sent."""
