//! Built-in Sink destinations opened by a Run or on-demand Sink operation
//! (`sqlite` and `webhook`, ADRs 0026 and 0103).

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::secrets::{Purpose, RunSecrets};
use goodfetch_engine::tasks::TaskStarter;
use goodfetch_engine::webhook_signing::{Signer, SigningKey};
use goodfetch_protocol::foxhound::sink::{SINKS, SinkSettings, SinkType, WriteMode};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::SettingsLayer;
use rusqlite::Connection;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::contract::{Batch, Failure, Primitives, Probe, RunCopy, Written};
use crate::driver::Driver;
use crate::duckdb_sink::{DEFAULT_DUCKDB_TARGET, DuckDbError, DuckDbSink};
use crate::files::{FilesConfig, FilesError, FilesSink};
use crate::ftp::{FtpDestination, FtpSinkError};
use crate::mysql::{DEFAULT_MYSQL_TARGET, MySqlError, MySqlSink};
use crate::object_store_sink::{ObjectStoreBackend, ObjectStoreDestination, ObjectStoreError};
use crate::postgres::{DEFAULT_POSTGRES_TARGET, PostgresError, PostgresSink};
use crate::sftp::{SftpDestination, SftpError};
use crate::smtp_sink::{SmtpError, SmtpSink};
use crate::sqlite::{SqliteError, SqliteSink};
use crate::throughput::Throughput;
use crate::webhook::{DEFAULT_MAX_BATCH_BYTES, WebhookConfig, WebhookError, WebhookSink};

/// The table a SQLite Sink writes when its `target` names none.
const SQLITE_TARGET: &str = "records";

/// Any in-binary Sink destination the runtime opens directly.
pub(super) enum BuiltInSink {
    Sqlite(SqliteSink),
    Webhook(WebhookSink),
    ObjectStore(FilesSink<ObjectStoreDestination>),
    Postgres(PostgresSink),
    MySql(MySqlSink),
    DuckDb(DuckDbSink),
    Smtp(SmtpSink),
    Sftp(FilesSink<SftpDestination>),
    Ftp(FilesSink<FtpDestination>),
}

/// Errors from any [`BuiltInSink`].
pub(super) enum BuiltInError {
    Sqlite(SqliteError),
    Webhook(WebhookError),
    ObjectStore(FilesError<ObjectStoreError>),
    Postgres(PostgresError),
    MySql(MySqlError),
    DuckDb(DuckDbError),
    Smtp(SmtpError),
    Sftp(FilesError<SftpError>),
    Ftp(FilesError<FtpSinkError>),
}

