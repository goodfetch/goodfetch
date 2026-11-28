# `goodfetch-collie` (Activepieces Piece)

Activepieces piece for Goodfetch Collie (ADRs 0027, 0070, 0090, 0095).

## Reachability

- **Local Daemon**: Works directly with self-hosted Activepieces when the Collie Daemon's HTTP listener is enabled on loopback or a reachable private address (`http://127.0.0.1:7801`).
- **Exposed Daemon**: Works with both self-hosted Activepieces and Activepieces Cloud when the Collie Daemon is exposed over TLS with a Bearer token (ADR 0017).
- **Tunnel**: Works with Activepieces Cloud when a tunnel exposes the Daemon's HTTP listener over HTTPS with a Bearer token.

## Actions and Trigger

- `start-recipe` (`recipe.start`, detached)
- `start-catalog-run` (`catalog_run.start`, detached)
- `get-run-report` (`run.report`)
- `get-run-progress` (`run.progress`)
- `notification` trigger (`notification.subscribe` / `notification.unsubscribe` with Standard Webhooks verification and single-delivery deduplication by `event_id`)
