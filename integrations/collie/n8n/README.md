# `@goodfetch/n8n-nodes-collie`

Verified n8n community node for Goodfetch Collie (ADRs 0027, 0070, 0090, 0095).

## Reachability

- **Local Daemon**: Works directly with self-hosted n8n when the Collie Daemon's HTTP listener is enabled on loopback or a reachable private address (`http://127.0.0.1:7801`).
- **Exposed Daemon**: Works with both self-hosted n8n and n8n Cloud when the Collie Daemon is exposed over TLS with a Bearer token (ADR 0017).
- **Tunnel**: Works with n8n Cloud when a tunnel exposes the Daemon's HTTP listener over HTTPS with a Bearer token.

## Actions and Trigger

- `start-recipe` (`recipe.start`, detached)
- `start-catalog-run` (`catalog_run.start`, detached)
- `get-run-report` (`run.report`)
- `get-run-progress` (`run.progress`)
- `notification` trigger (`notification.subscribe` / `notification.unsubscribe` with Standard Webhooks HMAC-SHA256 verification and single-delivery deduplication by `event_id`)

## Verified-Node Constraints

This package has zero runtime dependencies and does not access `process.env`, the filesystem, or Node.js built-in modules.
