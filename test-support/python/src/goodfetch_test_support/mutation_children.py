"""mutmut runs that reach the Python programs a test starts (ADR 0072).

mutmut puts its mutated copy of a package, `mutants/src`, on the path of the
pytest process alone, and learns which tests reach each function from that
process alone. Under `--mutation-children`, which each package's
`[tool.mutmut]` passes, a Python program a test starts runs the mutated copy
too, with the active mutant: the test's cleared environment map and the
runner's own environment name, on `PYTHONPATH`, the `mutation_child` folder
beside this module and the mutated copy, and give `MUTANT_UNDER_TEST`. While
mutmut learns which tests reach each function, `mutation_child`'s
`sitecustomize` writes each function the program reaches to the run's hits
folder, which the plugin counts as reached by the test running then.
"""

import os
import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Self

CHILD_FOLDER = Path(__file__).parent / "mutation_child"
"""The folder first on a program's `PYTHONPATH`, whose `sitecustomize` it runs."""

HITS_VARIABLE = "MUTATION_CHILD_HITS"
"""The variable that names the hits folder to a program, read by `sitecustomize`."""


@dataclass(frozen=True)
class MutationChildren:
    """One mutmut pytest run's hits folder and the mutated copies it mutates."""

    hits: Path
    """The folder where each program writes the functions it reached, a file each."""
    sources: tuple[Path, ...]
    """The mutated copies of the package's sources, such as `<package>/mutants/src`."""

    @classmethod
    def start(cls) -> Self:
        """Creates the run's hits folder, and reads the copies mutmut mutates."""
        # mutmut is the workspace's dev dependency, never test support's: only
        # a mutmut run passes `--mutation-children`.
        from mutmut.configuration import config

        hits = Path(tempfile.mkdtemp(prefix="goodfetch-mutation-hits-"))
        return cls(hits, tuple(config().resolved_mutated_source_paths))

    def variables(self) -> dict[str, str]:
        """The variables under which a program runs the mutated copy and its mutant."""
        from mutmut.mutation.trampoline import get_mutant_under_test

        return {
            "PYTHONPATH": os.pathsep.join(
                [str(CHILD_FOLDER), *(str(source) for source in self.sources)]
            ),
            HITS_VARIABLE: str(self.hits),
            "MUTANT_UNDER_TEST": get_mutant_under_test(),
        }

    def record_hits(self) -> None:
        """Counts for mutmut each function the programs reached since the last call.

        mutmut gives the running test each function counted since the last
        test, so the programs' reach counts as the test's own.
        """
        from mutmut.stats import record_trampoline_hit

        for name in sorted(take_hits(self.hits)):
            record_trampoline_hit(name)

    def stop(self) -> None:
        """Removes the hits folder."""
        shutil.rmtree(self.hits, ignore_errors=True)


def take_hits(folder: Path) -> set[str]:
    """Each function named in `folder`'s files, one per line, the files removed."""
    reached: set[str] = set()
    for file in folder.iterdir():
        reached.update(file.read_text("utf-8").splitlines())
        file.unlink()
    return reached
