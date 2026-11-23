---
name: goodfetch-collie
description: "Drives a real browser with goodfetch-collie: start a Run, open pages, navigate, click, fill and read them, then finish the Run and read its report. Use when a task needs a web page loaded, acted on or read, or a Run's Usage, Limits or report checked."
---
# goodfetch-collie

`goodfetch-collie` drives a browser for a program or an agent. Everything it does happens inside a Run: a Run holds its pages, counts its Usage against its Limits, and ends with a Run report.

## Pick the guide for the task

| Task | Where |
|---|---|
| Drive a browser from a shell: start a Run, open a page, act on it, read it | The `goodfetch-collie-cli` skill |
| Check what a Run used and what its Limits leave | `goodfetch-collie run usage <run-id>` |
| See what waits for a person across every Run | `goodfetch-collie needs-you list` |
| Check whether the Daemon runs, and as which Product | `goodfetch-collie daemon status` |

## Where the calls run

A Run outlives the command that starts it, so `goodfetch-collie run start` reaches the Daemon, starting it when none runs. Every later command on that Run, such as `goodfetch-collie page goto`, reaches the same Daemon. Add `--json` to any command to read its answer as JSON.

## What only a person answers

Some answers are a person's, never an agent's: raising a Limit, pinning a host's changed certificate key, and removing a host's pin. Each is refused with `person_required` under `--json` and when standard input is no terminal. When a task stops on one of them, stop and ask the person to answer it at their own terminal:

- To raise a Limit, ask the person to run `goodfetch-collie usage raise`.
- To pin a host's changed key, ask the person to run `goodfetch-collie certs accept <host>`.
- To remove a host's pin, ask the person to run `goodfetch-collie certs forget <host>`.

Never run these yourself. `goodfetch-collie usage set` adds a Limit of your own, which can only lower what applies.
