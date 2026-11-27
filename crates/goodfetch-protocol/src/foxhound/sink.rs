//! A crawl's Sinks as settings (ADRs 0026 and 0103): `sinks.<name>.*` is
//! one family per Sink, each entry a [`SinkSettings`] of the keys every
//! Sink type shares, set on a connection or a crawl's Run and refused on a
//! Label. [`GuaranteeClass`] is what a Sink's probe proves of its
//! destination, which its delivery reports; `crawl.min_sink_guarantee`
//! refuses a Run whose Sink proves less, and `sink.probe` probes one Sink
//! on demand.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, Stability, ValueKind, ValueShape, schema_of,
};
use crate::foxhound::export::{ExportCompression, ExportFormat};
use crate::foxhound::report::SinkStatus;
use crate::run::RunId;
use crate::secret_reference::{REFERENCE_PATTERN, SecretReference};
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The catalog destination a Sink delivers to (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SinkType {
    /// PostgreSQL.
    Postgres,
    /// PostgreSQL with pgvector.
    Pgvector,
    /// Supabase.
    Supabase,
    /// MySQL or MariaDB.
    Mysql,
    /// SQL Server.
    Sqlserver,
    /// SQLite, in the binary.
    Sqlite,
    /// MongoDB.
    Mongodb,
    /// Elasticsearch.
    Elasticsearch,
    /// OpenSearch.
    Opensearch,
    /// Redis keys.
    Redis,
    /// Snowflake.
    Snowflake,
    /// BigQuery.
    Bigquery,
    /// Redshift.
    Redshift,
    /// ClickHouse.
    Clickhouse,
    /// Databricks and Delta.
    Databricks,
    /// Iceberg, through its REST catalog.
    Iceberg,
    /// DuckDB.
    Duckdb,
    /// MotherDuck.
    Motherduck,
    /// Qdrant.
    Qdrant,
    /// Pinecone.
    Pinecone,
    /// Weaviate.
    Weaviate,
    /// Milvus.
    Milvus,
    /// Chroma.
    Chroma,
    /// LanceDB.
    Lancedb,
    /// An S3-compatible object store.
    S3,
    /// Google Cloud Storage.
    Gcs,
    /// Azure Blob Storage.
    AzureBlob,
    /// SFTP.
    Sftp,
    /// FTP or FTPS.
    Ftp,
    /// Google Drive.
    GoogleDrive,
    /// Google Sheets.
    GoogleSheets,
    /// Dropbox.
    Dropbox,
    /// OneDrive or SharePoint.
    Onedrive,
    /// Box.
    Box,
    /// Airtable.
    Airtable,
    /// Notion.
    Notion,
    /// Kafka or Redpanda.
    Kafka,
    /// Amazon SQS.
    Sqs,
    /// Amazon SNS.
    Sns,
    /// Amazon Kinesis.
    Kinesis,
    /// Amazon Data Firehose.
    Firehose,
    /// Google Pub/Sub.
    Pubsub,
    /// Azure Service Bus.
    ServiceBus,
    /// Azure Event Hubs.
    EventHubs,
    /// RabbitMQ.
    Rabbitmq,
    /// NATS JetStream.
    Nats,
    /// Redis Streams.
    RedisStreams,
    /// SMTP.
    Smtp,
    /// Amazon SES.
    Ses,
    /// Resend.
    Resend,
    /// Postmark.
    Postmark,
    /// SendGrid.
    Sendgrid,
    /// Mailgun.
    Mailgun,
    /// A signed webhook (Standard Webhooks).
    Webhook,
    /// A Bridge Sink running an Airbyte destination image.
    Airbyte,
    /// A Bridge Sink running a Singer target.
    Singer,
}

/// How a Sink writes the Records it receives (ADR 0026), keyed on its own
/// `key` or else each Dataset's (ADR 0103).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    /// Each Record is added.
    #[default]
    Append,
    /// A Record replaces the one of its key, or is added.
    Upsert,
    /// A Record is added unless one of its key is there.
    InsertOnly,
    /// The Records are written apart, then swapped in at once.
    Replace,
    /// Each Record of a key is kept as a version, valid from and to.
    History,
}

