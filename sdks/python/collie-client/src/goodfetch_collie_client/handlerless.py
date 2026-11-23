"""What a handler-less start runs (ADRs 0036 and 0102), with no handler in the program.

`start_catalog_run` takes one of Collie's code-free targets, whose verbs come
from the catalog: an Extraction of a page by a schema, `ExtractTarget`, an
Agent task, `AgentTarget`, or a Plan replayed with no model call,
`PlanReplayTarget`. Each is a frozen dataclass, matched by class, that the
SDK sends as the wire's `{kind, …}` object, a field left `None` left out.
A schema is a JSON Schema written by hand (ADR 0082) and an Agent task's
Success check a JSON value, each sent as written. TypeScript's
`CatalogTarget` union is its twin.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeAlias

if TYPE_CHECKING:
    from goodfetch_collie_client.json_values import JSONValue
    from goodfetch_collie_client.schema import JSONSchema

__all__ = ["AgentTarget", "CatalogTarget", "ExtractTarget", "PlanReplayTarget"]


@dataclass(frozen=True, kw_only=True)
class ExtractTarget:
    """An Extraction of one page by a schema, the `extract` kind."""

    url: str
    """The page."""
    schema: JSONSchema
    """The JSON Schema of what it extracts."""
    instruction: str | None = None
    """What to extract, in words, beside the schema."""


@dataclass(frozen=True, kw_only=True)
class AgentTarget:
    """An Agent task from a page (ADR 0091), the `agent` kind."""

    url: str
    """The page it starts from."""
    task: str
    """The goal, in words."""
    expect: JSONValue | None = None
    """Its Success check, which decides `verified`."""
    schema: JSONSchema | None = None
    """The JSON Schema of what it answers."""
    max_steps: int | None = None
    """The most steps it takes."""


@dataclass(frozen=True, kw_only=True)
class PlanReplayTarget:
    """A Plan replayed with no model call (ADR 0091), the `plan_replay` kind."""

    plan: str
    """The Plan file, on the program's machine, relative to its working directory."""


CatalogTarget: TypeAlias = ExtractTarget | AgentTarget | PlanReplayTarget
"""What a code-free catalog run does (ADR 0036): nothing in the program serves it."""


def target_on_wire(target: CatalogTarget) -> dict[str, object]:
    """`target` as the wire takes it: its `kind` and each field given."""
    match target:
        case ExtractTarget():
            fields: dict[str, object] = {
                "kind": "extract",
                "url": target.url,
                "schema": target.schema,
                "instruction": target.instruction,
            }
        case AgentTarget():
            fields = {
                "kind": "agent",
                "url": target.url,
                "task": target.task,
                "expect": target.expect,
                "schema": target.schema,
                "max_steps": target.max_steps,
            }
        case PlanReplayTarget():
            fields = {"kind": "plan_replay", "plan": target.plan}
    return {name: value for name, value in fields.items() if value is not None}
