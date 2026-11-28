# Collie LangChain and LangGraph adapters

Framework adapters projecting Collie's MCP tool profiles onto LangChain tools
used by both LangChain and LangGraph in TypeScript (`@goodfetch/collie-langchain`)
and Python (`goodfetch-collie-langchain`) (ADRs 0011, 0070, 0095 and 0099).

## Peer range

- TypeScript (`@goodfetch/collie-langchain`): `@langchain/core`: `^1.0.0` (current major at first publish)
- Python (`goodfetch-collie-langchain`): `langchain-core>=1.0.0,<2` (current major at first publish)

## TypeScript

```ts
import { Collie } from "@goodfetch/collie";
import { collieTools } from "@goodfetch/collie-langchain";

await using collie = await Collie.connect();
const tools = collieTools(collie, { profile: "browser" });
```

## Python

```python
from goodfetch_collie import Collie
from goodfetch_collie_langchain import collie_tools

with Collie.connect() as collie:
    tools = collie_tools(collie, profile="browser")
```