/// What a Sink delivers (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveredAs {
    /// Rows or messages, while the Run goes.
    Records,
    /// Numbered Export parts.
    Files,
    /// Both.
    Both,
}

/// When a Files-mode part rolls (ADR 0026): the first bound reached.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkFilePart {
    /// How many Records a part holds at most: 100,000 when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_records: Option<u64>,
    /// How many bytes a part holds at most: 256 MiB when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_bytes: Option<u64>,
    /// How long, in milliseconds, a part stays open at most: 5 minutes
    /// when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_age: Option<u64>,
}

/// A Sink's Files mode (ADR 0026).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkFiles {
    /// The parts' Export format: JSONL when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: Option<ExportFormat>,
    /// The parts' compression: Zstandard when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub compression: Option<ExportCompression>,
    /// When a part rolls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub part: Option<SinkFilePart>,
}

/// What a Sink does with a field its destination's columns do not hold
/// (ADR 0026, dlt's contracts).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaContract {
    /// The columns grow to hold it.
    #[default]
    Evolve,
    /// The Record is refused.
    Freeze,
    /// The Record is left out.
    DiscardRow,
    /// The field is left out.
    DiscardValue,
}

/// How a Sink's destination schema follows the Records (ADR 0026).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkSchema {
    /// Its contract: `evolve` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub contract: Option<SchemaContract>,
    /// Whether nested values are flattened into dotted columns rather than
    /// kept in the destination's own JSON type: off when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub flatten: Option<bool>,
}

/// A Sink's token bucket, over its destination's preset (ADR 0026).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkRate {
    /// How many writes a second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub per_second: Option<u64>,
    /// How many writes may go at once, the bucket's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub burst: Option<u64>,
}

/// A Sink's batch sizing, over its destination's preset (ADR 0026).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkBatch {
    /// How many Records one write holds at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_records: Option<u64>,
    /// How many bytes one write holds at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_bytes: Option<u64>,
}

/// What a webhook Sink's request carries (ADR 0026).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebhookPayload {
    /// The Records themselves.
    #[default]
    Batch,
    /// A record range and a URL to fetch it from.
    Thin,
}

/// A webhook Sink's keys (ADR 0026).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkWebhook {
    /// What a request carries: `batch` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<WebhookPayload>,
    /// How many bytes of Records one request carries at most: 256 KB when
    /// left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_batch_bytes: Option<u64>,
    /// The Secret reference of the key each request is signed with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub signing_secret: Option<String>,
}

/// A vector store Sink's embedding (ADR 0026).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkEmbedding {
    /// The Model provider's model that embeds the chunks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub model: Option<String>,
    /// The Record's fields whose text is embedded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: Option<Vec<String>>,
    /// How many characters a chunk holds at most: 2,000 when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub chunk_chars: Option<u64>,
}

/// One Sink, the entry of its name in `sinks` (ADRs 0026, 0103 and 0104):
/// the keys every Sink type shares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkSettings {
    /// The catalog destination it delivers to; left out for a program's own
    /// Sink handler of the entry's name.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sink_type: Option<SinkType>,
    /// The names of the Datasets it receives.
    #[schemars(length(min = 1), extend("arrived_in" = "0.1"))]
    pub datasets: Vec<String>,
    /// How it writes them: `append` when left out ([`Self::write_mode`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub mode: Option<WriteMode>,
    /// The fields a keyed mode uses: each Dataset's own key when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: Option<Vec<String>>,
    /// What it delivers: Records for a row or message destination, files
    /// for a file destination, when left out. Spec #582 spells it `as`,
    /// which Python cannot name a key of a `TypedDict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub deliver: Option<DeliveredAs>,
    /// The Secret reference of its DSN, URL or credentials, never the value
    /// itself; required with a `type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub destination: Option<String>,
    /// Its table, collection, index, topic, queue, sheet or object prefix:
    /// the type's own when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: Option<String>,
    /// Its Files mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub files: Option<SinkFiles>,
    /// How its destination's schema follows the Records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schema: Option<SinkSchema>,
    /// Its token bucket: the destination's preset when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rate: Option<SinkRate>,
    /// Its batch sizing: the destination's preset when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub batch: Option<SinkBatch>,
    /// A webhook Sink's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub webhook: Option<SinkWebhook>,
    /// A vector store Sink's embedding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub embedding: Option<SinkEmbedding>,
    /// A Bridge Sink's image digest, which an Operator policy must name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub image: Option<String>,
}

