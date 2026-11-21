"""`goodfetch_collie` re-exports every error class of Collie's Error codes.

A program catches any of them from its Product's package, never from the
Collie client package's generated layer, as `@goodfetch/collie` re-exports
every class of `@goodfetch/collie-client`'s generated errors. The classes
are read from the generated module, so a code the catalog gains fails here
until the package re-exports its class.
"""

import inspect

import goodfetch_collie
import pytest
from goodfetch_collie_client.generated import errors
from goodfetch_collie_client.goodfetch_error import GoodfetchError


def generated_error_classes() -> dict[str, type[GoodfetchError]]:
    """Each group class and code class the generated layer declares, by name."""
    return {
        name: value
        for name, value in vars(errors).items()
        if inspect.isclass(value)
        and issubclass(value, GoodfetchError)
        and value.__module__ == errors.__name__
    }


@pytest.mark.covers("story:11-187")
def test_re_exports_every_generated_error_class() -> None:
    """Every group class and code class is in `__all__`, as the same class."""
    classes = generated_error_classes()
    registered = {*errors.ERROR_GROUP_CLASSES.values(), *errors.ERROR_CLASSES.values()}
    assert registered <= set(classes.values())
    missing = sorted(name for name in classes if name not in goodfetch_collie.__all__)
    assert missing == []
    for name, value in classes.items():
        assert getattr(goodfetch_collie, name) is value
