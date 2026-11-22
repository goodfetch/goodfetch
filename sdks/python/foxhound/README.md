# goodfetch-foxhound

Foxhound's Python SDK package (ADR 0095). Area 11 fills it.

The first use in a process of an Experimental member, one outside semver until it is promoted, warns once through `warnings`, as an `ExperimentalWarning`, a `UserWarning` (ADR 0030). A program that accepts its Experimental members hides the warning with a filter on the category this package exports:

```python
import warnings

from goodfetch_foxhound import ExperimentalWarning

warnings.filterwarnings("ignore", category=ExperimentalWarning)
```
