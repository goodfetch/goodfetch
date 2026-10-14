# Terminal and TUI tools that give Claude Code sessions, teammates and sub-agents their own tabs or panes

Research date: 2026-10-13. Repository metadata (stars, last push, latest release) was read from the GitHub REST API (`gh api repos/<owner>/<repo>`) on that date; READMEs were read through the same API. Star counts are a snapshot. Exa and Firecrawl were out of credits, so the web pages were read with WebFetch and the discovery searches used `gh search repos` / `gh search issues`. No Hacker News or Reddit threads were read (see Gaps).

## Which tools exist and are maintained as of October 2026?

### Takeaway
The field splits into three groups. (1) Session managers: Claude Squad, ccmanager, Agent Deck, claude-tmux, workmux, uzi, Agent Farm. (2) Agent-aware terminals or multiplexers that own the panes: herdr (Rust, very popular) and cmux (Ghostty-based macOS app). (3) Small Zellij, kitty and WezTerm add-ons. Most are active in Sep-Oct 2026. uzi is abandoned (last push June 2025). Crystal has been renamed Nimbalyst and is a desktop app, so it is out of the terminal scope.

### Cited Findings

**Maintenance snapshot (GitHub API, 2026-10-13)**

| Tool | Stars | Last push | Latest release | Status | Source |
|---|---|---|---|---|---|
| herdr (herdrdev/herdr) | 43,763 | 2026-10-13 | v0.9.3, 2026-09-29 | Active. Rust, Apache-2.0, repo created 2026-03-27 | [GitHub](https://github.com/herdrdev/herdr) |
| cmux (manaflow-ai/cmux) | 28,144 | 2026-10-13 | v0.65.1, 2026-10-12 | Active. macOS only, Ghostty-based | [GitHub](https://github.com/manaflow-ai/cmux) |
| Claude Squad (smtg-ai/claude-squad) | 8,593 | 2026-08-20 | v1.0.20, 2026-08-20 | Maintained, no push for about 2 months | [GitHub](https://github.com/smtg-ai/claude-squad) |
| workmux (raine/workmux) | 2,825 | 2026-10-13 | (not checked) | Active | [GitHub](https://github.com/raine/workmux) |
| ccmanager (kbwo/ccmanager) | 1,261 | 2026-10-12 | v4.4.6, 2026-10-10 | Active | [GitHub](https://github.com/kbwo/ccmanager) |
| Agent Deck (asheshgoplani/agent-deck) | 1,076 | 2026-10-11 | v1.16.28, 2026-10-10 | Active | [GitHub](https://github.com/asheshgoplani/agent-deck) |
| Claude Code Agent Farm (Dicklesworthstone/claude_code_agent_farm) | 921 | 2026-09-21 | no GitHub releases | Maintained (script framework) | [GitHub](https://github.com/Dicklesworthstone/claude_code_agent_farm) |
| uzi (devflowinc/uzi) | 582 | 2025-06-04 | v0.0.2, 2025-06-03 | **Abandoned**: no push in 16 months | [GitHub](https://github.com/devflowinc/uzi) |
| claude-tmux (nielsgroen/claude-tmux) | 208 | 2026-04-17 | no GitHub releases | Quiet: 6 months since last push | [GitHub](https://github.com/nielsgroen/claude-tmux) |
| claude-code-zellij-status (thoo) | 43 | 2026-01-13 | - | Small, quiet | [GitHub](https://github.com/thoo/claude-code-zellij-status) |
| claude-code-zellij (addelong) | 5 | 2026-02-08 | - | Small patcher, probably stale (tested only on v2.1.34/2.1.36) | [GitHub](https://github.com/addelong/claude-code-zellij) |
| kitty-kommander (galeavenworth-personal) | 0 | 2026-04-19 | - | Hobby project | [GitHub](https://github.com/galeavenworth-personal/kitty-kommander) |
| familiar (DenoBY) | 5 | 2026-09-15 | - | Hobby project: kitty overlays for a session manager, a git reviewer and a log browser | [GitHub](https://github.com/DenoBY/familiar) |
| Crystal (stravu/crystal) | 3,128 | 2026-02-26 | v0.3.5, 2026-02-26 | Renamed "Nimbalyst"; desktop app, not a TUI | [GitHub](https://github.com/stravu/crystal) |
| it2 CLI (mkusaka/it2), needed for official iTerm2 panes | 290 | 2026-10-13 | v0.2.3, 2026-02-08 | Active | [GitHub](https://github.com/mkusaka/it2) |

Out of scope but found during discovery: vibe-kanban (28,344 stars, web UI) and emdash (5,972 stars, desktop ADE) are not terminal tools ([vibe-kanban](https://github.com/BloopAI/vibe-kanban), [emdash](https://github.com/generalaction/emdash)).

**What each tool does**
- **Claude Squad** manages "multiple AI terminal agents like Claude Code, Codex, OpenCode, and Amp". It uses tmux to give each agent an isolated terminal session and git worktrees to put each session on its own branch. Keys: `n` new session, `↵/o` attach, `ctrl-q` detach, `c` checkout (commits and pauses), `r` resume — [README](https://github.com/smtg-ai/claude-squad)
- **ccmanager** manages sessions for Claude Code, Gemini CLI, Codex CLI, Cursor Agent, Copilot CLI, Cline CLI, OpenCode and Kimi CLI across worktrees. It needs no tmux ("completely self-contained", PTY-based). It shows busy, waiting and idle states, can create, merge and delete worktrees, restores sessions after a restart, copies Claude session data between worktrees, and supports `.worktreeinclude` and status hooks. `Ctrl+E` returns to the menu — [README](https://github.com/kbwo/ccmanager)
- **ccmanager and agent teams**: for `claude` sessions it automatically adds `--teammate-mode in-process` "to prevent conflicts between Claude Code's agent teams feature and ccmanager's PTY-based session management". The README notes that setting `teammateMode` in `settings.json` alone is not enough inside a tmux-like environment — [README](https://github.com/kbwo/ccmanager)
- **Agent Deck** calls itself "mission control for your AI coding agents". It has one TUI for Claude, OpenCode, Codex, Gemini and others, plus groups, search, forking through each tool's native fork, git worktrees, cost tracking, per-session MCP and skills toggles, sandboxed container sessions, and a "conductor" session you can drive from a phone over Telegram — [README](https://github.com/asheshgoplani/agent-deck)
- **claude-tmux** is a Rust TUI shown as a tmux popup (`display-popup`). It lists sessions with idle, working or waiting status (detected by reading pane content) and shows a live ANSI preview of the selected Claude pane. It can create, kill and rename sessions, filter them, and supports git worktrees and pull requests — [README](https://github.com/nielsgroen/claude-tmux)
- **workmux** pairs git worktrees with tmux windows. Besides tmux it supports kitty, WezTerm and Zellij backends. It sets up pane layouts and post-creation hooks and has an agent dashboard — [README](https://github.com/raine/workmux)
- **uzi** runs many agents in parallel, each in its own worktree and tmux session. `uzi ls` shows AGENT/MODEL/STATUS/DIFF columns. `run` broadcasts a command to every agent and `checkpoint` rebases an agent's work into the current branch — [README](https://github.com/devflowinc/uzi)
- **Claude Code Agent Farm** runs "20+ Claude Code agents in parallel" in tmux panes. A controller window shows a monitoring dashboard (status, cycles, context, heartbeat, errors), pane titles carry context warnings, and a viewer offers grid, focus and split modes. It types `cc` into each pane, so any agent CLI can be aliased in — [README](https://github.com/Dicklesworthstone/claude_code_agent_farm)
- **herdr** describes itself as "the runtime your coding agents live on". It is a single Rust binary that runs in any terminal. It keeps terminals alive in a background server so you can detach and reattach, marks each pane working, blocked or idle, and lets agents spawn panes and prompt each other through its CLI and socket API. It supports several machines in one window, plugins, and `ctrl+b` prefix keys plus mouse. It doesn't wrap the agents: "it owns their terminals" — [README](https://github.com/herdrdev/herdr)
- herdr's ecosystem includes a mobile PWA, collie (1,326 stars), and a native GPUI client, herdr-gpui (1,218 stars) — [collie](https://github.com/AltanS/collie), [herdr-gpui](https://github.com/penso/herdr-gpui)
- herdr's Claude integration (`herdr integration install claude`) installs a `SessionStart` hook that reports only session identity. Claude's state comes from "screen manifest detection" — [herdr docs: integrations](https://herdr.dev/docs/integrations/)
- **cmux** is "a Ghostty-based macOS terminal with vertical tabs and notifications for AI coding agents". The sidebar shows each workspace's git branch, linked PR, working directory, listening ports and latest notification. A pane waiting for input gets a blue ring. It has `cmux notify` for hooks and a CLI and socket API to create workspaces, split panes and send keys — [README](https://github.com/manaflow-ai/cmux)
- **Zellij**: claude-code-zellij-status reports Claude activity across Zellij panes through zjstatus. claude-code-zellij patches the npm `cli.js` so agent-team teammates open as Zellij panes. The patch cannot work on the native, curl or Homebrew binary — [zellij-status](https://github.com/thoo/claude-code-zellij-status), [claude-code-zellij README](https://github.com/addelong/claude-code-zellij)

### Inferences
- By adoption, the leaders are herdr and cmux, both of which own the terminal rather than sitting on top of tmux. Among session managers, Claude Squad leads in stars, but ccmanager and Agent Deck release more often.
- The tools are converging on the same feature set: one row per agent with a status, a worktree per agent, attach and detach, and notifications. They differ on whether they need tmux (Claude Squad, uzi, Agent Farm and claude-tmux do; ccmanager does not; workmux supports several backends) and on the extras: forking, a phone-driven conductor, a browser pane.

### Gaps
- No Hacker News or Reddit discussion was read, so user sentiment and pain points are not covered here.
- I found no maintained Ghostty-native, WezTerm-native or kitty-native integration that gives teammates their own panes, beyond workmux's backends and hobby repos.
- I did not check "claude-code-tmux", "Claude Code Manager" or similarly named small tools individually. The `gh search` results showed many repos with under 20 stars (mux, ccs, cove and others).

## Do any of them show Task-tool sub-agents (not just separate sessions) as separate interactive tabs?

### Takeaway
No third-party terminal tool I found shows Task-tool sub-agents as their own interactive tabs or panes; they all work at the level of whole sessions. Only Claude Code itself lets you open and message a sub-agent, inside its own UI: rows in the in-session agent panel, plus `/tasks`. Panes exist only for agent-team teammates, which are separate Claude processes. An open feature request asks for workflow sub-agents to get tmux panes too.

### Cited Findings
- Running forks and background sub-agents appear in "a panel below the prompt input, with one row for the main session and one for each fork". `Enter` opens the selected transcript and lets you send follow-ups, and `x` stops the agent or dismisses its row — [Claude Code docs: sub-agents](https://code.claude.com/docs/en/sub-agents)
- Nested sub-agents show as a tree with a `(+N)` count. You can send a message mid-task with `Ctrl+Enter` or `Ctrl+X Ctrl+S` (v2.1.286+) — [sub-agents docs](https://code.claude.com/docs/en/sub-agents)
- When a sub-agent finishes, its row is removed (v2.1.232+). For 30 seconds `/tasks` still lists it and `Enter` opens its transcript. `/tasks` shows the model and effort (v2.1.242+) — [sub-agents docs](https://code.claude.com/docs/en/sub-agents)
- Sub-agent transcripts are saved at `~/.claude/projects/{project}/{sessionId}/subagents/agent-{agentId}.jsonl` — [sub-agents docs](https://code.claude.com/docs/en/sub-agents)
- Agent view (`claude agents`) does not give sub-agents or teammates their own rows. `Tab` on an empty input browses sub-agents instead — [Claude Code docs: agent view](https://code.claude.com/docs/en/agent-view)
- Open feature request #66686 (2026-06-09): "Let Workflow (dynamic workflow) subagents render in tmux panes, like teammateMode: 'tmux' does for agent teams" — [GitHub issue #66686](https://github.com/anthropics/claude-code/issues/66686)
- While agent teams are enabled, a sub-agent that Claude names launches as a teammate. So with `teammateMode: tmux` or `iterm2`, named delegations do get their own pane — [Claude Code docs: agent teams](https://code.claude.com/docs/en/agent-teams)
- cmux's `claude-teams` and the Zellij patcher both target teammates (the tmux split path), not Task sub-agents — [cmux docs](https://cmux.com/docs/agent-integrations/claude-code-teams), [claude-code-zellij](https://github.com/addelong/claude-code-zellij)

### Inferences
- Today the only route to a "sub-agent in its own pane" is to enable agent teams and let named sub-agents become teammates, which costs more tokens and is still experimental. Any external tool that wanted sub-agent tabs would have to tail the `subagents/*.jsonl` transcripts read-only. I found no tool that does.

### Gaps
- I did not read the last ~8.5k characters of the sub-agents page, which may cover `Ctrl+O` and `Shift+↓` details.

## Which offer diff review panes, file trees, or browser previews alongside?

### Takeaway
Diff review is common: Claude Squad has a diff tab, workmux has a dashboard diff view with hunk staging, uzi has a DIFF column, and Agent Farm has diff summaries. Only cmux has a browser pane in the terminal, and it is scriptable. I found no file-tree pane in any of these tools. That is left to the multiplexer layout, for example workmux pane layouts with an editor or yazi.

### Cited Findings
- **Claude Squad**: `tab` switches between the preview tab and the diff tab, and `shift-↓/↑` scrolls the diff — [README](https://github.com/smtg-ai/claude-squad)
- **workmux dashboard**: each agent row shows git diff stats for branch and uncommitted changes, PR number and checks, status and session title. `d` opens a WIP diff view with a clickable file sidebar and hunk staging ("Diff view and Patch mode"). The bottom half shows a live, auto-scrolling preview of the agent's terminal. `p` peeks, `i` types straight into an agent, and `1`-`9` jump to an agent — [workmux docs: dashboard](https://workmux.raine.dev/guide/dashboard)
- **uzi**: `uzi ls` has a DIFF column per agent — [README](https://github.com/devflowinc/uzi)
- **Agent Farm**: makes "Git commits with rich diff summaries" and writes HTML run reports; it has no interactive diff pane — [README](https://github.com/Dicklesworthstone/claude_code_agent_farm)
- **cmux**: has an in-app browser split (`⌘⇧L`) with a scriptable API ported from agent-browser (accessibility snapshot, click, fill, evaluate JS), so Claude Code can drive a dev server next to the terminal. It can import cookies from 20+ browsers, and over `cmux ssh` the browser routes through the remote network — [README](https://github.com/manaflow-ai/cmux)
- **familiar**: offers kitty overlays that include "a git reviewer" — [GitHub](https://github.com/DenoBY/familiar)
- **Claude Code agent view**: its docs describe no diff or file viewer. Edits happen in worktrees under `.claude/worktrees/` — [agent view docs](https://code.claude.com/docs/en/agent-view)

### Inferences
- A cockpit with a diff review pane plus a browser preview next to each agent does not exist as a single terminal tool today. cmux comes closest on browser and layout, and workmux and Claude Squad on diff review.

### Gaps
- I could not confirm whether Agent Deck or ccmanager has a diff pane; their READMEs, searched for "diff", showed nothing. herdr's integrations page mentions no diff, file or browser panes, though its plugin marketplace was not checked.

## What are the official agent-teams display modes (in-process, tmux, iterm2) and their limits?

### Takeaway
Agent teams are experimental and need `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`. `teammateMode` takes `in-process` (the default: an agent panel inside the lead's terminal), `auto`, `tmux` or `iterm2` (split panes, one full Claude process per teammate). Split panes work only through tmux or iTerm2 with the `it2` CLI. The docs list them as unsupported in VS Code's terminal, Windows Terminal and Ghostty. Open requests ask for WezTerm, Ghostty, Windows Terminal and cmux backends.

### Cited Findings
- To enable teams, set `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` in the environment or in `settings.json` `env`. They need an interactive session: no teammates are spawned in `-p` mode or from the Agent SDK — [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- **Modes**: `"in-process"` is the default. `"auto"` uses split panes when you are inside tmux, or in iTerm2 with `it2` installed, and falls back to in-process otherwise. `"tmux"` turns on split panes and auto-detects tmux or iTerm2. `"iterm2"` forces native iTerm2 panes and needs the `it2` CLI plus iTerm2's Python API. Set the mode with `teammateMode` in `~/.claude/settings.json`, or per session with `claude --teammate-mode auto`, an experimental flag hidden from `--help` — [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- **In-process UI**: `↑/↓` selects a teammate, `Enter` opens its transcript to message it, `Esc` interrupts it, `x` stops it, and `Ctrl+T` toggles the task list. Idle rows hide 30 seconds after the whole panel goes idle, and more than three idle teammates collapse into one `N idle agents` row — [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- **Split-pane UI**: "each teammate gets its own pane… click into a pane to interact directly. Each teammate has a full view of their own terminal." The docs suggest `tmux -CC` inside iTerm2 — [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- **Behaviour that depends on the mode**: a split-pane teammate uses the sub-agent definition body *in place of* the default system prompt and applies `mcpServers`. An in-process teammate appends the body and ignores `mcpServers`. Split-pane teammates inherit the lead's effort from v2.1.186 — [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- **Documented limits**:
  - `/resume` and `/rewind` don't restore in-process teammates.
  - Task status can lag, and shutdown can be slow.
  - A session has one team, and teams can't nest.
  - In-process teammates can't run background sub-agents.
  - The lead is fixed for the session's lifetime.
  - Permission modes are set at spawn.
  - Split panes require tmux or iTerm2 and are "not supported in VS Code's integrated terminal, Windows Terminal, or Ghostty".
  - tmux sessions can be left orphaned.

  Source: [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- Team state lives in `~/.claude/teams/{team}/config.json`, which holds tmux pane IDs. Mailboxes are at `~/.claude/teams/{team}/inboxes/{agent}.json` and tasks under `~/.claude/tasks/{team}/` — [agent teams docs](https://code.claude.com/docs/en/agent-teams)
- **Agent view (`claude agents`)**, the official multi-session baseline, is a research preview. It lists background sessions across projects, grouped Pinned / Ready for review / Needs input / Working / Completed, with PR labels. `Enter` or `→` attaches to a full session in fullscreen, `Space` peeks and lets you reply, and `Alt+1..9` jumps to a session. Related commands are `claude --bg`, `claude attach`, `logs`, `stop` and `respawn`. Sessions run under a local daemon in worktrees under `.claude/worktrees/` and stop when the machine shuts down. You can disable it with `disableAgentView` — [agent view docs](https://code.claude.com/docs/en/agent-view)
- **Open backend requests and bugs** (anthropics/claude-code):
  - WezTerm backend, #23574 (2026-02-06) — [issue](https://github.com/anthropics/claude-code/issues/23574)
  - Ghostty backend, #24189 — [issue](https://github.com/anthropics/claude-code/issues/24189)
  - Windows Terminal backend, #24384 — [issue](https://github.com/anthropics/claude-code/issues/24384)
  - cmux backend, #36926 (2026-03-21, open) — [issue](https://github.com/anthropics/claude-code/issues/36926)
  - tmux mode treats the window's first pane as the leader, #98301 — [issue](https://github.com/anthropics/claude-code/issues/98301)
  - Make the leader pane width configurable, #98300 — [issue](https://github.com/anthropics/claude-code/issues/98300)
  - tmux mode forces `pane-border-status` on, #89202 — [issue](https://github.com/anthropics/claude-code/issues/89202)
  - A split-pane teammate shows "manual mode" and a timer that never stops in the lead's view, #98657 — [issue](https://github.com/anthropics/claude-code/issues/98657)
  - A lead's `SendMessage` is delivered only once the teammate goes idle, #98998 — [issue](https://github.com/anthropics/claude-code/issues/98998)
- **Workarounds outside Anthropic**:
  - `cmux claude-teams` puts a tmux shim on `PATH`, sets `TMUX`/`TMUX_PANE`, and translates `split-window`/`send-keys` into cmux socket calls, so teammates "appear as native cmux splits" with sidebar metadata and notifications. Its limits: a fixed list of supported tmux commands, an empty `TMUX_PANE` in teammate panels, and teammates that can land as a tab behind an existing surface instead of a split — [cmux docs](https://cmux.com/docs/agent-integrations/claude-code-teams)
  - claude-code-zellij patches the npm `cli.js` (Zellij 0.40+) — [README](https://github.com/addelong/claude-code-zellij)

### Inferences
- Official tmux mode is the only portable way to get one pane per teammate. Third-party terminals join in by pretending to be tmux (cmux's shim) or by patching the bundle (Zellij), and both are fragile against Claude Code updates. In tmux mode each pane is a full interactive Claude TUI, but the tmux mode bugs listed above show it still has rough edges.

### Gaps
- I did not read the end of the agent view page (about 37k characters), so the version that introduced agent view is not recorded here.
- I did not confirm whether `teammateMode` covers Linux terminals other than tmux. The docs name only tmux and iTerm2.
