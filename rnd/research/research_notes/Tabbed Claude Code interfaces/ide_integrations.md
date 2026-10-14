# IDE and editor integrations for tabbed Claude Code sessions

Research date: 2026-10-13. Exa and Firecrawl were out of credits during this pass, so the sources are official docs fetched directly plus standard web search. Third-party claims are marked as such.

## Official Claude Code VS Code extension: multiple sessions in tabs? sub-agent visibility? diff tabs? built-in browser?

### Takeaway
Yes to session tabs: each conversation can be its own VS Code editor tab or window, with status dots, renaming, groups and a reopen-closed-tab shortcut. Subagents do not get their own tabs. They show as live progress rows inside the parent chat and in an "agent map" tree, where you can open a read-only transcript for each. Diffs open in VS Code's native side-by-side diff editor, and plans open as a Markdown document. There is no built-in browser pane: browser work goes through the external Claude in Chrome extension, which drives real Chrome tabs.

### Cited Findings
- "open multiple conversations in separate tabs or windows" is listed as a core feature — [Claude Code docs: VS Code](https://code.claude.com/docs/en/vs-code)
- Commands: "Open in New Tab" (`Cmd+Shift+Esc`) "open a new conversation as an editor tab", and "Open in New Window". "Each conversation maintains its own history and context, allowing you to work on different tasks in parallel." — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Tab status: a colored dot on the spark icon, blue for a pending permission request and orange when Claude finished while the tab was hidden — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- You can drag the panel to the secondary sidebar or to the editor area, where it "opens Claude as a tab alongside your files". The extension locks the editor group Claude opens in (`lockEditorGroups`, v2.1.274+) so files open in another group. The docs suggest "Use the sidebar for your main Claude session and open additional tabs for side tasks." — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Tab session management: Rename Session Tab, Add Session Tab to Group, Mark Session as Unread (v2.1.257+); Reopen Closed Session with `Cmd+Shift+T`. A sessions list in the Activity Bar has Active and status filters (Needs input, Working, Completed, Open, Closed). Editor tabs survive a window reload; sidebar chats survive only if they were active in the last 10 minutes — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Session history: clicking a conversation that is "already open in another tab of the current window" switches to that tab. A Local/Web dialog also lists claude.ai cloud sessions and Remote Control sessions. A URI handler, `vscode://anthropic.claude-code/open`, opens a new tab or focuses the one that already holds a given `session` — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Subagents (v2.1.269+): "live progress rows with their latest activity appear under the tool-call group that started them". An "N agents" count in the prompt box opens the **agent map**, which "draws the conversation's subagents as a tree under the main agent, each with its status, elapsed time, and token count. Click a subagent to see its prompt and tool calls, open its read-only transcript, or stop it while it runs." `/tasks` opens the same map, and background shells are listed there too (v2.1.277+) — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- "Run in background" moves a running command or subagent into a background task that stays in the agent map — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Diffs: in Manual mode an edit shows "a side-by-side comparison of the original and proposed changes" in VS Code's diff view. You can edit inside the diff and accept or reject each change (v2.1.275+). Diffs with more than 100 changes lose the per-change buttons — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Plan mode: "VS Code automatically opens the plan as a full Markdown document where you can add inline comments" — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Other panels: `/btw` side questions open "in a panel beside the chat"; checkpoints offer rewind and fork-conversation actions; Focus view hides tool calls — [VS Code docs](https://code.claude.com/docs/en/vs-code)
- Browser: "Connect Claude to your Chrome browser…" through `@browser`, which needs the Claude in Chrome extension 1.0.36+. "Claude opens new tabs for browser tasks and shares your browser's login state." These are tabs in external Chrome, not in VS Code — [VS Code docs](https://code.claude.com/docs/en/vs-code), [Chrome docs](https://code.claude.com/docs/en/chrome)
- A terminal mode (`claudeCode.useTerminal`) runs the CLI TUI instead of the graphical panel. The extension and CLI share conversation history (`claude --resume`) — [VS Code docs](https://code.claude.com/docs/en/vs-code)

### Inferences
- The model is "a session per editor tab", with subagents as a nested inspector (tree plus read-only transcript), not peer tabs. Files and diffs come from VS Code's own editor tabs. A browser tab inside the editor is absent; VS Code's Simple Browser is not mentioned as an integration.
- Version numbers like v2.1.257 to v2.1.280 show that the tab and session features were added recently (2026). Older third-party guides that say "no subagent support in the extension" (for example [ClaudeLog](https://claudelog.com/faqs/how-to-use-claude-code-with-vs-code)) are out of date.

### Gaps
- Whether a subagent's read-only transcript opens as an editor tab or inside a panel or overlay is not stated.
- No official statement on whether the extension's webview source is open.

## Official JetBrains plugin

### Takeaway
It is a thin bridge, not a graphical chat. It runs the `claude` CLI in the IDE's terminal and adds native diff viewing, selection sharing and diagnostics through a local `ide` MCP server. Multiple sessions are only as many terminal tabs as you open, and subagents show as they do in the CLI TUI. JetBrains' own AIR client consumes Claude through ACP with native subagent sessions (see the Zed section).

### Cited Findings
- "The plugin runs the `claude` command in your IDE's integrated terminal and connects to it. It does not bundle its own copy of the CLI" — [Claude Code docs: JetBrains](https://code.claude.com/docs/en/jetbrains)
- "Claude Code opens code changes in the IDE diff viewer instead of the terminal" (the Diff tool setting is `auto` or `terminal`) — [JetBrains docs](https://code.claude.com/docs/en/jetbrains)
- The plugin is still labelled "Claude Code [Beta]" on the JetBrains Marketplace (plugin 27310) — [JetBrains docs](https://code.claude.com/docs/en/jetbrains), [Marketplace](https://plugins.jetbrains.com/plugin/27310-claude-code-beta-)
- The local MCP server "is how the CLI opens diffs in the IDE's native diff viewer, reads your current selection…"; the only model-visible tool is `mcp__ide__getDiagnostics` — [JetBrains docs](https://code.claude.com/docs/en/jetbrains)
- The claude-agent-acp adapter has a specific path for "JetBrains AIR", which uses an earlier draft RFD (`subagent_spawned`, `subagent_state_update`) enabled with `nativeSubagentSessions` — [claude-agent-acp README](https://github.com/agentclientprotocol/claude-agent-acp)

### Inferences
- In JetBrains, the official route gives no session tabs and no subagent tabs. JetBrains AIR, through ACP, is the route that surfaces subagents as their own sessions.

### Gaps
- I did not fetch the JetBrains AIR docs, so how AIR lays out Claude threads and subagents (tabs or a tree) is unconfirmed.

## Cursor, Windsurf (now Devin Desktop) and Kiro

### Takeaway
All three are VS Code forks. They install the same official Anthropic extension from Open VSX (Cursor has a direct install link), so the Claude session-tab, agent-map and diff model is the same as in VS Code. Their own native agent UIs are separate products. Cursor 3 has a built-in browser pane and an Agents Window, but no source confirms that Claude Code runs inside the Agents Window.

### Cited Findings
- The official docs give an "Install for Cursor" link (`cursor:extension/anthropic.claude-code`) and say: "The extension also installs in other VS Code forks like Devin Desktop or Kiro… or install from the Open VSX registry" — [VS Code docs](https://code.claude.com/docs/en/vs-code), [Open VSX listing](https://open-vsx.org/extension/Anthropic/claude-code)
- Windsurf was renamed Devin Desktop on 2026-06-02, and Cascade was replaced by "Devin Local" (third-party reports; I did not find Cognition's own announcement) — [plushcap summary](https://www.plushcap.com/content/windsurf/blog/windsurf-windsurf-is-now-devin-desktop), [Claude Market blog](https://claudemarket.ai/blog/windsurf)
- Cursor's browser "opens as a pane within Cursor, giving Agent full control through MCP tools", "either in a separate window or an inline pane" — [Cursor docs: Browser](https://cursor.com/docs/agent/browser)
- Cursor 3 (2026-04-02) shipped an Agents Window that runs parallel agents across local, worktree, cloud and SSH (third-party coverage) — [The New Stack](https://thenewstack.io/cursors-agents-window-vs-claude-code/), [The Decoder](https://the-decoder.com/new-cursor-3-ditches-the-classic-ide-layout-for-an-agent-first-interface-built-around-parallel-ai-fleets/)

### Inferences
- In a fork, Claude Code tabs can sit next to the host's native browser pane (Cursor) and its file and diff tabs, so an "IDE plus Claude extension" setup already gives session tabs, file tabs, diff tabs and, in Cursor, a browser pane in one window. Subagents still have no tabs.

### Gaps
- Whether Cursor's built-in browser tools are reachable by the Claude Code extension, as opposed to Cursor's own Agent, is unconfirmed.
- Kiro's and Devin Desktop's native agent tab models were not researched in depth.
- Whether the Open VSX build lags the VS Code Marketplace build is unconfirmed.

## Zed and other ACP clients: how do Claude Code threads and sub-agents appear?

### Takeaway
Zed runs Claude through the ACP adapter `claude-agent-acp` (Claude Agent SDK, vendored CLI) as "Claude Agent" threads in the Agent Panel and the Threads Sidebar. Several threads can run at once (Parallel Agents, May 2026). Since May 2026, Terminal Threads can also host the plain `claude` TUI in the sidebar. The adapter can expose each subagent as its own ACP child session when the client declares that capability; otherwise subagents stay tool calls. Diff review opens as a multibuffer tab. Zed's docs do not describe how Zed itself renders subagent child sessions.

### Cited Findings
- Claude Agent is installed from the ACP Registry; threads are hosted in the Agent Panel and Threads Sidebar; "An Anthropic API key configured for Zed Agent does not automatically configure Claude Agent" — [Zed docs: External agents](https://zed.dev/docs/ai/external-agents)
- Zed runs the Claude Agent SDK through a dedicated adapter; it "will always use this managed version of the Claude Agent adapter, which includes a vendored version of the Claude Code CLI" (search-result excerpt of the Zed docs) — [Zed docs: External agents](https://zed.dev/docs/ai/external-agents.md)
- "You can run multiple agent threads at once, each working independently"; the Threads Sidebar (`cmd-alt-j`) groups threads by project; "Review Changes" "opens a special multi-buffer tab with all changes" with per-hunk accept or reject; "Open Thread as Markdown" opens a thread as a file tab — [Zed docs: Agent Panel](https://zed.dev/docs/ai/agent-panel)
- Terminal Threads (2026-05-20) bring terminal agents such as `claude` into the same sidebar and "complement" ACP — [Zed blog: Terminal Threads](https://zed.dev/blog/terminal-threads)
- The adapter's features include "Edit review", "TODO lists", "Interactive (and background) terminals", "Custom Slash commands" and "Nested subagent transcripts". The repo is `agentclientprotocol/claude-agent-acp`, package `@agentclientprotocol/claude-agent-acp` — [claude-agent-acp README](https://github.com/agentclientprotocol/claude-agent-acp)
- Subagent sessions: a client that declares `clientCapabilities.subagents` gets "each Agent or Task subagent as its own child session". The parent receives `subagent_update` with a title and description, and child state is `running`, `requires_action` or `idle`. Clients "can't yet prompt, cancel, or otherwise change a child, and replay doesn't restore children". Subagents are baseline in ACP v2. Without the capability, subagents "stay ordinary tool calls and child interactions stay on the root session" — [claude-agent-acp README](https://github.com/agentclientprotocol/claude-agent-acp)

### Inferences
- ACP now gives a protocol-level model of subagents as child sessions, the clean basis for "sub-agent tabs" in any ACP client. Child sessions are read-only today.
- The older name `claude-code-acp` (Zed's original 2025 adapter) appears to have been replaced by `claude-agent-acp` under the `agentclientprotocol` org. The README never mentions the old name, so the rename is unconfirmed.

### Gaps
- Whether Zed declares `clientCapabilities.subagents`, and how it draws child sessions (sidebar child rows or separate threads), is not documented in the pages fetched.
- No browser pane in Zed was found.

## Community VS Code extensions offering session or sub-agent tabs

### Takeaway
Community GUIs exist, notably andrepimenta's "Claude Code Chat", built on the SDK with checkpoints and an MCP manager. None was confirmed to offer sub-agent tabs. The official extension now covers session tabs and the subagent tree, which makes most of them redundant.

### Cited Findings
- "Claude Code Chat" by andrepimenta is a graphical interface over the Claude Code SDK with checkpoints, an MCP server manager and plan or thinking modes; it requires the CLI (third-party summary) — [SourcePulse](https://www.sourcepulse.org/projects/9089737)
- The GitHub topic `claude-code-extension` lists projects such as out-of-tree patches to the official extension and a live session-transcript viewer for VS Code (not opened or verified) — [GitHub topic](https://github.com/topics/claude-code-extension)
- A third-party blog claims VS Code 1.128 added multi-chat for Claude agent sessions in VS Code's own Chat view; I could not confirm this in VS Code release notes — [Developers Digest](https://www.developersdigest.tech/blog/vscode-1-128-multi-chat-claude-developer-guide-2026)

### Inferences
- The strongest "sub-agent tab" options found are protocol-driven (ACP child sessions) or native (the official agent map), not community VS Code extensions.

### Gaps
- No community extension confirmed with sub-agent tabs. Specific names such as Claudix were not found in search. Activity and licence of the GitHub-topic projects were not checked.
- Whether VS Code's native Chat or Agent Sessions view hosts Claude Code was not verified from code.visualstudio.com.

## Codex desktop app tab model (reference point)

### Takeaway
As of October 2026, the Codex docs at developers.openai.com/codex permanently redirect (308) to learn.chatgpt.com/docs, and the app is called the "ChatGPT desktop app". Its tabs sit beside the chat in split or full view (`Cmd+Shift+B`) and confirmed tab types are the Browser and a per-chat Terminal (opened with "New tab"). Diffs live in a separate "review pane", file and document previews appear "alongside the chat", and subagents are inspectable threads listed as Active and Done, not documented as tabs.

### Cited Findings
- `developers.openai.com/codex/app/features`, `/codex/browser` and `/codex/agent-configuration/subagents` all return 308 to `learn.chatgpt.com/docs/...` (observed 2026-10-13) — [learn.chatgpt.com features](https://learn.chatgpt.com/docs/features)
- Browser: a tab; "Select Enter full view to expand the current tab, or Enter split view to show the chat and tab side by side"; `Cmd+Shift+B` shows or hides tabs beside the chat; "For repository changes, use the review pane" — [Browser docs](https://learn.chatgpt.com/docs/browser)
- Terminal: "Each chat in the ChatGPT desktop app includes a terminal scoped to its current project or worktree"; opened with "Select New tab, then Terminal, or press Ctrl+`" — [Integrated terminal docs](https://learn.chatgpt.com/docs/integrated-terminal)
- Review pane: "Open the review pane to understand what changed, give line-specific feedback"; clicking a file name "typically opens that file in your chosen editor"; it requires a Git repository — [Code review docs](https://learn.chatgpt.com/docs/code-review)
- File previews: "The ChatGPT desktop app previews generated documents, presentations, spreadsheets, and PDF files alongside the chat", plus interactive HTML previews and a split view for `.tex` source and PDF — [Artifacts viewer docs](https://learn.chatgpt.com/docs/artifacts-viewer)
- Subagents: "The app surfaces each subagent thread so you can inspect its work and the summary returned to the main chat", with read-only "Active" and "Done" lists. You steer or stop them by asking in the chat. In the CLI, `/agent` switches between agent threads; in the IDE extension, active subagents "appear above the composer" — [Subagents docs](https://learn.chatgpt.com/docs/agent-configuration/subagents)

### Inferences
- Codex's model is one chat per thread with a tab strip beside it holding browser and terminal tabs, plus a review pane and artifact previews. It is closer to "browser, terminal and preview tabs per chat" than to "a tab per agent". Its subagent model (inspectable, read-only threads) matches Claude's VS Code agent map and ACP child sessions.
- Older material calling it the "Codex app" at developers.openai.com is outdated in naming and location.

### Gaps
- The full list of options in the "New tab" menu (files? diffs as tabs?) is not documented on the pages fetched.
- Whether the review pane can be detached as a tab is unconfirmed.
- Whether a subagent thread opens as a tab or replaces the main view is unconfirmed.
