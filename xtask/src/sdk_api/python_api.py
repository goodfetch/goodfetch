"""Prints how the Python SDK's packages break the previous release's API.

`xtask sdk-api` runs it on the repository's interpreter with one JSON
argument: `previous` and `current`, the folders of the two trees, and
`packages`, each package's source folder under a tree and its import name,
such as `["sdks/python/collie/src", "goodfetch_collie"]`. It prints a JSON
array, one object per breakage Griffe finds between the two trees' packages,
testing modules included, as `sdk_api.rs`'s `PythonBreak` reads it. Griffe
reads the sources and imports none of them, so reading the SDK runs none of
its code; each tree's packages are read together, so a name one package
re-exports from another resolves.

Each object carries:

- `module`, the module the broken object belongs to, such as
  `goodfetch_collie.testing`;
- `top`, the name of the module's object it is or belongs to, `(module)` for
  a module itself;
- `path`, the object's dotted path;
- `kind`, Griffe's kind of breakage, such as `Public object was removed`;
- `experimental`, whether the object, in either tree, is a function that
  calls `warn_experimental(`, the SDK's mark of an Experimental member.

A module with a `generated` segment is the generator's, which `xtask
api-check` judges, and is left out. A package the previous tree lacks has no
API to break; a package the current tree lacks is one breakage, its removal.
"""

from __future__ import annotations

import json
import logging
import sys
from pathlib import Path

import griffe

GENERATED = "generated"
MODULE = "(module)"
EXPERIMENTAL_CALL = "warn_experimental("


def load(tree: Path, packages: list[tuple[str, str]]) -> griffe.GriffeLoader:
    """Each package of `tree` that exists, read by one loader."""
    folders = [tree / folder for folder, _ in packages]
    loader = griffe.GriffeLoader(
        search_paths=[folder for folder in folders if folder.is_dir()],
        allow_inspection=False,
    )
    for folder, name in packages:
        if (tree / folder / name).is_dir():
            loader.load(name)
    loader.resolve_aliases(external=False, implicit=False)
    return loader


def find(loader: griffe.GriffeLoader, path: str) -> griffe.Object | None:
    """The object at `path`, or `None` when the loader has none."""
    try:
        found: griffe.Object | griffe.Alias = loader.modules_collection[path]
    except KeyError:
        return None
    if isinstance(found, griffe.Alias):
        try:
            return found.final_target
        except griffe.AliasResolutionError:
            return None
    return found


def is_experimental(obj: griffe.Object | None) -> bool:
    """Whether `obj` is a function that calls `warn_experimental(`."""
    if obj is None or not obj.is_function:
        return False
    try:
        return EXPERIMENTAL_CALL in obj.source
    except (OSError, ValueError):
        return False


def located(obj: griffe.Object | griffe.Alias) -> tuple[str, str]:
    """The module `obj` belongs to, and its top-level object's name there."""
    if obj.is_module:
        return obj.path, MODULE
    module = obj.module.path
    return module, obj.path.removeprefix(f"{module}.").split(".")[0]


def breakages(
    previous: Path, current: Path, packages: list[tuple[str, str]]
) -> list[dict[str, object]]:
    """Every breakage of `current`'s packages against `previous`'s."""
    old_loader = load(previous, packages)
    new_loader = load(current, packages)
    rows: list[dict[str, object]] = []
    for _, name in packages:
        old = find(old_loader, name)
        if old is None:
            continue
        new = find(new_loader, name)
        if new is None:
            rows.append(
                {
                    "module": name,
                    "top": MODULE,
                    "path": name,
                    "kind": griffe.BreakageKind.OBJECT_REMOVED.value,
                    "experimental": False,
                }
            )
            continue
        for breakage in griffe.find_breaking_changes(old, new):
            obj = breakage.obj
            if GENERATED in obj.path.split("."):
                continue
            module, top = located(obj)
            experimental = is_experimental(find(old_loader, obj.path)) or (
                is_experimental(find(new_loader, obj.path))
            )
            rows.append(
                {
                    "module": module,
                    "top": top,
                    "path": obj.path,
                    "kind": breakage.kind.value,
                    "experimental": experimental,
                }
            )
    return rows


if __name__ == "__main__":
    # Griffe logs what it cannot follow, such as an import from outside the
    # SDK; the comparison is what this prints.
    logging.getLogger("griffe").setLevel(logging.ERROR)
    request = json.loads(sys.argv[1])
    found = breakages(
        Path(request["previous"]),
        Path(request["current"]),
        [(folder, name) for folder, name in request["packages"]],
    )
    sys.stdout.write(json.dumps(found))
