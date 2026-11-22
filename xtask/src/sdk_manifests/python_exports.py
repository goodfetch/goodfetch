"""Prints the members the Python packages of an SDK's folder export.

`xtask sdk-exports` runs it on the repository's interpreter with the SDK's
folder, `sdks/python`, and reads the JSON array it prints: one object per
member an entry exports, as `exports.rs`'s `Exported` reads it. It parses
each package's source with `ast` and imports none of it, so reading the SDK
runs none of its code.

A package is each `<folder>/src/<name>/__init__.py` of the SDK's folder; test
code, outside `src`, is never read. An entry is a module of a package whose
dotted path has no `generated` segment and no segment starting with `_`, and
that declares a literal `__all__`: the names a program imports, as PEP 8 and
Griffe take `__all__` to be when a module declares it. Each name of an
entry's `__all__` is a member of `(module)`, and each public member of a
class it names is a member of that name:

- a member is a function, a class, or a name a class body or its
  `__init__`'s `self.<name>` assigns, whose name has no leading `_`;
- a class's members are its own and those of each base the SDK declares, a
  base outside it passed over, so an inherited member is one declaration
  however many classes inherit it;
- a class with no public method is a shape of data, and its fields are not
  members;
- a dunder is the language's own protocol, as `__enter__` is: a line may
  name it and none has to.

A member is generated when the module declaring it has a `generated`
segment, and the language library's when it comes from outside the SDK.
Each carries its docstring, `doc`, and its decorators' names, `markers`,
such as PEP 702's `deprecated`.
An `__all__` declared other than as one list of strings, or naming what
its module neither defines nor imports, fails the read: its message goes
to stderr and the exit code is 1.
"""

from __future__ import annotations

import ast
import json
import sys
from dataclasses import dataclass
from pathlib import Path

GENERATED = "generated"
MODULE = "(module)"
# The longest chain of re-exports a name is followed through.
HOPS = 32


class ReadError(Exception):
    """A package's source the reader cannot read its exports from."""


@dataclass(frozen=True)
class Module:
    """One module of a package, parsed."""

    name: str
    package: str
    generated: bool
    is_package: bool
    tree: ast.Module


@dataclass(frozen=True)
class Declared:
    """A name the module `module` declares, `node` its statement."""

    module: Module
    name: str
    node: ast.stmt | None


@dataclass(frozen=True)
class Library:
    """A name declared outside the SDK, as `module.name`."""

    module: str
    name: str


def read_modules(sdk: Path) -> dict[str, Module]:
    """Each module of each package of the SDK's folder, by dotted name."""
    modules: dict[str, Module] = {}
    for init in sorted(sdk.glob("*/src/*/__init__.py")):
        source = init.parent.parent
        for path in sorted(init.parent.rglob("*.py")):
            parts = path.relative_to(source).with_suffix("").parts
            is_package = parts[-1] == "__init__"
            if is_package:
                parts = parts[:-1]
            modules[".".join(parts)] = Module(
                name=".".join(parts),
                package=parts[0],
                generated=GENERATED in parts,
                is_package=is_package,
                tree=ast.parse(path.read_text(encoding="utf-8"), filename=str(path)),
            )
    return modules


def statements(body: list[ast.stmt]) -> list[ast.stmt]:
    """The statements of a module's top level, through `if` and `try`."""
    found: list[ast.stmt] = []
    for node in body:
        if isinstance(node, ast.If):
            found += statements(node.body) + statements(node.orelse)
        elif isinstance(node, ast.Try):
            found += statements(node.body) + statements(node.orelse)
            for handler in node.handlers:
                found += statements(handler.body)
            found += statements(node.finalbody)
        else:
            found.append(node)
    return found


def assigned(node: ast.stmt) -> list[str]:
    """The plain names an assignment binds."""
    if isinstance(node, ast.Assign):
        return [target.id for target in node.targets if isinstance(target, ast.Name)]
    if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
        return [node.target.id]
    if isinstance(node, ast.TypeAlias):
        return [node.name.id]
    return []


def definitions(module: Module) -> dict[str, ast.stmt]:
    """The names a module's top level defines."""
    defined: dict[str, ast.stmt] = {}
    for node in statements(module.tree.body):
        if isinstance(node, ast.ClassDef | ast.FunctionDef | ast.AsyncFunctionDef):
            defined.setdefault(node.name, node)
        for name in assigned(node):
            defined.setdefault(name, node)
    return defined


def imported(module: Module) -> dict[str, tuple[str, str | None]]:
    """Each name a module imports: the module it names, and the name."""
    names: dict[str, tuple[str, str | None]] = {}
    for node in statements(module.tree.body):
        if isinstance(node, ast.ImportFrom):
            source = absolute(module, node.module, node.level)
            for alias in node.names:
                names[alias.asname or alias.name] = (source, alias.name)
        elif isinstance(node, ast.Import):
            for alias in node.names:
                if alias.asname is not None:
                    names[alias.asname] = (alias.name, None)
    return names


