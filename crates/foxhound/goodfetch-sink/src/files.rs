//! Files mode for Sinks (ADR 0026; issue 1152): buffers committed Records per
//! Dataset and writes numbered Export parts rolled by `max_records` (default
//! 100,000), `max_bytes` (default 256 MiB) or `max_age` (default 5 min), with
//! a final part written on [`Primitives::finish`]. Every part is a valid
//! Export in `files.format` and `files.compression`, carries the union of
//! fields seen so far for table formats, and its key carries the delivery ID
//! and run epoch.

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_dataset::{ExportOptions, PartColumns, PartRecord, WriteExportError};
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_protocol::foxhound::sink::SinkFiles;
use tokio::time::Instant;

use crate::contract::{
    Batch, Delivered, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};

/// Default maximum Records per part (100,000; ADR 0026).
pub const DEFAULT_MAX_RECORDS: usize = 100_000;

/// Default maximum bytes per part (256 MiB; ADR 0026).
pub const DEFAULT_MAX_BYTES: usize = 256 * 1024 * 1024;

/// Default maximum age of an open part (5 minutes; ADR 0026).
pub const DEFAULT_MAX_AGE: Duration = Duration::from_mins(5);

/// Configuration for Files-mode part rolling and Export encoding.
#[derive(Clone, Debug)]
pub struct FilesConfig {
    /// Export format of each part (`jsonl` by default).
    pub format: ExportFormat,
    /// Export compression of each part (`zstd` by default).
    pub compression: ExportCompression,
    /// Maximum Records per part (at least 1).
    pub max_records: usize,
    /// Maximum encoded/buffered bytes per part (at least 1).
    pub max_bytes: usize,
    /// Maximum age of an open part before rolling.
    pub max_age: Duration,
    /// Format-specific Export options.
    pub export_options: ExportOptions,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            format: ExportFormat::Jsonl,
            compression: ExportCompression::Zstd,
            max_records: DEFAULT_MAX_RECORDS,
            max_bytes: DEFAULT_MAX_BYTES,
            max_age: DEFAULT_MAX_AGE,
            export_options: ExportOptions::of("records"),
        }
    }
}

impl FilesConfig {
    /// Builds a [`FilesConfig`] from protocol [`SinkFiles`] settings.
    #[must_use]
    pub fn from_settings(files: Option<&SinkFiles>) -> Self {
        let mut config = Self::default();
        let Some(files) = files else {
            return config;
        };
        if let Some(format) = files.format {
            config.format = format;
        }
        if let Some(compression) = files.compression {
            config.compression = compression;
        }
        if let Some(part) = &files.part {
            if let Some(max_records) = part.max_records {
                config.max_records = usize::try_from(max_records).unwrap_or(usize::MAX).max(1);
            }
            if let Some(max_bytes) = part.max_bytes {
                config.max_bytes = usize::try_from(max_bytes).unwrap_or(usize::MAX).max(1);
            }
            if let Some(max_age_ms) = part.max_age {
                config.max_age = Duration::from_millis(max_age_ms.max(1));
            }
        }
        config
    }
}

/// Probe result returned by a [`FileDestination`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileProbe {
    /// Guarantee class proven by the destination.
    pub class: GuaranteeClass,
    /// Highest committed Record number already durable per Dataset for this
    /// `(run_id, run_epoch)`.
    pub delivered: BTreeMap<String, u64>,
    /// Next part ordinal per Dataset already present for this epoch (so new
    /// parts continue numbering without overwriting prior parts).
    pub next_parts: BTreeMap<String, u64>,
    /// If a newer copy superseded this Run, its epoch.
    pub superseded_by: Option<u64>,
}

/// Outcome of writing one part to a [`FileDestination`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutOutcome {
    /// The part is durable and atomically visible at its final key.
    Written,
    /// A newer copy of the Run superseded this copy; the part was not written.
    Superseded {
        /// The superseding run epoch.
        run_epoch: u64,
    },
}

/// Destination backend for [`FilesSink`] (object stores, SFTP, FTP).
pub trait FileDestination: Send {
    /// Destination-specific error type before translation.
    type Error: Send;

    /// Probes the destination for guarantee class, fence state and existing
    /// committed parts of `run`.
    ///
    /// # Errors
    ///
    /// Destination-specific probe error.
    fn probe(&mut self, run: &RunCopy) -> Result<FileProbe, Self::Error>;