impl Primitives for BuiltInSink {
    type Error = BuiltInError;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, BuiltInError> {
        match self {
            Self::Sqlite(sink) => sink.probe(run).map_err(BuiltInError::Sqlite),
            Self::Webhook(sink) => sink.probe(run).map_err(BuiltInError::Webhook),
            Self::ObjectStore(sink) => sink.probe(run).map_err(BuiltInError::ObjectStore),
            Self::Postgres(sink) => sink.probe(run).map_err(BuiltInError::Postgres),
            Self::MySql(sink) => sink.probe(run).map_err(BuiltInError::MySql),
            Self::DuckDb(sink) => sink.probe(run).map_err(BuiltInError::DuckDb),
            Self::Smtp(sink) => sink.probe(run).map_err(BuiltInError::Smtp),
            Self::Sftp(sink) => sink.probe(run).map_err(BuiltInError::Sftp),
            Self::Ftp(sink) => sink.probe(run).map_err(BuiltInError::Ftp),
        }
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, BuiltInError> {
        match self {
            Self::Sqlite(sink) => sink.write(batch).map_err(BuiltInError::Sqlite),
            Self::Webhook(sink) => sink.write(batch).map_err(BuiltInError::Webhook),
            Self::ObjectStore(sink) => sink.write(batch).map_err(BuiltInError::ObjectStore),
            Self::Postgres(sink) => sink.write(batch).map_err(BuiltInError::Postgres),
            Self::MySql(sink) => sink.write(batch).map_err(BuiltInError::MySql),
            Self::DuckDb(sink) => sink.write(batch).map_err(BuiltInError::DuckDb),
            Self::Smtp(sink) => sink.write(batch).map_err(BuiltInError::Smtp),
            Self::Sftp(sink) => sink.write(batch).map_err(BuiltInError::Sftp),
            Self::Ftp(sink) => sink.write(batch).map_err(BuiltInError::Ftp),
        }
    }

    fn write_with_timeout(
        &mut self,
        batch: &Batch,
        timeout: Duration,
    ) -> Result<Written, BuiltInError> {
        match self {
            Self::Sqlite(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::Sqlite),
            Self::Webhook(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::Webhook),
            Self::ObjectStore(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::ObjectStore),
            Self::Postgres(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::Postgres),
            Self::MySql(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::MySql),
            Self::DuckDb(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::DuckDb),
            Self::Smtp(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::Smtp),
            Self::Sftp(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::Sftp),
            Self::Ftp(sink) => sink
                .write_with_timeout(batch, timeout)
                .map_err(BuiltInError::Ftp),
        }
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, BuiltInError> {
        match self {
            Self::Sqlite(sink) => sink.finish(run).map_err(BuiltInError::Sqlite),
            Self::Webhook(sink) => sink.finish(run).map_err(BuiltInError::Webhook),
            Self::ObjectStore(sink) => sink.finish(run).map_err(BuiltInError::ObjectStore),
            Self::Postgres(sink) => sink.finish(run).map_err(BuiltInError::Postgres),
            Self::MySql(sink) => sink.finish(run).map_err(BuiltInError::MySql),
            Self::DuckDb(sink) => sink.finish(run).map_err(BuiltInError::DuckDb),
            Self::Smtp(sink) => sink.finish(run).map_err(BuiltInError::Smtp),
            Self::Sftp(sink) => sink.finish(run).map_err(BuiltInError::Sftp),
            Self::Ftp(sink) => sink.finish(run).map_err(BuiltInError::Ftp),
        }
    }

    fn dedup_window(&self) -> Option<Duration> {
        match self {
            Self::Sqlite(sink) => sink.dedup_window(),
            Self::Webhook(sink) => sink.dedup_window(),
            Self::ObjectStore(sink) => sink.dedup_window(),
            Self::Postgres(sink) => sink.dedup_window(),
            Self::MySql(sink) => sink.dedup_window(),
            Self::DuckDb(sink) => sink.dedup_window(),
            Self::Smtp(sink) => sink.dedup_window(),
            Self::Sftp(sink) => sink.dedup_window(),
            Self::Ftp(sink) => sink.dedup_window(),
        }
    }

    fn map_failure(&self, error: BuiltInError) -> Failure {
        match (self, error) {
            (Self::Sqlite(sink), BuiltInError::Sqlite(err)) => sink.map_failure(err),
            (Self::Webhook(sink), BuiltInError::Webhook(err)) => sink.map_failure(err),
            (Self::ObjectStore(sink), BuiltInError::ObjectStore(err)) => sink.map_failure(err),
            (Self::Postgres(sink), BuiltInError::Postgres(err)) => sink.map_failure(err),
            (Self::MySql(sink), BuiltInError::MySql(err)) => sink.map_failure(err),
            (Self::DuckDb(sink), BuiltInError::DuckDb(err)) => sink.map_failure(err),
            (Self::Smtp(sink), BuiltInError::Smtp(err)) => sink.map_failure(err),
            (Self::Sftp(sink), BuiltInError::Sftp(err)) => sink.map_failure(err),
            (Self::Ftp(sink), BuiltInError::Ftp(err)) => sink.map_failure(err),
            _ => unreachable!("invariant: error matches its BuiltInSink variant"),
        }
    }
}

/// A Sink's destination, as its entry names it.
pub(super) struct Destination<'a> {
    pub(super) name: &'a str,
    pub(super) settings: &'a SinkSettings,
    /// The layer that set its entry.
    pub(super) layer: SettingsLayer,
    /// Per-write timeout (`timeout.sink_write`, ADR 0080).
    pub(super) write_timeout: Duration,
    /// The Run it delivers for; none for a probe outside any Run.
    pub(super) run_id: Option<&'a RunId>,
    /// Operator-configured HTTP endpoint for webhook Sinks.
    pub(super) endpoint: &'a NotificationEndpoint,
}

impl Destination<'_> {
    /// Opens the configured built-in Sink driver, resolving any Secret
    /// references through `secrets`.
    pub(super) async fn open_with_epoch(
        &self,
        secrets: &RunSecrets,
        owner: &TaskStarter,
        clock: Clock,
        mut run: RunCopy,
        keep_destination_epoch: bool,
    ) -> Result<Driver<BuiltInSink>, String> {
        let resolved_dest = self
            .resolve_secret(
                self.settings.destination.as_deref().expect(
                    "invariant: `SinkSettings::read` refuses a typed Sink with no `destination`",
                ),
                "destination",
                secrets,
            )
            .await?
            .expose_secret()
            .to_owned();
        let mode = self.settings.mode.unwrap_or_default();
        let mut throughput = Throughput::from_settings(
            self.settings.sink_type,
            self.settings.rate.as_ref(),
            self.settings.batch.as_ref(),
            self.write_timeout,
        );
        let built_in = self
            .open_built_in(
                resolved_dest,
                mode,
                secrets,
                owner,
                clock,
                &mut run,
                &mut throughput,
                keep_destination_epoch,
            )
            .await?;
        let driver = Driver::new(self.name, built_in, run, self.settings.datasets.clone())
            .with_clock(clock)
            .with_masks(secrets.mask().masks().clone())
            .with_throughput(throughput);
        Ok(if mode == WriteMode::Append {
            driver
        } else {
            driver.keyed(self.settings.key.clone())
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "internal factory split from open_with_epoch"
    )]
    async fn open_built_in(
        &self,
        resolved_dest: String,
        mode: WriteMode,
        secrets: &RunSecrets,
        owner: &TaskStarter,
        clock: Clock,
        run: &mut RunCopy,
        throughput: &mut Throughput,
        keep_destination_epoch: bool,
    ) -> Result<BuiltInSink, String> {
        let schema_cfg = self.settings.schema.as_ref();
        let contract = schema_cfg.and_then(|s| s.contract).unwrap_or_default();
        let flatten = schema_cfg.and_then(|s| s.flatten).unwrap_or(false);
        match self.settings.sink_type {
            Some(SinkType::Sqlite) => {
                let (sqlite, existing_epoch) = self
                    .open_sqlite(
                        resolved_dest,
                        mode,
                        &run.run_id,
                        keep_destination_epoch,
                        owner,
                    )
                    .await?;
                if let Some(existing) = existing_epoch {
                    run.run_epoch = existing;
                }
                Ok(BuiltInSink::Sqlite(sqlite))
            }
            Some(SinkType::Webhook) => {
                let (webhook, max_bytes) =
                    self.open_webhook(&resolved_dest, secrets, clock).await?;
                throughput.batch.max_bytes = throughput.batch.max_bytes.min(max_bytes);
                Ok(BuiltInSink::Webhook(webhook))
            }
            Some(
                sink_type @ (SinkType::S3
                | SinkType::Gcs
                | SinkType::AzureBlob
                | SinkType::Sftp
                | SinkType::Ftp),
            ) => self.open_files_sink(sink_type, &resolved_dest, clock),
            Some(SinkType::Postgres | SinkType::Supabase) => {
                let table = self
                    .settings
                    .target
                    .as_deref()
                    .unwrap_or(DEFAULT_POSTGRES_TARGET);
                let pg = PostgresSink::new(resolved_dest, table, mode, contract, flatten)
                    .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::Postgres(pg))
            }
            Some(SinkType::Mysql) => {
                let table = self
                    .settings
                    .target
                    .as_deref()
                    .unwrap_or(DEFAULT_MYSQL_TARGET);
                let mysql = MySqlSink::new(resolved_dest, table, mode, contract, flatten)
                    .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::MySql(mysql))
            }
            Some(SinkType::Duckdb) => {
                let table = self
                    .settings
                    .target
                    .as_deref()
                    .unwrap_or(DEFAULT_DUCKDB_TARGET);
                let duck = DuckDbSink::open(&resolved_dest, table, mode, contract, flatten)
                    .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::DuckDb(duck))
            }
            Some(SinkType::Smtp) => {
                let smtp = SmtpSink::from_url(
                    self.name,
                    &resolved_dest,
                    self.settings.target.as_deref(),
                    self.endpoint.clone(),
                )
                .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::Smtp(smtp))
            }
            other => Err(format!(
                "Sink type `{other:?}` is not deliverable by this engine"
            )),
        }
    }

    fn open_files_sink(
        &self,
        sink_type: SinkType,
        resolved_dest: &str,
        clock: Clock,
    ) -> Result<BuiltInSink, String> {
        let prefix = self.settings.target.as_deref().unwrap_or_default();
        let config = FilesConfig::from_settings(self.settings.files.as_ref());
        match sink_type {
            SinkType::S3 | SinkType::Gcs | SinkType::AzureBlob => {
                let backend = match sink_type {
                    SinkType::S3 => ObjectStoreBackend::S3,
                    SinkType::Gcs => ObjectStoreBackend::Gcs,
                    SinkType::AzureBlob => ObjectStoreBackend::AzureBlob,
                    _ => unreachable!("invariant: matched above"),
                };
                let dest = ObjectStoreDestination::from_url(
                    backend,
                    self.name,
                    resolved_dest,
                    self.settings.target.as_deref(),
                )
                .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::ObjectStore(FilesSink::new(
                    self.name, prefix, config, clock, dest,
                )))
            }
            SinkType::Sftp => {
                let dest = SftpDestination::from_url(self.name, resolved_dest, Some(prefix))
                    .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::Sftp(FilesSink::new(
                    self.name, prefix, config, clock, dest,
                )))
            }
            SinkType::Ftp => {
                let dest = FtpDestination::from_url(self.name, resolved_dest, Some(prefix))
                    .map_err(|err| err.to_string())?;
                Ok(BuiltInSink::Ftp(FilesSink::new(
                    self.name, prefix, config, clock, dest,
                )))
            }
            _ => unreachable!("invariant: open_files_sink called only for files SinkTypes"),
        }
    }

    pub(super) async fn open(
        &self,
        secrets: &RunSecrets,
        owner: &TaskStarter,
        clock: Clock,
        run: RunCopy,
    ) -> Result<Driver<BuiltInSink>, String> {
        self.open_with_epoch(secrets, owner, clock, run, false)
            .await
    }

    async fn resolve_secret(
        &self,
        raw_reference: &str,
        key_suffix: &str,
        secrets: &RunSecrets,
    ) -> Result<SecretString, String> {
        let reference = SecretReference::parse(raw_reference)
            .map_err(|error| format!("its `{key_suffix}` is no valid Secret reference: {error}"))?;
        let placeholder = format!("{}.{}.{key_suffix}", SINKS.name, self.name);
        let purpose = Purpose {
            placeholder: &placeholder,
            domain: None,
            domains: &[],
            layer: Some(self.layer),
            run_id: self.run_id,
        };
        secrets
            .resolve(&reference, &purpose)
            .await
            .map_err(|error| {
                format!("its `{key_suffix}` (`{raw_reference}`) did not resolve: {error}")
            })
    }

    async fn open_sqlite(
        &self,
        path: String,
        mode: WriteMode,
        run_id: &str,
        keep_destination_epoch: bool,
        owner: &TaskStarter,
    ) -> Result<(SqliteSink, Option<u64>), String> {
        let target = self
            .settings
            .target
            .as_deref()
            .unwrap_or(SQLITE_TARGET)
            .to_owned();
        let schema = self.settings.schema.clone().unwrap_or_default();
        let run_id_owned = run_id.to_owned();
        owner
            .blocking(move || {
                let connection = Connection::open(path)?;
                let sqlite = SqliteSink::new(connection, &target)
                    .with_mode(mode)
                    .with_schema(schema);
                let existing = keep_destination_epoch
                    .then(|| sqlite.current_epoch(&run_id_owned))
                    .flatten();
                Ok::<_, rusqlite::Error>((sqlite, existing))
            })
            .await
            .map_err(|error| {
                format!(
                    "its SQLite database did not open ({:?})",
                    error.sqlite_error_code()
                )
            })
    }

    async fn open_webhook(
        &self,
        url_text: &str,
        secrets: &RunSecrets,
        clock: Clock,
    ) -> Result<(WebhookSink, usize), String> {
        let url =
            Url::parse(url_text).map_err(|error| format!("its webhook URL is invalid: {error}"))?;
        let webhook_settings = self.settings.webhook.as_ref();
        let secret_ref = webhook_settings
            .and_then(|w| w.signing_secret.as_deref())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "its `webhook.signing_secret` is missing".to_owned())?;
        let signing_secret = self
            .resolve_secret(secret_ref, "webhook.signing_secret", secrets)
            .await?;
        let key = SigningKey::from_secret(&signing_secret)
            .map_err(|error| format!("its `webhook.signing_secret` is invalid: {error}"))?;
        let signer = Signer::new(vec![key], None)
            .map_err(|error| format!("its webhook signer did not build: {error}"))?;
        let max_batch_bytes = webhook_settings
            .and_then(|w| w.max_batch_bytes)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(DEFAULT_MAX_BATCH_BYTES)
            .max(1);
        let config = WebhookConfig {
            payload: webhook_settings.and_then(|w| w.payload).unwrap_or_default(),
            max_batch_bytes,
            ..WebhookConfig::default()
        };
        let sink = WebhookSink::new(self.name, url, signer, self.endpoint.clone(), clock, config)
            .map_err(|error| error.to_string())?;
        Ok((sink, max_batch_bytes))
    }
}