def absolute(module: Module, name: str | None, level: int) -> str:
    """The dotted name a relative import names."""
    if level == 0:
        return name or ""
    parts = module.name.split(".")
    if not module.is_package:
        parts = parts[:-1]
    parts = parts[: len(parts) - (level - 1)]
    return ".".join([*parts, name] if name else parts)


def exported_names(module: Module) -> list[str] | None:
    """A module's literal `__all__`, or `None` when it declares none.

    An `__all__` built any other way, extended, or holding anything but
    strings is refused: the reader would miss the names it adds.
    """
    names: list[str] | None = None
    for node in statements(module.tree.body):
        extended = (
            isinstance(node, ast.AugAssign)
            and isinstance(node.target, ast.Name)
            and node.target.id == "__all__"
        )
        if not extended and "__all__" not in assigned(node):
            continue
        value = node.value if isinstance(node, ast.Assign | ast.AnnAssign) else None
        if extended or names is not None or not isinstance(value, ast.List | ast.Tuple):
            raise ReadError(unreadable(module))
        names = []
        for element in value.elts:
            if not isinstance(element, ast.Constant) or not isinstance(
                element.value, str
            ):
                raise ReadError(unreadable(module))
            names.append(element.value)
    return names


def unreadable(module: Module) -> str:
    """Why the reader refuses the `__all__` of `module`."""
    return f"`{module.name}` declares `__all__` other than as one list of strings"


class Reader:
    """Resolves the names the SDK's modules re-export."""

    def __init__(self, modules: dict[str, Module]) -> None:
        """Holds the SDK's modules and what each defines and imports."""
        self.modules = modules
        self.defined = {name: definitions(module) for name, module in modules.items()}
        self.imports = {name: imported(module) for name, module in modules.items()}

    def resolve(self, module: Module, name: str) -> Declared | Library:
        """Where the name `name` of `module` is declared."""
        for _ in range(HOPS):
            node = self.defined[module.name].get(name)
            if node is not None:
                return Declared(module, name, node)
            source = self.imports[module.name].get(name)
            if source is None:
                return Declared(module, name, None)
            target, original = source
            if original is None:
                # `import x.y as z` names the module itself.
                if target in self.modules:
                    return Declared(self.modules[target], target, None)
                return Library(target, "")
            if target not in self.modules:
                return Library(target, original)
            known = self.defined[target].keys() | self.imports[target].keys()
            if original not in known and f"{target}.{original}" in self.modules:
                submodule = f"{target}.{original}"
                return Declared(self.modules[submodule], submodule, None)
            module, name = self.modules[target], original
        message = f"`{module.name}.{name}` is re-exported more than {HOPS} times"
        raise ReadError(message)

    def bases(self, declared: Declared) -> list[Declared]:
        """The class `declared` and each base the SDK declares, nearest first.

        A base outside the SDK, such as `Generic[T]` or `Exception`, or one
        the reader cannot name, is passed over: its members are its
        library's.
        """
        order: list[Declared] = []
        pending = [declared]
        while pending:
            current = pending.pop(0)
            if not isinstance(current.node, ast.ClassDef) or any(
                seen.module is current.module and seen.name == current.name
                for seen in order
            ):
                continue
            order.append(current)
            for base in current.node.bases:
                expression = base.value if isinstance(base, ast.Subscript) else base
                if isinstance(expression, ast.Name):
                    found = self.resolve(current.module, expression.id)
                    if isinstance(found, Declared):
                        pending.append(found)
        return order


def is_dunder(name: str) -> bool:
    """Whether `name` is one of the language's protocol names."""
    return len(name) > 4 and name.startswith("__") and name.endswith("__")


def is_method(node: ast.stmt) -> bool:
    """Whether a class body's statement is a public method."""
    if not isinstance(node, ast.FunctionDef | ast.AsyncFunctionDef):
        return False
    return not node.name.startswith("_")


def body_members(node: ast.ClassDef) -> list[tuple[str, bool]]:
    """A class's own members, each with whether it is a method."""
    members: list[tuple[str, bool]] = []
    for statement in node.body:
        if isinstance(statement, ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef):
            members.append((statement.name, not isinstance(statement, ast.ClassDef)))
            if isinstance(statement, ast.FunctionDef) and statement.name == "__init__":
                members += [(name, False) for name in instance_fields(statement)]
        else:
            members += [(name, False) for name in assigned(statement)]
    return members


