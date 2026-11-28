# `@goodfetch/collie-mastra`

Collie's Mastra framework adapter (ADRs 0011, 0070, 0095 and 0099).

Projects Collie's canonical MCP tool profiles (`browser`, `agent`, `run`, `code`, `full`) onto Mastra `createTool()` definitions backed by `@goodfetch/collie`.

## Peer range

- `@mastra/core`: `^1.0.0`

## Usage

```ts
import { Collie } from "@goodfetch/collie";
import { collieTools } from "@goodfetch/collie-mastra";
import { Agent } from "@mastra/core/agent";

await using collie = await Collie.connect();
const tools = collieTools(collie, { profile: "browser" });

const agent = new Agent({
  id: "collie-browser",
  name: "Collie browser agent",
  instructions: "Use the Collie tools to browse and read pages.",
  model,
  tools,
});
```
