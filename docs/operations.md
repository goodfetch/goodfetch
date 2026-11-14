---
title: Operations
description: Find the Daemon's logs, bound them on each deployment target, see when a new Limit holds a Run already going, and turn off the age notice.
products: [collie]
---

# Operations

This guide is for the operator of a Goodfetch Collie Daemon on a server, a VM or a container platform. It is Collie's page: Foxhound runs the same Daemon, so it shows this page under the heading that names Collie.

## Where the logs are

Where the Daemon writes its logs depends on how it was started.

- **`goodfetch-collie daemon run`**, the foreground Daemon that a server, a VM, Compose or Kubernetes runs, writes JSON lines to stdout, and writes log files too only when `log.output` is `file`. Its platform collects stdout and rotates it: see [How each target bounds the logs](#how-each-target-bounds-the-logs).
- **Every other Daemon** writes JSON lines to files of its own in the log folder. That is a Daemon started on demand by a program, by `goodfetch-collie daemon start`, by the per-user service `daemon install` registers, or by the Windows logon task.
- **Any other process** writes a log file only when `log.output` is `file` in its configuration. That includes the Attached engine a Lambda function or a Cloud Run job starts for each invocation: its stdout is its program's protocol channel, so what it logs reaches the platform through the program's log events and stderr.

The log folder is the `logs` folder of the Goodfetch home:

| Where | Log folder |
|---|---|
| Linux | `~/.local/share/goodfetch/logs`, or `$XDG_DATA_HOME/goodfetch/logs` when `XDG_DATA_HOME` is set |
| macOS | `~/Library/Application Support/goodfetch/logs` |
| Windows | `%APPDATA%\goodfetch\logs` |
| `GOODFETCH_HOME` is set | `$GOODFETCH_HOME/logs` |

In the log folder:

- A Daemon writes `daemon-<major>.jsonl`, such as `daemon-1.jsonl`, one set of files across its restarts. During `0.y` previews each minor counts as a major, so the file is `daemon-0.4.jsonl`.
- Any other process writes `goodfetch-collie-<process ID>.jsonl`. When it starts, it keeps the `log.file.max_files` (5) sets of `goodfetch-collie-<process ID>` files written last, its own included, and removes the older sets.
- When the next line would take a file past `log.file.max_size` (10 MiB), the file rotates: `daemon-1.jsonl` becomes `daemon-1.1.jsonl`, the older files move one number up, and the oldest goes. `log.file.max_files` (5) files are kept, the current one included, so a Daemon's logs stay under about 50 MiB.
- A log write never blocks a Run. A line that cannot be written, because the writer's buffer or the disk is full, is dropped and counted, and a line with `fields.dropped` says how many were dropped before the next one.

At the default `log.level`, `info`, the Daemon logs state changes only: a Run started, resumed or stopped, a storage level changing, a Daemon draining. A line per request or per fetch is `debug`, so the volume follows what happens to Runs, not the number of requests.

## How each target bounds the logs

What `daemon run` writes to stdout, and what a Lambda function's or a Cloud Run job's program logs, is the platform's to keep and rotate. Each deployment target bounds it as follows.

| Target | Where stdout goes | What bounds it |
|---|---|---|
| Compose | Docker's `local` logging driver | Set `logging: driver: local` on the Daemon's service in your Compose file. The `local` driver rotates and compresses, keeping 100 MB per container by default. Docker's default driver, `json-file`, never rotates. This release writes no Compose file for you: `goodfetch-collie init --deploy compose`, which will write one with `local` already set, comes in a later release. Read the logs with `docker compose logs daemon`. |
| systemd | journald | A unit that runs `daemon run` sends its stdout to journald by default (`StandardOutput=journal`), which caps its files at 10 % of their filesystem and at most 4 GiB (`SystemMaxUse` in `journald.conf`). This release ships no system unit: write your own, or wait for `goodfetch-collie init --deploy systemd`, which will write one, in a later release. Read the logs with `journalctl -u <your unit>`. |
| Kubernetes | The node's container log files | The kubelet rotates each container's log at 10 MiB and keeps 5 files by default (`containerLogMaxSize`, `containerLogMaxFiles`). This release ships no Helm chart: run `daemon run` in a pod of your own, whose stdout the kubelet rotates; the chart, which will rely on that rotation, comes in a later release. Read the logs with `kubectl logs`. |
| Cloud Run | Cloud Logging | Cloud Logging keeps a log bucket's entries for its retention period, 30 days for the `_Default` bucket. Change the bucket's retention to keep less or more. |
| Lambda | Amazon CloudWatch Logs | A function's log group keeps its entries forever unless you set its retention. Set a retention period on the log group. |

## When a Limit holds a Run already going

A Limit set or raised while a Run is going (`usage set`, `usage raise`) holds the Run's next charge when the process that changed it is the one working the Run: the Daemon you asked, or a program's own engine. A Run another process works, such as an engine a program started itself while you ask the Daemon, reads the Limits again before it takes its next Slice, and one second after its last read at the latest. Until then it charges only what its Slices already hold, and a Run no Limit held goes on for at most that second.

## The age notice

An executable more than three months past its own release date prints one line a day on stderr. The line says that newer versions carry security fixes and a current Chrome, and gives the upgrade command. It compares dates only and makes no network call. It never appears in `--json` output or on a protocol stream, and the Daemon process `daemon start` launches in the background, which has no terminal, does not print it. `daemon run` prints it when it starts, at most once a day, so it shows in the platform's logs above. The day it was last shown is kept per user in the Goodfetch home.

To turn it off, set `version.age_notice` to `false` in the user configuration or in an Operator policy. Both are files in the per-major folder of the Goodfetch home whose `logs` folder the table above names: the user configuration is `configuration.toml` there, such as `~/.local/share/goodfetch/1/configuration.toml` on Linux for major 1 (the folder is `0.4` during the `0.4` previews), and the Operator policy is `operator-policy.toml` beside it, such as `~/.local/share/goodfetch/1/operator-policy.toml`. A settings file starts with its format's version, so a new file holds:

```toml
version = "0.1"

[settings.version]
age_notice = false
```

In a file that already exists, keep its `version` line and add the `[settings.version]` table.

A Run reads both files once, when it starts, and keeps the values it resolved until it ends. A change to either file, whether you edit it by hand or the engine writes it (`config.policy_write`, `config.user_write`), a tightened Operator policy cap included, reaches the next Run, not the Runs already going.

The engine keeps one copy of the machine's settings, the Operator policy and user configuration, for what serves every Run at once: the Resource governor's samples, the Audit log's retention, the managed Limits and the deadline of a forwarded call. It reads them when it starts, again whenever it writes either file itself, and again whenever it reads them for something else, such as a Run's start or `config explain`. So the engine's own write is seen at once, by the next sample and the next Audit log entry, while a hand edit is seen at the engine's next read of the files, when the next Run starts at the latest.
