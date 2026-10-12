# An ops listener serves token-free probes and token-protected metrics, from one OpenTelemetry instrumentation

A Daemon can open an **ops listener**, off by default, on its own port. Its `/livez` and `/readyz` answer without a token and return an HTTP status only; its Prometheus `/metrics` requires a scoped read-only token. Metrics are instrumented once with the OpenTelemetry metrics API and exported through OTLP (decided opt-in in [Adopt runtime and state decisions](https://github.com/Jbmanllr/automan/issues/70)), the Prometheus endpoint, or both, each off until configured. Metric labels hold bounded values only, and metric names are a versioned contract. Decided in [Self-hosted deployment and operations](https://github.com/Jbmanllr/automan/issues/120).

Browserless users asked for a health check that needs no token, for Kubernetes probes ([browserless#4168](https://github.com/browserless/browserless/issues/4168)), and its `/pressure` once failed to return 503 under load ([browserless#4820](https://github.com/browserless/browserless/pull/4820)); its v2 `/metrics` is JSON, not Prometheus. Firecrawl's production probes are an open request ([firecrawl#4291](https://github.com/firecrawl/firecrawl/issues/4291)) and its prom-client metrics sit behind an admin key. Crawlee has OpenTelemetry tracing (`@crawlee/otel`, `crawlee[otel]`) and no metrics endpoint. Kubernetes deprecated `/healthz` in 1.16 for `/livez` and `/readyz` ([docs](https://kubernetes.io/docs/reference/using-api/health-checks/)). Prometheus can receive OTLP, but only with `--web.enable-otlp-receiver`, which most installs do not set.

## Considered Options

- **Probes behind the endpoint token** (ADR 0017): rejected for the two probe paths only. The token would sit in every pod spec, and `exec` probes do not exist on every platform. The probes reveal nothing beyond up or down.
- **Unauthenticated metrics**: rejected. Even bounded metrics reveal a deployment's activity.
- **OTLP only**: rejected. Kubernetes users scrape Prometheus endpoints through `ServiceMonitor`s.
- **Two instrumentations** (the `metrics` crate for Prometheus beside OpenTelemetry): rejected. Names and units would drift between exporters. `opentelemetry-prometheus` is maintained (0.33.1).
- **URLs, domains or run IDs as labels**: rejected. They blow up cardinality and leak targets into monitoring stacks; that detail stays in each run's own state and traces.

## Consequences

**Ops listener** (changes ADRs 0016 and 0017: the probe paths are the only endpoints without a token).
- `/livez` succeeds while the Tokio runtime and the run-state writer respond.
- `/readyz` succeeds while the Daemon can accept a new run: run state is writable, the browsers its configuration uses are available, it is not draining, and the governor has not reached its last-resort stage, where runs stop as `resources_exhausted` (ADR 0040).
- Neither returns a body, a version or run information.
- `/metrics` takes a scoped read-only token as a bearer token.
- `goodfetch daemon health` answers the same questions for `exec` probes and systemd.

**Metrics** (adds to ADR 0030).
- Names follow OpenTelemetry conventions under `goodfetch.*`, rendered `goodfetch_*` for Prometheus, and are generated from `goodfetch-protocol` into the reference, as the Operation catalog is (ADR 0038).
- Each metric is Stable or Experimental under ADR 0030's semver scope.
- The initial set: runs active by Lifecycle and finished by Verdict and Completeness (ADR 0029); work items queued, leased and retried; requests by tier and outcome, Block causes, and latency histograms by tier; browsers running, recycled and their memory; the governor's concurrency target and pressure source; Sink delivery lag and failures by Guarantee class (ADR 0026); model calls, tokens and latency by provider under the GenAI conventions; connected Clients and drain state.
- Labels take values from bounded sets only, such as tier, Block cause, Error code, browser kind and Guarantee class.
- Usage stays journaled and is never derived from metrics or traces.
