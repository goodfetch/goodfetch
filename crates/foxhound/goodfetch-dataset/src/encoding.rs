//! How an Export's bytes are written from its Records (ADRs 0103 and 0119):
//! each Record's value as its JSON text was committed, so nothing is
//! normalized or repaired and ADR 0073's Unicode record round-trips byte
//! for byte; UTF-8 with no BOM, characters written as themselves, since
//! the committed text already escapes only what JSON requires. `json` is
//! one array of the values, `[` then each value after a `,` but the first,
//! then `]`; `jsonl` one value per line, each ended by a line feed. Pure:
//! it writes into a buffer and does no I/O.

use goodfetch_protocol::foxhound::export::ExportFormat;

/// The writer of one Export's bytes, value by value.
#[derive(Clone, Copy, Debug)]
pub struct ExportEncoder {
    format: ExportFormat,
    values: u64,
}

impl ExportEncoder {
    /// The writer of an Export in `format`, which holds no value yet.
    #[must_use]
    pub fn new(format: ExportFormat) -> Self {
        Self { format, values: 0 }
    }

    /// Writes what comes before the first value into `out`.
    pub fn begin(&self, out: &mut Vec<u8>) {
        match self.format {
            ExportFormat::Json => out.push(b'['),
            ExportFormat::Jsonl => {}
        }
    }

    /// Writes the next value, a Record's JSON text as committed, into
    /// `out`.
    pub fn value(&mut self, json: &str, out: &mut Vec<u8>) {
        match self.format {
            ExportFormat::Json => {
                if self.values > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(json.as_bytes());
            }
            ExportFormat::Jsonl => {
                out.extend_from_slice(json.as_bytes());
                out.push(b'\n');
            }
        }
        self.values += 1;
    }

    /// Writes what comes after the last value into `out`.
    pub fn end(&self, out: &mut Vec<u8>) {
        match self.format {
            ExportFormat::Json => out.push(b']'),
            ExportFormat::Jsonl => {}
        }
    }

    /// How many values it wrote.
    #[must_use]
    pub fn values(&self) -> u64 {
        self.values
    }
}

#[cfg(test)]
mod tests;
