# `@goodfetch/collie-ai-sdk`

Collie's Framework adapter for the [Vercel AI SDK](https://ai-sdk.dev/) (ADRs 0011, 0070, 0095 and 0099).

## Peer range

- `ai`: `^7.0.0` (current major at first publish)

## Usage

```ts
import { Collie } from "@goodfetch/collie";
import { collieTools } from "@goodfetch/collie-ai-sdk";
import { generateText } from "ai";

await using collie = await Collie.connect();
const tools = collieTools(collie, { profile: "browser" });

const result = await generateText({
  model,
  tools,
  prompt: "Open https://example.com and read its title.",
});
```

### Profiles

`collieTools(connection, { profile })` accepts Collie's five MCP profiles (`"browser"` by default, `"agent"`, `"run"`, `"code"`, `"full"`). Only agent-surface operations are exposed; granting, person-only, and SDK-only operations (`page.lend`, `plugin.*`, `model.set_price`, `session.cookies`, `effect.*`, `live_view.*`, etc.) are never offered.