impl SinkSettings {
    /// How it writes its Records: its `mode`, else `append`.
    #[must_use]
    pub fn write_mode(&self) -> WriteMode {
        self.mode.unwrap_or_default()
    }

    /// Reads `value` as a Sink's entry.
    ///
    /// # Errors
    ///
    /// What is wrong with it, never repeating a Secret reference's place's
    /// text, since a value given there may be the secret itself.
    pub fn read(value: &Value) -> Result<Self, String> {
        let settings: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!("expected a Sink's entry of `datasets` and the keys every Sink shares: {error}")
        })?;
        if settings.datasets.is_empty() {
            return Err("a Sink's `datasets` names at least one Dataset".to_owned());
        }
        if settings.sink_type.is_some() && settings.destination.is_none() {
            return Err(
                "a Sink of a `type` names its `destination`, a Secret reference such as `env:SINK_DSN`"
                    .to_owned(),
            );
        }
        reference("destination", settings.destination.as_deref())?;
        reference(
            "webhook.signing_secret",
            settings
                .webhook
                .as_ref()
                .and_then(|webhook| webhook.signing_secret.as_deref()),
        )?;
        Ok(settings)
    }
}

/// Whether `text`, the value of the key `key` names, is a Secret reference
/// when given, or why not, never repeating it.
fn reference(key: &str, text: Option<&str>) -> Result<(), String> {
    match text.map(SecretReference::parse) {
        Some(Err(error)) => Err(format!(
            "a Sink's `{key}` is a Secret reference such as `env:NAME`: {error}; a value is never taken in its place"
        )),
        None | Some(Ok(_)) => Ok(()),
    }
}

