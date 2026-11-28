---
title: Frameworks reached through MCP
description: Model-free examples connecting OpenAI Agents SDK, Pydantic AI, CrewAI, Google ADK, LlamaIndex and Strands to `goodfetch-collie mcp` over stdio.
products: [collie]
---

# Frameworks reached through MCP

Frameworks that do not ship a dedicated Collie adapter connect directly to Collie's MCP server over stdio using their own MCP client (ADRs 0011 and 0070).

## Install Collie

Install the Collie executable (see [Install](install.md) for all methods):

```sh
cargo binstall goodfetch-collie
```

## Profile

Every example below launches `goodfetch-collie mcp --profile browser`, which exposes Collie's `browser` MCP profile (run lifecycle, page navigation, inspection and effects, with no granting or person-only operations).

## OpenAI Agents SDK

```python
"""Connect OpenAI Agents SDK's MCPServerStdio to `goodfetch-collie mcp` without a model."""

import asyncio
import os

from agents.mcp import MCPServerStdio


async def main() -> None:
    executable = os.environ.get("GOODFETCH_COLLIE_BIN", "goodfetch-collie")
    server = MCPServerStdio(
        params={
            "command": executable,
            "args": ["mcp", "--profile", "browser"],
        }
    )
    async with server:
        tools = await server.list_tools()
        assert any(tool.name == "run_start" for tool in tools)
        started = await server.call_tool("run_start", {})
        assert started is not None


if __name__ == "__main__":
    asyncio.run(main())
```

## Pydantic AI

```python
"""Connect Pydantic AI's MCPServerStdio to `goodfetch-collie mcp` without a model."""

import asyncio
import os

from pydantic_ai.mcp import MCPServerStdio


async def main() -> None:
    executable = os.environ.get("GOODFETCH_COLLIE_BIN", "goodfetch-collie")
    server = MCPServerStdio(executable, args=["mcp", "--profile", "browser"])
    async with server:
        tools = await server.list_tools()
        assert any(tool.name == "run_start" for tool in tools)
        started = await server.direct_call_tool("run_start", {})
        assert started is not None


if __name__ == "__main__":
    asyncio.run(main())
```

## CrewAI

```python
"""Connect CrewAI's MCPServerAdapter to `goodfetch-collie mcp` without a model."""

import os

from crewai_tools import MCPServerAdapter
from mcp import StdioServerParameters


def main() -> None:
    executable = os.environ.get("GOODFETCH_COLLIE_BIN", "goodfetch-collie")
    params = StdioServerParameters(
        command=executable,
        args=["mcp", "--profile", "browser"],
    )
    with MCPServerAdapter(params) as tools:
        by_name = {tool.name: tool for tool in tools}
        assert "run_start" in by_name
        started = by_name["run_start"].run()
        assert started is not None


if __name__ == "__main__":
    main()
```

## Google ADK

```python
"""Connect Google ADK's MCPToolset to `goodfetch-collie mcp` without a model."""

import asyncio
import os

from google.adk.tools.mcp_tool import MCPToolset, StdioConnectionParams
from mcp import StdioServerParameters


async def main() -> None:
    executable = os.environ.get("GOODFETCH_COLLIE_BIN", "goodfetch-collie")
    toolset = MCPToolset(
        connection_params=StdioConnectionParams(
            server_params=StdioServerParameters(
                command=executable,
                args=["mcp", "--profile", "browser"],
            )
        )
    )
    try:
        tools = await toolset.get_tools()
        by_name = {tool.name: tool for tool in tools}
        assert "run_start" in by_name
        started = await by_name["run_start"].run_async(args={}, tool_context=None)
        assert started is not None
    finally:
        await toolset.close()


if __name__ == "__main__":
    asyncio.run(main())
```

## LlamaIndex

```python
"""Connect LlamaIndex's BasicMCPClient to `goodfetch-collie mcp` without a model."""

import asyncio
import os

from llama_index.tools.mcp import BasicMCPClient


async def main() -> None:
    executable = os.environ.get("GOODFETCH_COLLIE_BIN", "goodfetch-collie")
    client = BasicMCPClient(executable, args=["mcp", "--profile", "browser"])
    tools = await client.list_tools()
    assert any(tool.name == "run_start" for tool in tools.tools)
    started = await client.call_tool("run_start", {})
    assert started is not None


if __name__ == "__main__":
    asyncio.run(main())
```

## Strands

```python
"""Connect Strands' MCPClient to `goodfetch-collie mcp` without a model."""

import os

from mcp import StdioServerParameters
from mcp.client.stdio import stdio_client
from strands.tools.mcp import MCPClient


def main() -> None:
    executable = os.environ.get("GOODFETCH_COLLIE_BIN", "goodfetch-collie")
    params = StdioServerParameters(
        command=executable,
        args=["mcp", "--profile", "browser"],
    )
    client = MCPClient(lambda: stdio_client(params))
    with client:
        tools = client.list_tools_sync()
        assert any(tool.tool_name == "run_start" for tool in tools)
        started = client.call_tool_sync(
            tool_use_id="collie-1",
            name="run_start",
            arguments={},
        )
        assert started is not None


if __name__ == "__main__":
    main()
```