    /// Atomically publishes one complete Export part at `key`. An interrupted
    /// upload must never leave a partial file visible at `key`.
    ///
    /// # Errors
    ///
    /// Destination-specific write error.
    fn put_part(
        &mut self,
        key: &str,
        bytes: Vec<u8>,
        run: &RunCopy,
    ) -> Result<PutOutcome, Self::Error>;

    /// Translates a destination error into the driver's [`Failure`].
    fn map_failure(&self, error: Self::Error) -> Failure;
}

/// Error returned by [`FilesSink`] before translation.
#[derive(Debug)]
pub enum FilesError<E> {
    /// Encoding a part as an Export failed (e.g. XLSX sheet capacity).
    Encode(WriteExportError),
    /// The underlying destination returned an error.
    Destination(E),
}

struct PartBuffer {
    records: Vec<Delivered>,
    opened_at: Option<Instant>,
    estimated_bytes: usize,
    next_part: u64,
    columns: PartColumns,
}

impl PartBuffer {
    fn new(config: &FilesConfig) -> Self {
        Self {
            records: Vec::new(),
            opened_at: None,
            estimated_bytes: 0,
            next_part: 1,
            columns: PartColumns::new(config.format, &config.export_options),
        }
    }

    fn is_expired(&self, now: Instant, max_age: Duration) -> bool {
        self.opened_at
            .is_some_and(|opened| now.saturating_duration_since(opened) >= max_age)
    }
}

/// A Files-mode Sink over any [`FileDestination`], implementing [`Primitives`].
pub struct FilesSink<D: FileDestination> {
    sink_name: String,
    prefix: String,
    config: FilesConfig,
    clock: Clock,
    destination: D,
    buffers: BTreeMap<String, PartBuffer>,
    written_keys: Vec<String>,
}

impl<D: FileDestination> FilesSink<D> {
    /// Creates a new Files-mode Sink writing under `prefix` into `destination`.
    #[must_use]
    pub fn new(
        sink_name: impl Into<String>,
        prefix: impl Into<String>,
        config: FilesConfig,
        clock: Clock,
        destination: D,
    ) -> Self {
        Self {
            sink_name: sink_name.into(),
            prefix: prefix.into().trim_matches('/').to_owned(),
            config,
            clock,
            destination,
            buffers: BTreeMap::new(),
            written_keys: Vec::new(),
        }
    }

    /// Returns the keys of all parts published so far in this process.
    #[must_use]
    pub fn written_keys(&self) -> &[String] {
        &self.written_keys
    }

    /// Returns a reference to the underlying destination.
    #[must_use]
    pub fn destination(&self) -> &D {
        &self.destination
    }

    /// Returns a mutable reference to the underlying destination.
    pub fn destination_mut(&mut self) -> &mut D {
        &mut self.destination
    }

    fn flush_dataset(
        &mut self,
        dataset: &str,
        run: &RunCopy,
    ) -> Result<Option<u64>, FilesError<D::Error>> {
        let Some(buffer) = self.buffers.get_mut(dataset) else {
            return Ok(None);
        };
        if buffer.records.is_empty() {
            return Ok(None);
        }
        let serialized: Vec<String> = buffer
            .records
            .iter()
            .map(|r| serde_json::to_string(&r.value).unwrap_or_else(|_| "null".to_owned()))
            .collect();
        let part_records: Vec<PartRecord<'_>> = buffer
            .records
            .iter()
            .zip(serialized.iter())
            .map(|(r, value)| PartRecord {
                number: r.number,
                value: value.as_str(),
                committed_at: &r.committed_at,
            })
            .collect();
        buffer.columns.see(&part_records);
        let bytes = buffer
            .columns
            .encode(
                dataset,
                self.config.format,
                &self.config.export_options,
                self.config.compression,
                &part_records,
            )
            .map_err(FilesError::Encode)?;
        let first = &buffer.records[0];
        let last = buffer
            .records
            .last()
            .expect("invariant: buffer is non-empty");
        let key = format_part_key(
            &self.prefix,
            &run.run_id,
            &self.sink_name,
            dataset,
            run.run_epoch,
            buffer.next_part,
            first.delivery_id.as_str(),
            first.number,
            last.number,
            self.config.format,
            self.config.compression,
        );
        match self
            .destination
            .put_part(&key, bytes, run)
            .map_err(FilesError::Destination)?
        {
            PutOutcome::Written => {
                buffer.records.clear();
                buffer.opened_at = None;
                buffer.estimated_bytes = 0;
                buffer.next_part += 1;
                self.written_keys.push(key);
                Ok(None)
            }
            PutOutcome::Superseded { run_epoch } => Ok(Some(run_epoch)),
        }
    }
}