impl crate::catalog::SettingJson for SinkSettings {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// A value of `sinks`: one Sink's entry.
pub const SINK_SHAPE: ValueShape = ValueShape {
    name: "SinkSettings",
    check: |value| SinkSettings::read(value).map(drop),
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<SinkSettings>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    // The first gives every key a value: the settings corpus of a
    // format's compatibility set takes a map's entry from it alone.
    examples: || {
        vec![
            json!({
                "type": "webhook",
                "datasets": ["products"],
                "mode": "upsert",
                "key": ["sku"],
                "deliver": "both",
                "destination": "env:HOOK_URL",
                "target": "products",
                "files": {
                    "format": "jsonl",
                    "compression": "zstd",
                    "part": { "max_records": 100_000, "max_bytes": 268_435_456, "max_age": 300_000 },
                },
                "schema": { "contract": "evolve", "flatten": false },
                "rate": { "per_second": 10, "burst": 2 },
                "batch": { "max_records": 100, "max_bytes": 262_144 },
                "webhook": {
                    "payload": "batch",
                    "max_batch_bytes": 262_144,
                    "signing_secret": "env:HOOK_SECRET",
                },
                "embedding": { "model": "openai/text-embedding-3-small", "fields": ["body"], "chunk_chars": 2_000 },
                "image": "airbyte/destination-postgres@sha256:0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0",
            }),
            json!({
                "type": "sqlite",
                "datasets": ["products"],
                "destination": "env:SINK_SQLITE_PATH",
                "target": "products",
            }),
            json!({
                "type": "postgres",
                "datasets": ["products", "reviews"],
                "mode": "upsert",
                "key": ["sku"],
                "deliver": "records",
                "destination": "env:WAREHOUSE_DSN",
                "schema": { "contract": "freeze", "flatten": true },
                "rate": { "per_second": 20, "burst": 5 },
                "batch": { "max_records": 500, "max_bytes": 4_194_304 },
            }),
            json!({
                "type": "s3",
                "datasets": ["products"],
                "deliver": "files",
                "destination": "env:BUCKET_URL",
                "target": "crawls/products/",
                "files": {
                    "format": "jsonl",
                    "compression": "zstd",
                    "part": { "max_records": 100_000, "max_bytes": 268_435_456, "max_age": 300_000 },
                },
            }),
            json!({
                "type": "webhook",
                "datasets": ["products"],
                "destination": "env:HOOK_URL",
                "webhook": {
                    "payload": "thin",
                    "max_batch_bytes": 262_144,
                    "signing_secret": "env:HOOK_SECRET",
                },
            }),
            json!({
                "type": "qdrant",
                "datasets": ["articles"],
                "destination": "env:QDRANT_URL",
                "embedding": { "model": "openai/text-embedding-3-small", "fields": ["body"], "chunk_chars": 2_000 },
            }),
            json!({
                "type": "airbyte",
                "datasets": ["products"],
                "destination": "env:AIRBYTE_CONFIG",
                "image": "airbyte/destination-postgres@sha256:0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0",
            }),
        ]
    },
};

fn no_sinks() -> Value {
    json!({})
}

/// `sinks` (ADRs 0026 and 0103).
pub const SINKS: Setting = Setting {
    name: "sinks",
    description: "The Sinks a crawl's Run delivers its Records to, each by its name, as `sinks.<name>.*`: `type`, the catalog destination, left out for a program's own Sink handler of that name; `datasets`, the Datasets it receives; `mode`, `append` (the default), `upsert`, `insert_only`, `replace` or `history`; `key`, the fields a keyed mode uses, each Dataset's own key when left out; `deliver`, `records`, `files` or `both`; `destination`, the Secret reference of its DSN, URL or credentials, a value given in its place being refused with `setting_invalid`; `target`, its table, collection, index, topic, queue, sheet or object prefix; and `files`, `schema`, `rate`, `batch`, `webhook`, `embedding` and `image`, which only some types take. Every delivery carries its delivery ID, the Record's Dataset and number and the run epoch. None by default. Set on a connection or a crawl's Run, from code, the project file, the CLI or a Recipe; a Label refuses it with `setting_not_applicable`.",
    kind: ValueKind::Map(&ValueKind::Shape(&SINK_SHAPE)),
    default: no_sinks,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What a Sink's probe proves of its destination (ADR 0026), from the
/// strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuaranteeClass {
    /// A stale run epoch is refused in the same transaction as the write.
    Fenced,
    /// A stale run epoch cannot overwrite a Record, but can add one.
    RecordFenced,
    /// A retry lands on the same key, sometimes only within a
    /// deduplication window.
    Keyed,
    /// A Record may arrive more than once; the receiver deduplicates on its
    /// delivery ID.
    AtLeastOnce,
}

/// `crawl.min_sink_guarantee` (ADR 0026).
pub const CRAWL_MIN_SINK_GUARANTEE: Setting = Setting {
    name: "crawl.min_sink_guarantee",
    description: "The weakest Guarantee class a crawl's Run accepts of each of its Sinks, from the strongest: `fenced`, `record_fenced`, `keyed` or `at_least_once` (ADR 0026). Each Sink is probed at the Run's start, and a Sink whose probe proves a weaker class, or fails, refuses the start with `sink_guarantee_unmet` before the Run does any work. None by default: any class is accepted, and a Sink whose probe fails delivers nothing; `null` clears it.",
    kind: ValueKind::Choice(&["fenced", "record_fenced", "keyed", "at_least_once"]),
    default: no_minimum,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn no_minimum() -> Value {
    Value::Null
}

/// A crawl's Run refused at its start for a Sink below its
/// `crawl.min_sink_guarantee` (ADR 0026).
pub const SINK_GUARANTEE_UNMET: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "sink_guarantee_unmet",
    description: "A Sink of a crawl's Run proved a weaker Guarantee class than the Run's `crawl.min_sink_guarantee`, or its probe failed, so the Run was refused at its start and did no work. The details name the Sink, the class required, the class its probe proved, if any, and the cause of a failed probe. Deliver to a destination that proves the class, such as a Fenced SQLite Sink, or lower `crawl.min_sink_guarantee`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `sink_guarantee_unmet` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinkGuaranteeUnmetDetails {
    /// The Sink's name, its key under `sinks`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sink: String,
    /// The class the Run requires, its `crawl.min_sink_guarantee`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub required: GuaranteeClass,
    /// The class the Sink's probe proved; none when the probe failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proved: Option<GuaranteeClass>,
    /// Why the probe failed, for people; none when it proved a class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cause: Option<String>,
}

/// What `sink.probe` takes: the Sink, by its name among the `sinks` the
/// settings resolve to as a crawl's Run would.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkProbeParams {
    /// The Sink's name, its key under `sinks`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sink: String,
    /// Settings over the connection's, nested by key, as a crawl's Run
    /// takes them (ADR 0083), such as the `sinks` to probe.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// What `sink.probe` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinkProbe {
    /// The Sink's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sink: String,
    /// What its probe found.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub outcome: SinkProbeOutcome,
}

