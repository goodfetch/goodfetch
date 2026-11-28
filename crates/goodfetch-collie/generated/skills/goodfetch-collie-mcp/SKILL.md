---
name: goodfetch-collie-mcp
description: "Drives a browser through the goodfetch-collie Model Context Protocol (MCP) server: starts the server over stdio, calls projected Run, page and locator tools, and checks Run reports and Usage. Use when an MCP host needs a web page loaded, acted on or read."
---
# Driving a browser with the goodfetch-collie MCP server

`goodfetch-collie mcp` runs the Model Context Protocol server over standard input and output. Standard output carries only JSON-RPC frames; logs go to standard error.

## Starting the server

Start the server in your MCP host configuration:

```console
$ goodfetch-collie mcp
```

By default it connects to a running Daemon (starting one when configured) or runs the engine in-process. Pass `--daemon` to require a Daemon, or `--attached` to run an in-process engine that stops when the MCP session ends.

## Tools and workflow

Each catalog operation is projected onto an MCP tool name with `_` in place of `.` (for example `run.start` becomes `run_start` and `page.goto` becomes `page_goto`). Arguments and `structuredContent` fields use `snake_case`, and tool errors return `isError: true` with a structured `CommandError` and its `fix` first in text.

A browser Run over MCP follows the same lifecycle as the CLI:

1. Start a Run with `run_start` (`run.start`), which returns `run_id`.
2. Open a session with `run_new_session` (`run.new_session`) and a page with `session_new_page` (`session.new_page`), which returns `page_id`.
3. Navigate with `page_goto` (`page.goto`), read fields with `page_read` (`page.read`), and interact with elements using `locator_click` (`locator.click`), `locator_fill` (`locator.fill`), and `locator_press` (`locator.press`).
4. Check progress with `run_progress` (`run.progress`), stop safely with `run_stop` (`run.stop`), or finish with `run_finish` (`run.finish`) and read the Run report with `run_report` (`run.report`).

## What only a person answers

Person-only and granting operations are never exposed as MCP tools. When a Run hits a Limit or waits on a certificate pin, stop and ask the person to answer it at their own terminal:

- To raise a Limit, ask the person to run `goodfetch-collie usage raise`.
- To pin a host's changed key, ask the person to run `goodfetch-collie certs accept <host>`.
- To remove a host's pin, ask the person to run `goodfetch-collie certs forget <host>`.
