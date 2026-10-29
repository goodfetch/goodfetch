---
title: Operations
description: Find the Daemon's logs, bound them on each deployment target, and turn off the age notice.
products: [collie]
---

# Operations

This guide is for the operator of a Goodfetch Collie Daemon on a server, a VM or a container platform. It is Collie's page: Foxhound runs the same Daemon, so it shows this page under the heading that names Collie.

## Where the logs are

Where the Daemon writes its logs depends on how it was started.

- **`goodfetch-collie daemon run`**, the foreground Daemon that every deployment target runs, writes JSON lines to stdout, and writes log files too only when `log.output` is `file`. Its platform collects stdout and rotates it: see [How each target bounds the logs](#how-each-target-bounds-the-logs).
- **Every other Daemon** writes JSON lines to files of its own in the log folder. That is a Daemon started on demand by a program, by `goodfetch-collie daemon start`, by the per-user service `daemon install` registers, or by the Windows logon task.
- **Any other process** writes a log file only when `log.output` is `file` in its configuration.

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

`daemon run`'s stdout is the platform's to keep and rotate. Each deployment target bounds it as follows.

| Target | Where stdout goes | What bounds it |
|---|---|---|
| Compose | Docker's `local` logging driver | Set `logging: driver: local` on the Daemon's service in your Compose file. The `local` driver rotates and compresses, keeping 100 MB per container by default. Docker's default driver, `json-file`, never rotates. This release writes no Compose file for you: `goodfetch-collie init --deploy compose`, which will write one with `local` already set, comes in a later release. Read the logs with `docker compose logs daemon`. |
| systemd | journald | The system unit `goodfetch-collie-<major>.service` sends stdout to journald, which caps its files at 10 % of their filesystem and at most 4 GiB (`SystemMaxUse` in `journald.conf`). Read the logs with `journalctl -u goodfetch-collie-<major>.service`. |
| Kubernetes | The node's container log files | The kubelet rotates each container's log at 10 MiB and keeps 5 files by default (`containerLogMaxSize`, `containerLogMaxFiles`). The Helm chart relies on that rotation. Read the logs with `kubectl logs`. |
| Cloud Run | Cloud Logging | Cloud Logging keeps a log bucket's entries for its retention period, 30 days for the `_Default` bucket. Change the bucket's retention to keep less or more. |
| Lambda | Amazon CloudWatch Logs | A function's log group keeps its entries forever unless you set its retention. Set a retention period on the log group. |

## The age notice

An executable more than three months past its own release date prints one line a day on stderr. The line says that newer versions carry security fixes and a current Chrome, and gives the upgrade command. It compares dates only and makes no network call. It never appears in `--json` output or on a protocol stream, and the Daemon process `daemon start` launches in the background, which has no terminal, does not print it. `daemon run` prints it when it starts, at most once a day, so it shows in the platform's logs above. The day it was last shown is kept per user in the Goodfetch home.

To turn it off, set `version.age_notice` to `false` in the user configuration or in an Operator policy:

```toml
[settings.version]
age_notice = false
```
