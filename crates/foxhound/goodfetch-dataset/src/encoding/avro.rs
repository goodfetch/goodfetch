//! Avro: one object container file (Apache Avro 1.12) of one record per
//! Record, uncompressed (`null` codec), in blocks of at most [`BLOCK`]
//! records. Its schema is a record named after the Dataset with one field
//! per column, each a union of `null` and the column's type: `boolean`,
//! `long`, `double`, or `string` holding each cell's text. A name Avro
//! cannot take (a letter or `_`, then letters, digits and `_`) has every
//! other character written `_`, a leading digit given an `_` before it and
//! a name taken twice a `_2`, `_3`… after it; the field's own name is then
//! its `doc`. The sync marker is the first 16 bytes of the schema's
//! SHA-256, so the same Records give the same bytes.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::columns::{Cell, Kind};

/// The most records one block holds.
const BLOCK: u64 = 1024;

/// The writer of one Avro Export.
#[derive(Clone, Debug)]
pub(super) struct Avro {
    kinds: Vec<Kind>,
    sync: [u8; 16],
    header: Vec<u8>,
    block: Vec<u8>,
    count: u64,
}

impl Avro {
    pub(super) fn new(dataset: &str, names: &[String], kinds: &[Kind]) -> Self {
        let mut taken = Vec::new();
        let fields: Vec<Value> = names
            .iter()
            .zip(kinds)
            .map(|(name, kind)| {
                let avro = unique(&avro_name(name), &mut taken);
                let mut field = Map::new();
                field.insert("name".to_owned(), Value::from(avro.clone()));
                field.insert("type".to_owned(), json!(["null", type_of(*kind)]));
                field.insert("default".to_owned(), Value::Null);
                if avro != *name {
                    field.insert("doc".to_owned(), Value::from(name.clone()));
                }
                Value::Object(field)
            })
            .collect();
        let schema = json!({
            "type": "record",
            "name": avro_name(dataset),
            "fields": fields,
        })
        .to_string();
        let digest = Sha256::digest(schema.as_bytes());
        let mut sync = [0; 16];
        sync.copy_from_slice(&digest[..16]);
        let mut header = b"Obj\x01".to_vec();
        long(2, &mut header);
        bytes(b"avro.codec", &mut header);
        bytes(b"null", &mut header);
        bytes(b"avro.schema", &mut header);
        bytes(schema.as_bytes(), &mut header);
        long(0, &mut header);
        header.extend_from_slice(&sync);
        Self {
            kinds: kinds.to_vec(),
            sync,
            header,
            block: Vec::new(),
            count: 0,
        }
    }

    pub(super) fn begin(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.header);
    }

    pub(super) fn row(&mut self, cells: &[Cell], out: &mut Vec<u8>) {
        for (cell, kind) in cells.iter().zip(&self.kinds) {
            if *cell == Cell::Empty {
                long(0, &mut self.block);
                continue;
            }
            long(1, &mut self.block);
            match (kind, cell) {
                (Kind::Boolean, Cell::Boolean(boolean)) => self.block.push(u8::from(*boolean)),
                (Kind::Integer, Cell::Number(text)) => long(
                    text.parse()
                        .expect("invariant: an integer column holds 64-bit integers"),
                    &mut self.block,
                ),
                (Kind::Float, Cell::Number(text)) => self.block.extend_from_slice(
                    &text
                        .parse::<f64>()
                        .expect("invariant: a float column holds numbers")
                        .to_le_bytes(),
                ),
                _ => bytes(cell.text().as_bytes(), &mut self.block),
            }
        }
        self.count += 1;
        if self.count == BLOCK {
            self.flush(out);
        }
    }

    pub(super) fn end(&mut self, out: &mut Vec<u8>) {
        self.flush(out);
    }

    fn flush(&mut self, out: &mut Vec<u8>) {
        if self.count == 0 {
            return;
        }
        long(
            i64::try_from(self.count).expect("invariant: a block's count is small"),
            out,
        );
        long(
            i64::try_from(self.block.len()).expect("invariant: a block's size fits"),
            out,
        );
        out.append(&mut self.block);
        out.extend_from_slice(&self.sync);
        self.count = 0;
    }
}

fn type_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Boolean => "boolean",
        Kind::Integer => "long",
        Kind::Float => "double",
        Kind::Unseen | Kind::Text => "string",
    }
}

/// `name` made an Avro name.
fn avro_name(name: &str) -> String {
    let mut avro: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if !avro.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_') {
        avro.insert(0, '_');
    }
    avro
}

/// `name`, or with `_2`, `_3`… after it when `taken` holds it.
fn unique(name: &str, taken: &mut Vec<String>) -> String {
    let mut candidate = name.to_owned();
    let mut suffix = 2;
    while taken.contains(&candidate) {
        candidate = format!("{name}_{suffix}");
        suffix += 1;
    }
    taken.push(candidate.clone());
    candidate
}

/// Writes `value` as Avro writes a `long`: zig-zag, then 7 bits a byte.
fn long(value: i64, out: &mut Vec<u8>) {
    #[expect(
        clippy::cast_sign_loss,
        reason = "zig-zag encoding reinterprets the shifted bits, as the Avro specification defines"
    )]
    let mut rest = ((value << 1) ^ (value >> 63)) as u64;
    while rest >= 0x80 {
        out.push(u8::try_from(rest & 0x7f).expect("invariant: seven bits") | 0x80);
        rest >>= 7;
    }
    out.push(u8::try_from(rest).expect("invariant: below 0x80"));
}

/// Writes `value` as Avro writes `bytes` and a `string`: its length, then
/// itself.
fn bytes(value: &[u8], out: &mut Vec<u8>) {
    long(
        i64::try_from(value.len()).expect("invariant: a value's length fits"),
        out,
    );
    out.extend_from_slice(value);
}