/// What a Sink's probe found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SinkProbeOutcome {
    /// Its destination proved a Guarantee class.
    Proved {
        /// The class.
        #[schemars(extend("arrived_in" = "0.1"))]
        class: GuaranteeClass,
    },
    /// Its probe failed: its destination did not resolve, open or answer.
    Failed {
        /// Why, for people.
        #[schemars(extend("arrived_in" = "0.1"))]
        cause: String,
    },
}

/// `sink.probe` (ADR 0026).
pub const SINK_PROBE: Operation = Operation {
    name: "sink.probe",
    description: "Probes one Sink now, by its name among the `sinks` the connection's settings and the call's `settings` resolve to, as a crawl's Run would, and answers the Guarantee class its destination proves: `fenced`, `record_fenced`, `keyed` or `at_least_once` (ADR 0026). A probe may create what the Sink writes to, such as a SQLite Sink's table. A probe that fails answers `failed` with its cause, such as a destination that did not resolve or open; a Sink the settings do not name answers `failed` naming those they do. A Sink this engine does not deliver to yet fails with `capability_unavailable`. A crawl's Run probes each of its Sinks again at its start.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SinkProbeParams>,
    result: schema_of::<SinkProbe>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `sink.list` takes: optionally one Run to read each Sink's status
/// from, or settings over the connection's to probe and summarize every
/// configured Sink (ADRs 0026 and 0099).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkListParams {
    /// When set, lists the Sinks that applied to this Run with its
    /// Run-scoped class, status, dead letters and deliveries outside the
    /// deduplication window. When absent, lists the Sinks configured in
    /// `settings` and aggregates their dead letters and late deliveries
    /// across Runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// Settings over the connection's, nested by key, as a crawl's Run
    /// takes them (ADR 0083).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// One Sink in `sink.list`'s answer (ADR 0026).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinkListEntry {
    /// The Sink's name, its key under `sinks`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its Guarantee class, when probed or recorded on a Run; none when
    /// probing failed outside any Run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub guarantee_class: Option<GuaranteeClass>,
    /// How many deliveries are currently on its dead-letter list.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dead_letters: u64,
    /// How many deliveries fell outside the destination's deduplication
    /// window, and so were at least once.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub outside_dedup_window: u64,
    /// Its delivery status on the named Run; absent when listing
    /// configured Sinks outside a single Run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: Option<SinkStatus>,
    /// Why probing failed outside a Run, when `guarantee_class` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub probe_cause: Option<String>,
}

/// What `sink.list` answers (ADR 0026).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinkList {
    /// Each Sink, in name order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sinks: Vec<SinkListEntry>,
}

/// `sink.list` (ADRs 0026 and 0099).
pub const SINK_LIST: Operation = Operation {
    name: "sink.list",
    description: "Lists each Sink with its Guarantee class, its dead-letter count and its deliveries outside the destination's deduplication window (ADR 0026). With `run_id`, reads the Sinks that applied to that Run; without `run_id`, resolves `sinks` from the connection's and call's settings, probes each deliverable Sink and aggregates its dead letters and late deliveries across Runs.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SinkListParams>,
    result: schema_of::<SinkList>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A crawl's Sinks' entries: their settings, `sink.probe`, `sink.list` and the
/// refusal of a Run whose Sink proves too little.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[SINK_PROBE, SINK_LIST],
    events: &[],
    handler_requests: &[],
    error_codes: &[SINK_GUARANTEE_UNMET],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[SINKS, CRAWL_MIN_SINK_GUARANTEE],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