impl<D: FileDestination> Primitives for FilesSink<D> {
    type Error = FilesError<D::Error>;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Self::Error> {
        let probed = self
            .destination
            .probe(run)
            .map_err(FilesError::Destination)?;
        for (dataset, next_part) in probed.next_parts {
            let buffer = self
                .buffers
                .entry(dataset)
                .or_insert_with(|| PartBuffer::new(&self.config));
            buffer.next_part = buffer.next_part.max(next_part);
        }
        Ok(Probe {
            class: probed.class,
            delivered: probed.delivered,
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        let now = self.clock.now();
        let config = self.config.clone();
        self.buffers
            .entry(batch.dataset.clone())
            .or_insert_with(|| PartBuffer::new(&config));

        if self.buffers[&batch.dataset].is_expired(now, self.config.max_age)
            && let Some(superseded) = self.flush_dataset(&batch.dataset, &batch.run)?
        {
            return Ok(Written::Superseded {
                run_epoch: superseded,
            });
        }

        for record in &batch.records {
            let approx = estimate_record_bytes(record);
            let would_exceed_bytes = {
                let buf = &self.buffers[&batch.dataset];
                !buf.records.is_empty() && buf.estimated_bytes + approx > self.config.max_bytes
            };
            if would_exceed_bytes
                && let Some(superseded) = self.flush_dataset(&batch.dataset, &batch.run)?
            {
                return Ok(Written::Superseded {
                    run_epoch: superseded,
                });
            }

            let should_roll = {
                let buf = self
                    .buffers
                    .get_mut(&batch.dataset)
                    .expect("invariant: buffer inserted above");
                if buf.opened_at.is_none() {
                    buf.opened_at = Some(now);
                }
                buf.records.push(record.clone());
                buf.estimated_bytes += approx;
                buf.records.len() >= self.config.max_records
                    || buf.estimated_bytes >= self.config.max_bytes
            };
            if should_roll
                && let Some(superseded) = self.flush_dataset(&batch.dataset, &batch.run)?
            {
                return Ok(Written::Superseded {
                    run_epoch: superseded,
                });
            }
        }

        if self.buffers[&batch.dataset].is_expired(self.clock.now(), self.config.max_age)
            && let Some(superseded) = self.flush_dataset(&batch.dataset, &batch.run)?
        {
            return Ok(Written::Superseded {
                run_epoch: superseded,
            });
        }

        Ok(Written::Delivered)
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        let datasets: Vec<String> = self.buffers.keys().cloned().collect();
        for dataset in datasets {
            if let Some(superseded) = self.flush_dataset(&dataset, run)? {
                return Ok(Written::Superseded {
                    run_epoch: superseded,
                });
            }
        }
        Ok(Written::Delivered)
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        match error {
            FilesError::Encode(err) => Failure::new(FailureKind::Permanent, err.to_string()),
            FilesError::Destination(err) => self.destination.map_failure(err),
        }
    }
}

/// Estimates the uncompressed byte contribution of one Record for `max_bytes`
/// part rolling.
fn estimate_record_bytes(record: &Delivered) -> usize {
    serde_json::to_vec(&record.value).map_or(0, |v| v.len() + 1)
}

/// File extension for a `(format, compression)` Export part.
#[must_use]
pub fn part_extension(format: ExportFormat, compression: ExportCompression) -> String {
    let base = match format {
        ExportFormat::Json => "json",
        ExportFormat::Jsonl => "jsonl",
        ExportFormat::Csv => "csv",
        ExportFormat::Xlsx => "xlsx",
        ExportFormat::HtmlTable => "html",
        ExportFormat::Markdown => "md",
        ExportFormat::Xml => "xml",
        ExportFormat::Parquet => "parquet",
        ExportFormat::Avro => "avro",
        ExportFormat::ArrowIpc => "arrow",
        ExportFormat::Rss => "rss.xml",
        ExportFormat::Atom => "atom.xml",
    };
    match compression {
        ExportCompression::None => base.to_owned(),
        ExportCompression::Gzip => format!("{base}.gz"),
        ExportCompression::Zstd => format!("{base}.zst"),
    }
}

/// Formats a deterministic Files-mode part key carrying the delivery ID, run
/// epoch, dataset, part ordinal and record number range.
#[expect(
    clippy::too_many_arguments,
    reason = "part key combines prefix, run identity, dataset, ordinal, delivery range and Export format"
)]
#[must_use]
pub fn format_part_key(
    prefix: &str,
    run_id: &str,
    sink_name: &str,
    dataset: &str,
    run_epoch: u64,
    part_number: u64,
    first_delivery_id: &str,
    from: u64,
    through: u64,
    format: ExportFormat,
    compression: ExportCompression,
) -> String {
    let delivery_safe = first_delivery_id.replace('/', "_");
    let ext = part_extension(format, compression);
    let rel = format!(
        "{run_id}/{sink_name}/{dataset}/epoch-{run_epoch}/part-{part_number:05}-{delivery_safe}-r{from:010}-{through:010}.{ext}"
    );
    if prefix.is_empty() {
        rel
    } else {
        format!("{prefix}/{rel}")
    }
}

