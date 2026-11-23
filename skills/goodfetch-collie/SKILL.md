---
name: {{executable}}
description: "Drives a real browser with {{executable}}: start a Run, open pages, navigate, click, fill and read them, then finish the Run and read its report. Use when a task needs a web page loaded, acted on or read, or a Run's Usage, Limits or report checked."
---
# {{executable}}

`{{executable}}` drives a browser for a program or an agent. Everything it does happens inside a Run: a Run holds its pages, counts its Usage against its Limits, and ends with a Run report.

## Pick the guide for the task

| Task | Where |
|---|---|
| Drive a browser from a shell: start a Run, open a page, act on it, read it | The `{{executable}}-cli` skill |
| Check what a Run used and what its Limits leave | `{{executable}} run usage <run-id>` |
| See what waits for a person across every Run | `{{executable}} needs-you list` |
| Check whether the Daemon runs, and as which Product | `{{executable}} daemon status` |

## Where the calls run

A Run outlives the command that starts it, so `{{executable}} run start` reaches the Daemon, starting it when none runs. Every later command on that Run, such as `{{executable}} page goto`, reaches the same Daemon. Add `--json` to any command to read its answer as JSON.

## What only a person answers

Some answers are a person's, never an agent's: raising a Limit, pinning a host's changed certificate key, and removing a host's pin. Each is refused with `person_required` under `--json` and when standard input is no terminal. When a task stops on one of them, stop and ask the person to answer it at their own terminal:

- To raise a Limit, ask the person to run `{{executable}} usage raise`.
- To pin a host's changed key, ask the person to run `{{executable}} certs accept <host>`.
- To remove a host's pin, ask the person to run `{{executable}} certs forget <host>`.

Never run these yourself. `{{executable}} usage set` adds a Limit of your own, which can only lower what applies.
