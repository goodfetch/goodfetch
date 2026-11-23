"""Prints how the Python SDK's packages break the previous release's API.

`xtask sdk-api` runs it on the repository's interpreter with one JSON
argument: `previous` and `current`, the folders of the two trees,
`packages`, each package's source folder under a tree and its import name,
such as `["sdks/python/collie/src", "goodfetch_collie"]`, and
`experimental_line`, the docstring line that marks an Experimental object
(ADR 0099), the generator's own text. It prints a JSON
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
- `experimental`, whether the object, or a class that holds it, is
  Experimental in either tree: a function that calls `warn_experimental(`,
  the SDK's run-time mark, or an object whose docstring has
  `experimental_line`, as the generated Plugins' handlers have alone.

The generated layer's modules are read as the rest are: they are public API
a program imports. A package the previous tree lacks has no API to break; a
package the current tree lacks is one breakage, its removal.
"""

from __future__ import annotations

import json
import logging
import sys
from pathlib import Path

import griffe

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


def is_experimental(obj: griffe.Object | None, line: str) -> bool:
    """Whether `obj`, or a class that holds it, is Experimental.

    That is a function that calls `warn_experimental(`, or an object whose
    docstring has `line`, the one ADR 0099 gives Python.
    """
    while obj is not None and not obj.is_module:
        docstring = obj.docstring.value if obj.docstring is not None else ""
        if any(text.strip() == line for text in docstring.splitlines()):
            return True
        if obj.is_function and calls_warn_experimental(obj):
            return True
        obj = obj.parent
    return False


def calls_warn_experimental(function: griffe.Object) -> bool:
    """Whether `function`'s source calls `warn_experimental(`."""
    try:
        return EXPERIMENTAL_CALL in function.source
    except (OSError, ValueError):
        return False


def located(obj: griffe.Object | griffe.Alias) -> tuple[str, str]:
    """The module `obj` belongs to, and its top-level object's name there."""
    if obj.is_module:
        return obj.path, MODULE
    module = obj.module.path
    return module, obj.path.removeprefix(f"{module}.").split(".")[0]


def breakages(
    previous: Path, current: Path, packages: list[tuple[str, str]], line: str
) -> list[dict[str, object]]:
    """Every breakage of `current`'s packages against `previous`'s.

    `line` is the docstring line that marks an Experimental object.
    """
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
            module, top = located(obj)
            experimental = is_experimental(find(old_loader, obj.path), line) or (
                is_experimental(find(new_loader, obj.path), line)
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
        request["experimental_line"],
    )
    sys.stdout.write(json.dumps(found))