/// Parsed metadata from a part key produced by [`format_part_key`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPartKey {
    /// Dataset name.
    pub dataset: String,
    /// Run epoch.
    pub run_epoch: u64,
    /// 1-based part ordinal.
    pub part_number: u64,
    /// Last Record number included in this part.
    pub through: u64,
}

/// Parses a part key under `{prefix}/{run_id}/{sink_name}/`, returning its
/// dataset, epoch, part ordinal and `through` record number.
#[must_use]
pub fn parse_part_key(
    prefix: &str,
    run_id: &str,
    sink_name: &str,
    key: &str,
) -> Option<ParsedPartKey> {
    let trimmed = key.trim_start_matches('/');
    let base = if prefix.is_empty() {
        format!("{run_id}/{sink_name}/")
    } else {
        format!("{}/{run_id}/{sink_name}/", prefix.trim_matches('/'))
    };
    let rest = trimmed.strip_prefix(&base)?;
    let mut segments = rest.split('/');
    let dataset = segments.next()?.to_owned();
    let epoch_seg = segments.next()?;
    let run_epoch: u64 = epoch_seg.strip_prefix("epoch-")?.parse().ok()?;
    let filename = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let stem = filename.split('.').next()?;
    let after_part = stem.strip_prefix("part-")?;
    let (part_str, tail) = after_part.split_once('-')?;
    let part_number: u64 = part_str.parse().ok()?;
    let range = tail.rsplit_once("-r")?.1;
    let (_from_str, through_str) = range.split_once('-')?;
    let through: u64 = through_str.parse().ok()?;
    Some(ParsedPartKey {
        dataset,
        run_epoch,
        part_number,
        through,
    })
}

/// Builds a [`GuaranteeClass::Keyed`] [`FileProbe`] from the listed relative
/// part keys under `{prefix}/{run_id}/{sink_name}/`.
#[must_use]
pub fn probe_from_keys(
    prefix: &str,
    run: &RunCopy,
    sink_name: &str,
    keys: impl IntoIterator<Item = String>,
) -> FileProbe {
    let mut delivered: BTreeMap<String, u64> = BTreeMap::new();
    let mut next_parts: BTreeMap<String, u64> = BTreeMap::new();
    for key in keys {
        if let Some(parsed) = parse_part_key(prefix, &run.run_id, sink_name, &key)
            && parsed.run_epoch == run.run_epoch
        {
            delivered
                .entry(parsed.dataset.clone())
                .and_modify(|max| *max = (*max).max(parsed.through))
                .or_insert(parsed.through);
            next_parts
                .entry(parsed.dataset)
                .and_modify(|next| *next = (*next).max(parsed.part_number + 1))
                .or_insert(parsed.part_number + 1);
        }
    }
    FileProbe {
        class: GuaranteeClass::Keyed,
        delivered,
        next_parts,
        superseded_by: None,
    }
}