def instance_fields(init: ast.FunctionDef) -> list[str]:
    """The names `__init__` assigns on `self`."""
    if not init.args.args:
        return []
    this = init.args.args[0].arg
    fields: list[str] = []
    for node in ast.walk(init):
        targets: list[ast.expr] = []
        if isinstance(node, ast.Assign):
            targets = node.targets
        elif isinstance(node, ast.AnnAssign):
            targets = [node.target]
        fields += [
            target.attr
            for target in targets
            if isinstance(target, ast.Attribute)
            and isinstance(target.value, ast.Name)
            and target.value.id == this
        ]
    return fields


def decorator_name(decorator: ast.expr) -> str | None:
    """The last name of a decorator, `deprecated` for `@warnings.deprecated(…)`."""
    expression = decorator.func if isinstance(decorator, ast.Call) else decorator
    if isinstance(expression, ast.Name):
        return expression.id
    if isinstance(expression, ast.Attribute):
        return expression.attr
    return None


def documentation(nodes: list[ast.stmt]) -> tuple[str, list[str]]:
    """The docstring and decorators' names of a member's statements.

    An overloaded function's statements are its `@overload` stubs and its
    implementation: the first docstring among them is its doc, and every
    decorator of any of them is its marker, as PEP 702's `@deprecated` is.
    """
    doc = ""
    markers: list[str] = []
    for node in nodes:
        if not isinstance(node, ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef):
            continue
        doc = doc or (ast.get_docstring(node) or "")
        for decorator in node.decorator_list:
            name = decorator_name(decorator)
            if name is not None and name not in markers:
                markers.append(name)
    return doc, markers


def named(body: list[ast.stmt], name: str) -> list[ast.stmt]:
    """The functions and classes of `body` named `name`."""
    return [
        node
        for node in body
        if isinstance(node, ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef)
        and node.name == name
    ]


def row(
    entry: Module,
    receiver: str,
    member: str,
    declared: Declared | Library,
    identity: str,
    nodes: list[ast.stmt],
) -> dict[str, object]:
    """One exported member, as `exports.rs` reads it, `nodes` its statements."""
    library = isinstance(declared, Library) or is_dunder(member)
    doc, markers = documentation(nodes)
    return {
        "entry": entry.name,
        "receiver": receiver,
        "member": member,
        "id": identity,
        "home": entry.package,
        "package": declared.module.package if isinstance(declared, Declared) else None,
        "generated": isinstance(declared, Declared) and declared.module.generated,
        "library": library,
        "doc": doc,
        "markers": markers,
    }


def class_rows(
    reader: Reader, entry: Module, receiver: str, declared: Declared
) -> list[dict[str, object]]:
    """The public members of the class `declared`, exported as `receiver`."""
    classes = [
        (owner, owner.node)
        for owner in reader.bases(declared)
        if isinstance(owner.node, ast.ClassDef)
    ]
    owned = [
        (owner, name, method)
        for owner, node in classes
        for name, method in body_members(node)
    ]
    data = not any(
        is_method(statement) for _, node in classes for statement in node.body
    )
    rows: list[dict[str, object]] = []
    seen: set[str] = set()
    for owner, name, method in owned:
        private = name.startswith("_") and not is_dunder(name)
        if name in seen or private or (data and not method):
            continue
        seen.add(name)
        identity = f"{owner.module.name}:{owner.name}.{name}"
        nodes = (
            named(owner.node.body, name) if isinstance(owner.node, ast.ClassDef) else []
        )
        rows.append(row(entry, receiver, name, owner, identity, nodes))
    return rows


def exports(sdk: Path) -> list[dict[str, object]]:
    """Every member each entry of the SDK's packages exports."""
    modules = read_modules(sdk)
    reader = Reader(modules)
    rows: list[dict[str, object]] = []
    for module in modules.values():
        private = any(part.startswith("_") for part in module.name.split("."))
        names = exported_names(module)
        if module.generated or private or names is None:
            continue
        for name in names:
            declared = reader.resolve(module, name)
            if (
                isinstance(declared, Declared)
                and declared.node is None
                and declared.name not in modules
            ):
                message = (
                    f"`{module.name}`'s `__all__` names `{name}`, "
                    "which it neither defines nor imports"
                )
                raise ReadError(message)
            identity = (
                f"{declared.module.name}:{declared.name}"
                if isinstance(declared, Declared)
                else f"{declared.module}:{declared.name}"
            )
            nodes = (
                named(statements(declared.module.tree.body), declared.name)
                if isinstance(declared, Declared)
                else []
            )
            rows.append(row(module, MODULE, name, declared, identity, nodes))
            if isinstance(declared, Declared) and isinstance(
                declared.node, ast.ClassDef
            ):
                rows += class_rows(reader, module, name, declared)
    return rows


if __name__ == "__main__":
    try:
        sys.stdout.write(json.dumps(exports(Path(sys.argv[1]))))
    except ReadError as error:
        sys.stderr.write(f"{error}\n")
        sys.exit(1)
