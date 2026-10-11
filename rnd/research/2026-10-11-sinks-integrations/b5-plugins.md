# B5. How other systems let third parties add sinks (plugin models)

Research date: 2026-10-11. Sources are official docs, GitHub repositories (read through the GitHub API), crates.io and PyPI. Anything not stated by a source is marked **inference**.

## Summary table

| System | Process model | Languages | Distribution | Delivery notes |
|---|---|---|---|---|
| Vector (v0.59.0, MPL-2.0) | In-process, compiled into the binary. No external plugin system. The `wasm` transform was deprecated in 0.16 and removed in 0.17 (2021). Lua transform remains. | Rust (fork). Lua for transforms only. | Fork and rebuild, or upstream a PR. | End-to-end acknowledgements from source to sink. At-least-once with disk buffers. No exactly-once. |
| Fluent Bit (v5.1.3, Apache-2.0) | C plugins: in-process, compiled in. Go plugins: in-process shared objects (`.so`, `dlopen`). Wasm: in-process WAMR sandbox, for filters and inputs only (`filter_wasm`, `in_exec_wasi`). Lua: in-process LuaJIT filter. | C, Go (output and input), Rust/TinyGo/C to Wasm, Lua | Build the `.so` yourself and load it with `-e` or `plugins_file`. No registry. | Each output returns OK, RETRY or ERROR. Retries use exponential backoff and `Retry_Limit`. Data is dropped when retries run out, unless the dead-letter queue (`storage.keep.rejected`) is on. |
| Redpanda Connect (v4.113.0) | Go plugins: in-process, compiled in through `public/service` (needs a custom build). Dynamic plugins (since v4.56.0, 2025-06-05): one subprocess per plugin, gRPC over Unix sockets, batch-only. Wasm processor: in-process wazero. | Go (compiled). Go and Python SDKs for dynamic plugins; any gRPC language in principle. Wasm for processors. Bloblang for mapping. | Custom build: your own `main.go` imports your plugins plus the bundle. Dynamic plugins: a `plugin.yaml` loaded with `--rpc-plugins`. Python SDK on PyPI. No plugin registry. | In-process transaction model with acks back to the input. At-least-once with no disk state. Licences: Apache-2.0 for most, RCL for enterprise features. |
| Kafka Connect (Kafka 4.3 docs) | In the JVM. Each plugin gets its own classloader on `plugin.path`. Discovery by scanning or `ServiceLoader` manifests (`plugin.discovery`). | Java and other JVM languages | JARs or plugin directories on `plugin.path`. Confluent Hub / Marketplace, with `confluent connect plugin install`. | Exactly-once for sources since 3.3.0 (KIP-618): transactional writes of records plus offsets, distributed mode only. Sinks get exactly-once by storing offsets in the destination (`flush` / offsets). By default, sink offsets are committed to the consumer group, so delivery is at-least-once. |
| Terraform providers | Separate process. Plugin protocol v5 (CLI 0.12+) or v6 (CLI 1.0+), protobuf over gRPC, served by `hashicorp/go-plugin` on a local connection. | Go officially (plugin framework, plugin-go, SDKv2). go-plugin says gRPC allows any language. | Terraform Registry: public GitHub repo `terraform-provider-{NAME}`, release zips, manifest, SHA256SUMS with a GPG signature, usually built with GoReleaser. OpenTofu Registry: submit through a GitHub issue form. | Not a streaming system. No delivery semantics; results go to Terraform state. |
| OpenTelemetry Collector (releases v0.162.0) | In-process, compiled in. Dynamic plugins were rejected (issue #1005, closed 2022). | Go | OpenTelemetry Collector Builder (`ocb`): a YAML manifest lists components by `gomod`, and ocb builds a custom distribution binary. | Exporter helper: sending queue plus `retry_on_failure`. Persistent queue through a storage extension (`file_storage`) survives restarts. When the queue is full, data is dropped. |
| Airbyte | One container per connector (Docker image). Airbyte Protocol: JSON lines on stdin/stdout. Newer socket mode: protobuf over Unix domain sockets. | Any language. CDKs: low-code YAML, Python. Java/Kotlin CDK not accepting contributions. Community TypeScript and .NET CDKs. | Push the image to a registry and add it as a custom Docker connector (plan-gated). Or contribute to the Marketplace. | A destination re-emits STATE messages only after the earlier records are committed. The source resumes from the last state both sides confirmed. **Inference**: at-least-once. |
| Debezium Server (3.7.0.Final) | In the JVM (Quarkus). A sink is a CDI bean implementing `DebeziumEngine.ChangeConsumer`, with `@Named` and `@Dependent`. | Java/JVM | Build your own module and add it to the server. **Inference**: likely a rebuild of the Quarkus app, because Quarkus does CDI discovery at build time. | The sink calls `markProcessed` / `markBatchFinished` to commit offsets. Exactly once in normal operation, duplicates after crashes, so at-least-once. |
| Wasmtime component model | In-process Wasm sandbox. WIT interfaces. Host-granted capabilities (`wasi:http`, `wasi:sockets`). | Any language that targets components (Rust, Go, JS, Python, C, ...) | **Inference**: host-defined (a registry or OCI artifacts). Examples: Zed (GitHub registry repo), Spin and wasmCloud (OCI). | Host-defined. The host must drive acks across the Wasm boundary. |
| Fluvio connectors (v0.18.1) | Separate process: one Rust binary per connector, built with the `fluvio-connector-common` crate. SmartModules are Wasm (Wasmtime) running in the cluster. | Rust | `cdk build/test/deploy`. Hub subcommands were removed from the CLI in Jan 2026. | Sink with `offset.strategy: manual` calls `offset_commit()` and `offset_flush()` after the send. **Inference**: at-least-once. |
| Arroyo (v0.15.0) | Connectors compiled in (`arroyo-connectors`, a static registry in `connectors()`). UDFs: Rust compiled to `cdylib`, loaded with `dlopen2`. Python UDFs through PyO3. | Rust (connectors and UDFs), Python (UDFs) | Fork for connectors. UDFs are compiled by the compiler service and stored. | Checkpoint-based. The Kafka sink has an exactly-once mode (transactions, two-phase commit per checkpoint epoch) and an at-least-once mode. |

## Notes

### Vector
- Latest release is v0.59.0 (2026-10-06). The licence is MPL-2.0 (GitHub API). https://github.com/vectordotdev/vector
- Vector added an experimental `wasm` transform in v0.10.0. It was deprecated in 0.16.0 and planned for removal in 0.17.0, and users were pointed to the `lua` transform. https://vector.dev/highlights/2021-08-23-removing-wasm/
- The 0.17.0 changelog lists "Remove WASM code". https://vector.dev/releases/0.17.0/
- An old Wasm plugin RFC exists. https://github.com/vectordotdev/vector/blob/master/rfcs/2020-04-15-2341-wasm-plugins.md
- The use-case collection issue for Wasm is still open (#9466). https://github.com/vectordotdev/vector/issues/9466
- "Writing external plugin?" (#1500) is closed. https://github.com/vectordotdev/vector/issues/1500
- I found no current external plugin mechanism. New sinks are added in-tree.
- End-to-end acknowledgements work like this. A source attaches a batch notifier to its events. The sink records the outcome. On fan-out, the source gets the worst status across sinks. Sources such as `kafka`, `aws_sqs` and `http` can ack upstream. `socket` cannot, and Vector warns about possible silent loss. https://vector.dev/docs/architecture/end-to-end-acknowledgements/
- Delivery guarantees:
  - At-least-once needs a disk buffer.
  - Sinks without ack support count an event as delivered once it is handed off.
  - There is no exactly-once.
  - https://vector.dev/docs/architecture/guarantees/

### Fluent Bit
- Latest release is v5.1.3 (2026-10-01), licence Apache-2.0. https://github.com/fluent/fluent-bit
- Core plugins are C and are compiled into the binary (`plugins/`).
- Go output plugins:
  - Built with `-DFLB_PROXY_GO=On`.
  - The plugin is built with `go build -buildmode=c-shared` and loaded with `-e /path/x.so` or a `plugins_file`.
  - Lifecycle exports: `FLBPluginRegister`, `FLBPluginInit`, `FLBPluginFlushCtx`, `FLBPluginExit`.
  - The interface is labelled "under development but functional".
  - https://docs.fluentbit.io/manual/development/golang-output-plugins
- `fluent-bit-go` supports output plugins (Fluent Bit 1.4+) and input plugins (1.9+). https://github.com/fluent/fluent-bit-go
- Wasm plugins:
  - Supported only for input and filter plugins, not outputs.
  - Config keys: `wasm_path`, `function_name`, `accessible_paths`.
  - Toolchains: Rust, TinyGo, C (WASI SDK).
  - Optional AOT compilation with `flb-wamrc`.
  - https://docs.fluentbit.io/manual/development/wasm-filter-plugins
- The repo bundles `wasm-micro-runtime-WAMR-2.4.1` and `luajit` in `lib/`. So Wasm and Lua both run in-process. https://github.com/fluent/fluent-bit/tree/master/lib
- The Lua filter (`script`, `call`, `protected_mode`) only transforms records. https://docs.fluentbit.io/manual/data-pipeline/filters/lua
- Retries:
  - Exponential backoff with jitter (`scheduler.base`, `scheduler.cap`).
  - `Retry_Limit` is set per output.
  - When retries run out, data is discarded unless `storage.keep.rejected` is on.
  - https://docs.fluentbit.io/manual/administration/scheduling-and-retries

### Redpanda Connect (formerly Benthos)
- Latest release is v4.113.0 (2026-10-09). https://github.com/redpanda-data/connect
- Compiled Go plugins:
  - You write a `main` that imports `github.com/redpanda-data/benthos/v4/public/service`, the free bundle `public/bundle/free/v4` and your packages.
  - Then you run `service.RunCLI` and `go build`.
  - Example: https://github.com/redpanda-data/redpanda-connect-plugin-example
- Dynamic plugins:
  - Each plugin is "an external executable that communicates with Redpanda Connect via gRPC".
  - One subprocess per plugin, over Unix sockets. A crash does not take down the engine.
  - Only the batch types `BatchInput`, `BatchProcessor` and `BatchOutput` are supported.
  - Configured in `plugin.yaml` and loaded with `rpk connect run --rpc-plugins=plugin.yaml`.
  - Requires v4.56.0+. The docs call the framework Apache-2.0.
  - https://docs.redpanda.com/redpanda-connect/plugins/about/
- The changelog for 4.56.0 (2025-06-05) says "New plugin mechanism added over gRPC for dynamically loaded plugins". A later release added `DiscoverAndRegisterPlugins` in `public/plugins/go/rpcnloader`. https://github.com/redpanda-data/connect/blob/main/CHANGELOG.md
- The SDKs live in `public/plugin/go` and `public/plugin/python`. The Python SDK is on PyPI as `redpanda-connect` 0.1.3. https://pypi.org/project/redpanda-connect/
- The `wasm` processor runs on wazero with WASI. It was added in 4.11.0, is described as "a work in progress", and pools runtimes per thread. It is a processor, not a sink. https://docs.redpanda.com/redpanda-connect/components/processors/wasm/
- Licences: "Apache-2.0 covers the majority of connectors and functionality", and the RCL (Redpanda Community License) "covers enterprise features". https://github.com/redpanda-data/connect/blob/main/licenses/README.md
- Delivery: an "in-process transaction model with no need for any disk persisted state". This gives at-least-once with at-least-once sources and sinks. https://github.com/redpanda-data/benthos
- **Inference**: dynamic plugin outputs take part in the same ack model, because the wrapper is a `BatchOutput`.

### Kafka Connect
- Plugins are placed on `plugin.path`. "Connect isolates each plugin from one another so libraries in one plugin are not affected". `CLASSPATH` is discouraged. Install from the Confluent Marketplace (confluent.io/hub) or with `confluent connect plugin install`. https://docs.confluent.io/platform/current/connect/userguide.html
- `plugin.discovery` takes `only_scan`, `service_load`, `hybrid_warn` or `hybrid_fail`. Plugins need a `META-INF/services` manifest. https://kafka.apache.org/41/configuration/kafka-connect-configs/
- Exactly-once:
  - Sources gained exactly-once support "as of version 3.3.0", only in distributed mode.
  - It needs `exactly.once.source.support=enabled`.
  - Sinks need `consumer.isolation.level=read_committed`.
  - https://kafka.apache.org/43/kafka-connect/user-guide/
- KIP-618 covers transactional source offsets, `transaction.boundary` and zombie fencing. It notes that sinks "could already implement exactly-once". https://cwiki.apache.org/confluence/display/KAFKA/KIP-618%3A+Exactly-Once+Support+for+Source+Connectors
- For sinks, offsets can be stored "in the destination store to provide exactly-once delivery" (HDFS uses atomic moves). `flush()` must wait for the destination's ack. https://kafka.apache.org/43/kafka-connect/connector-development-guide/

### Terraform / OpenTofu providers
- The plugin protocol is protobuf over gRPC. v6 works with CLI 1.0+ and v5 with 0.12+. Minor versions are additive. https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol
- `hashicorp/go-plugin`:
  - It is a plugin system over RPC, designed for a "local [reliable] network", with plugins as subprocesses.
  - It says "gRPC-based plugins enable plugins to be written in any language".
  - It is used by Terraform, Vault, Nomad and Packer.
  - https://github.com/hashicorp/go-plugin
- Terraform Registry publishing:
  - Public repo `terraform-provider-{NAME}` with semver `v` tags.
  - Per-OS/arch zips, a manifest JSON and `SHA256SUMS`.
  - A detached binary GPG signature, with RSA or DSA keys only.
  - GoReleaser through GitHub Actions. A webhook ingests new releases.
  - https://developer.hashicorp.com/terraform/registry/providers/publishing
- OpenTofu Registry:
  - Providers and signing keys are submitted through GitHub issue forms only.
  - Versions are immutable. https://github.com/opentofu/registry
  - Signatures are checked at install time. When a provider has no keys in the registry, GPG checking is skipped. `OPENTOFU_ENFORCE_GPG_VALIDATION=true` makes the check strict. https://opentofu.org/docs/cli/plugins/signing/

### OpenTelemetry Collector
- `ocb` "generates a custom OpenTelemetry Collector binary based on a given configuration". The manifest lists `dist`, `receivers`, `processors`, `exporters` and `providers`, each with `gomod:` references. https://github.com/open-telemetry/opentelemetry-collector/blob/main/cmd/builder/README.md
- Issue #1005, "Plug-in system to allow for plug-ins to be dynamically loaded", was closed in 2022. Maintainers said Go's `plugin` package is unfinished, unmaintained and does not work on Windows. They pointed to the builder instead. https://github.com/open-telemetry/opentelemetry-collector/issues/1005
- Latest releases tag is v0.162.0 (2026-09-29). https://github.com/open-telemetry/opentelemetry-collector-releases
- The persistent queue (`storage` set to `file_storage`) resumes after a restart. Data is dropped when the queue is full or storage fails. https://github.com/open-telemetry/opentelemetry-collector/blob/main/exporter/exporterhelper/README.md

### Airbyte
- Protocol:
  - Commands: `spec`, `check`, `discover`, `read`, `write`.
  - One JSON `AirbyteMessage` per line, with logs on stderr.
  - "Outputting a state message indicates that all records which came before it have been successfully written to the destination."
  - State must be returned in order.
  - https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol
- Docker interface:
  - Run as `docker run --rm -i <image> <cmd> --config ...`.
  - The image needs `AIRBYTE_ENTRYPOINT`, a non-root `airbyte` user, and amd64 support.
  - Socket mode (`DATA_CHANNEL_MEDIUM=SOCKET`, protobuf over Unix sockets) is described as 4-10x faster.
  - https://docs.airbyte.com/platform/understanding-airbyte/airbyte-protocol-docker
- Custom connectors: "Add a new Docker connector" with an image name and tag from a registry the instance can pull from. The plan badge shows this on Core. https://docs.airbyte.com/platform/operator-guides/using-custom-connectors
- Build paths: Connector Builder, low-code CDK and Python CDK. The Java CDK is not accepting contributions. https://docs.airbyte.com/platform/connector-development

### Debezium Server
- Latest tag is v3.7.0.Final. Repo modules include sinks for:
  - Kafka, Kinesis, Pub/Sub, Pulsar, Event Hubs, Redis
  - NATS Streaming, NATS JetStream, RabbitMQ, RocketMQ
  - HTTP, Infinispan, Pravega, SNS, SQS, JDBC
  - Milvus, Qdrant, InstructLab, Fluss
  - Databricks Zerobus
  - https://github.com/debezium/debezium-server
- The docs also list Iceberg, Pub/Sub Lite and RabbitMQ native stream. https://debezium.io/documentation/reference/stable/operations/debezium-server.html
- A new sink is "a CDI bean implementing interface `DebeziumEngine.ChangeConsumer` and with annotation `@Named` and unique name and scope `@Dependent`". The bean name becomes `debezium.sink.type`. Config is read through the MicroProfile Config API. (Same page.)
- The API is `handleBatch(List<R>, RecordCommitter<R>)`, using `markProcessed` and `markBatchFinished`. https://github.com/debezium/debezium/blob/main/debezium-api/src/main/java/io/debezium/engine/DebeziumEngine.java
- The distribution's `run.sh` puts `lib/*` on the classpath. **Inference**: dropping a jar into `lib/` may not register a new CDI bean without a Quarkus rebuild. https://github.com/debezium/debezium-server/blob/main/debezium-server-dist/src/main/resources/distro/run.sh
- Delivery: "Under normal operating scenarios, the application sees every event exactly one time". After a crash, events can repeat, so delivery is at-least-once. https://debezium.io/documentation/faq/

### Wasmtime component model and WIT
- Crate versions on crates.io:
  - `wasmtime`, `wasmtime-wasi` and `wasmtime-wasi-http` 49.0.2 (2026-10-02). v50.0.0-rc.1 is tagged (2026-10-05).
  - `wit-bindgen` 0.62.0 (2026-09-10).
  - https://crates.io/crates/wasmtime
  - https://github.com/bytecodealliance/wasmtime/releases
- WASI status:
  - WASI 0.3.0 was ratified and released 2026-06-11. 0.3.1 followed on 2026-08-11.
  - 0.3 replaces `wasi:io` pollables and streams with native component-model `future` and `stream`, and `async func`.
  - 0.2.x is still maintained (v0.2.12, 2026-06-02).
  - https://github.com/WebAssembly/WASI/releases/tag/v0.3.0
  - https://github.com/WebAssembly/WASI
- Wasmtime 46.0.0 (2026-06-22): "Wasmtime now supports WASI 0.3.0 by default and the `component-model-async` wasm feature is now enabled by default". Wasmtime 49 enables the `p3` feature of `wasmtime-wasi-http` by default. https://github.com/bytecodealliance/wasmtime/blob/release-49.0.0/RELEASES.md
- The docs.rs page for `wasmtime-wasi-http` 49.0.2 still calls its `p3` module "Experimental, unstable and incomplete". `p2` is the stable one. https://docs.rs/wasmtime-wasi-http/49.0.2/wasmtime_wasi_http/
- Network I/O for sink plugins:
  - **`wasi:sockets`**: `WasiCtxBuilder` has `inherit_network`, `socket_addr_check` (a per-address callback), `allow_tcp`, `allow_udp` and `allow_ip_name_lookup`. The docs say addresses are denied by default, and they are inconsistent about the TCP and UDP defaults. https://docs.rs/wasmtime-wasi/49.0.2/wasmtime_wasi/struct.WasiCtxBuilder.html
  - **`wasi:http`**: the host implements the outgoing handler. `WasiHttpHooks` and `default_send_request` let the host intercept or filter requests. https://docs.rs/wasmtime-wasi-http/49.0.2/wasmtime_wasi_http/
  - **Inference**: a sink needing Kafka, Postgres and similar wire protocols needs `wasi:sockets` and guest-side client libraries that compile to components. HTTP-only sinks can use `wasi:http`.
- Rust hosts in practice:
  - **Zed**: `extension_host` uses `wasmtime` and `wasmtime-wasi`. Versioned WIT lives in `crates/extension_api/wit/since_v0.x`. Zed exposes its own `http-client` WIT interface rather than `wasi:http`. Extensions are submitted to the `zed-industries/extensions` repo and reviewed. https://github.com/zed-industries/zed/tree/main/crates/extension_api/wit and https://github.com/zed-industries/extensions
  - **Spin** (now `spinframework`): depends on `wasmtime` 49.0.1 with the `p3` feature. Outbound HTTP goes through `wasi-http` and is denied unless listed in `allowed_outbound_hosts`. https://github.com/spinframework/spin and https://spinframework.dev/v3/http-outbound
  - **wasmCloud** (CNCF Incubating, v2.10.3): `wasmtime` 48.0.5, components are deny-by-default and capabilities are granted through interfaces. https://github.com/wasmCloud/wasmCloud
  - **Extism** (crate 1.41.0): not the component model. It uses its own PDK and host-function ABI on `wasmtime` 49 with WASI p1. It offers "host-controlled HTTP without WASI" through manifest `allowed_hosts` (empty means none, null means all). https://github.com/extism/extism and https://extism.org/docs/concepts/manifest
  - **Lapce**: pins `wasmtime` 14 with WASI p1 and a fork of `wasi-experimental-http`. Not the component model. https://github.com/lapce/lapce/blob/master/lapce-proxy/Cargo.toml
  - **Fluvio SmartModules**: `fluvio-smartengine` uses `wasmtime` and `wasi-common`. https://github.com/infinyon/fluvio/tree/master/crates/fluvio-smartengine
  - **Redpanda data transforms**:
    - The broker embeds Wasmtime through its C++ code (`src/v/wasm/wasmtime.cc`). https://github.com/redpanda-data/redpanda/tree/dev/src/v/wasm
    - SDKs for Go, Rust and JS.
    - "Transform functions have no external access to disk or network resources", and delivery is at-least-once. https://docs.redpanda.com/current/develop/data-transforms/how-transforms-work/
  - **Arroyo UDFs**: these are native `cdylib`s, not Wasm (see the Arroyo section).
  - **Envoy proxy-wasm**:
    - Proxy-Wasm is an ABI, not the component model. Latest ABI is v0.2.1.
    - Hosts: Envoy, ATS, NGINX (Kong), MOSN, OpenResty.
    - SDKs: Rust, Go, C++, AssemblyScript.
    - Outbound calls go through the host with `proxy_http_call` and `proxy_grpc_call`.
    - https://github.com/proxy-wasm/spec

### Fluvio connectors
- A connector is a standalone Rust binary using the `#[connector(sink)]` macro from `fluvio-connector-common`. `Connector.toml` declares `[deployment] binary = ...`. https://github.com/infinyon/fluvio/tree/master/connector/sink-test-connector
- The sink example sends and then calls `stream.offset_commit()` and `offset_flush()`, with `consumer.offset.strategy: manual`. **Inference**: at-least-once.
- `cdk` subcommands are Build, Test, Generate, Deploy and Version. Deploy runs the connector locally as a process. https://github.com/infinyon/fluvio/tree/master/crates/cdk
- Commit 4f1edb3 (2026-01-06) is "chore: remove hub related subcommands in cli". https://github.com/infinyon/fluvio/commit/4f1edb3d32e2c4a9ccb419077988dc360e742952
- Latest release is v0.18.1 (2025-07-04).

### Arroyo connectors
- Connectors are Rust modules in `crates/arroyo-connectors`, registered in a static `connectors()` map. That map covers Kafka, Kinesis, NATS, RabbitMQ, Redis, MQTT, Fluvio, filesystem, Delta, Iceberg, webhook and others. Adding a sink means a fork and rebuild. https://github.com/ArroyoSystems/arroyo/blob/master/crates/arroyo-connectors/src/lib.rs
- UDFs:
  - The compiler service builds `crate-type = ["cdylib"]`, stores the dylib, and `arroyo-udf-host` loads it with `dlopen2`.
  - Python UDFs use PyO3.
  - https://github.com/ArroyoSystems/arroyo/tree/master/crates/arroyo-udf
- Kafka sink delivery: exactly-once uses "its transaction API" with two-phase commit per checkpoint epoch. Recovery is not clean when a failure happens during the commit phase. At-least-once mode can produce duplicates. https://doc.arroyo.dev/connectors/kafka
- Latest release is v0.15.0 (2025-12-01).

## Gaps and caveats
- Exa was out of credits, so I used WebSearch, WebFetch and the GitHub/crates.io APIs instead.
- I did not read the full Debezium Server packaging steps for custom sinks. The Quarkus rebuild point is **inference**.
- I did not confirm whether Redpanda Connect dynamic plugins are labelled beta. The docs page does not say.
- The Airbyte custom Docker connector availability by plan comes from page badges only.
