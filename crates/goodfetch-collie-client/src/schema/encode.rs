//! The checked serializer a sent value goes through: `serde_json`'s value
//! serializer, but one that refuses what JSON cannot carry, naming its JSON
//! Pointer, where `serde_json` turns NaN and the infinities into `null`
//! and a key `1` into `"1"`. It tags an enum's variants as `serde_json`
//! does, so a value reads back through `serde_json` as it would have.

use std::fmt::{self, Display};

use serde::ser::{self, Impossible, Serialize};
use serde_json::{Map, Number, Value};

use super::SchemaError;

/// `value` as JSON, or where it first holds something that is not.
pub(super) fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<Value, SchemaError> {
    value
        .serialize(Encoder {
            path: String::new(),
        })
        .map_err(|refused| SchemaError::NotJson {
            path: refused.path.unwrap_or_default(),
            found: refused.found,
        })
}

/// Where `read`, the JSON of the value read back, first differs from
/// `sent`, and what it holds there (`null` for a member it lacks).
pub(super) fn first_change(sent: &Value, read: &Value) -> Option<(String, Value)> {
    match (sent, read) {
        (Value::Object(sent), Value::Object(read)) => {
            for (key, member) in sent {
                let Some(read_member) = read.get(key) else {
                    return Some((token(key), Value::Null));
                };
                if let Some((path, found)) = first_change(member, read_member) {
                    return Some((format!("{}{path}", token(key)), found));
                }
            }
            read.iter()
                .find(|(key, _)| !sent.contains_key(*key))
                .map(|(key, member)| (token(key), member.clone()))
        }
        (Value::Array(sent), Value::Array(read)) if sent.len() == read.len() => sent
            .iter()
            .zip(read)
            .enumerate()
            .find_map(|(index, (item, read_item))| {
                first_change(item, read_item)
                    .map(|(path, found)| (format!("/{index}{path}"), found))
            }),
        _ if sent == read => None,
        _ => Some((String::new(), read.clone())),
    }
}

/// The kind of a JSON value, as a message names it.
pub(super) fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// `key` as one step of a JSON Pointer (RFC 6901).
fn token(key: &str) -> String {
    format!("/{}", key.replace('~', "~0").replace('/', "~1"))
}

/// What the serializer refused, and where once known: a `Serialize` that
/// fails says what, and the member it was serializing says where.
#[derive(Debug)]
struct Refused {
    path: Option<String>,
    found: String,
}

impl Refused {
    fn at(path: &str, found: String) -> Self {
        Self {
            path: Some(path.to_owned()),
            found,
        }
    }

    /// Places a refusal that does not know where it is at `path`.
    fn within(mut self, path: &str) -> Self {
        self.path.get_or_insert_with(|| path.to_owned());
        self
    }
}

impl Display for Refused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.found)
    }
}

impl std::error::Error for Refused {}

impl ser::Error for Refused {
    fn custom<T: Display>(message: T) -> Self {
        Self {
            path: None,
            found: format!("a value whose `Serialize` failed ({message})"),
        }
    }
}

/// Serializes one value, the one at `path`.
struct Encoder {
    path: String,
}

/// `value`, the member at `key` of the value at `path`, as JSON.
fn member<T: Serialize + ?Sized>(path: &str, key: &str, value: &T) -> Result<Value, Refused> {
    let path = format!("{path}{}", token(key));
    value
        .serialize(Encoder { path: path.clone() })
        .map_err(|refused| refused.within(&path))
}

/// `{ variant: value }`, as `serde_json` tags an enum's variant.
fn tagged(variant: &str, value: Value) -> Value {
    let mut tagged = Map::new();
    tagged.insert(variant.to_owned(), value);
    Value::Object(tagged)
}

impl Encoder {
    fn float(&self, value: f64) -> Result<Value, Refused> {
        Number::from_f64(value).map(Value::Number).ok_or_else(|| {
            let found = if value.is_nan() {
                "NaN"
            } else if value > 0.0 {
                "infinity"
            } else {
                "-infinity"
            };
            Refused::at(&self.path, found.to_owned())
        })
    }

    fn wide(&self, value: impl Display, fits: Option<Value>) -> Result<Value, Refused> {
        fits.ok_or_else(|| {
            Refused::at(
                &self.path,
                format!("the integer {value}, wider than JSON's 64-bit numbers"),
            )
        })
    }
}

impl ser::Serializer for Encoder {
    type Ok = Value;
    type Error = Refused;
    type SerializeSeq = Items;
    type SerializeTuple = Items;
    type SerializeTupleStruct = Items;
    type SerializeTupleVariant = Items;
    type SerializeMap = Members;
    type SerializeStruct = Members;
    type SerializeStructVariant = Members;

    fn serialize_bool(self, value: bool) -> Result<Value, Refused> {
        Ok(Value::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_i16(self, value: i16) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_i32(self, value: i32) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_i64(self, value: i64) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_i128(self, value: i128) -> Result<Value, Refused> {
        let fits = i64::try_from(value)
            .map(Value::from)
            .or_else(|_| u64::try_from(value).map(Value::from))
            .ok();
        self.wide(value, fits)
    }

    fn serialize_u8(self, value: u8) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_u16(self, value: u16) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_u32(self, value: u32) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_u64(self, value: u64) -> Result<Value, Refused> {
        Ok(Value::from(value))
    }

    fn serialize_u128(self, value: u128) -> Result<Value, Refused> {
        self.wide(value, u64::try_from(value).map(Value::from).ok())
    }

    fn serialize_f32(self, value: f32) -> Result<Value, Refused> {
        if value.is_finite() {
            // `serde_json`'s own conversion, which keeps the shortest
            // decimal that reads back as the same `f32`.
            Ok(Value::from(value))
        } else {
            self.float(f64::from(value))
        }
    }

    fn serialize_f64(self, value: f64) -> Result<Value, Refused> {
        self.float(value)
    }

    fn serialize_char(self, value: char) -> Result<Value, Refused> {
        Ok(Value::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> Result<Value, Refused> {
        Ok(Value::String(value.to_owned()))
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<Value, Refused> {
        Ok(Value::Array(
            value.iter().copied().map(Value::from).collect(),
        ))
    }

    fn serialize_none(self) -> Result<Value, Refused> {
        Ok(Value::Null)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Value, Refused> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, Refused> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, Refused> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Value, Refused> {
        Ok(Value::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value, Refused> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value, Refused> {
        Ok(tagged(variant, member(&self.path, variant, value)?))
    }

    fn serialize_seq(self, length: Option<usize>) -> Result<Items, Refused> {
        Ok(Items::new(self.path, None, length))
    }

    fn serialize_tuple(self, length: usize) -> Result<Items, Refused> {
        Ok(Items::new(self.path, None, Some(length)))
    }

    fn serialize_tuple_struct(self, _name: &'static str, length: usize) -> Result<Items, Refused> {
        Ok(Items::new(self.path, None, Some(length)))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Items, Refused> {
        Ok(Items::new(self.path, Some(variant), Some(length)))
    }

    fn serialize_map(self, _length: Option<usize>) -> Result<Members, Refused> {
        Ok(Members::new(self.path, None))
    }

    fn serialize_struct(self, _name: &'static str, _length: usize) -> Result<Members, Refused> {
        Ok(Members::new(self.path, None))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _length: usize,
    ) -> Result<Members, Refused> {
        Ok(Members::new(self.path, Some(variant)))
    }
}

/// The items of an array, or of a tuple variant's, at `path`.
struct Items {
    /// The array's own path: the value's, or its variant's member.
    path: String,
    variant: Option<&'static str>,
    written: Vec<Value>,
}

impl Items {
    fn new(path: String, variant: Option<&'static str>, length: Option<usize>) -> Self {
        let path = match variant {
            Some(variant) => format!("{path}{}", token(variant)),
            None => path,
        };
        Self {
            path,
            variant,
            written: Vec::with_capacity(length.unwrap_or_default()),
        }
    }

    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Refused> {
        let item = member(&self.path, &self.written.len().to_string(), value)?;
        self.written.push(item);
        Ok(())
    }

    fn end(self) -> Value {
        let items = Value::Array(self.written);
        match self.variant {
            Some(variant) => tagged(variant, items),
            None => items,
        }
    }
}

impl ser::SerializeSeq for Items {
    type Ok = Value;
    type Error = Refused;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Refused> {
        self.push(value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Items::end(self))
    }
}

impl ser::SerializeTuple for Items {
    type Ok = Value;
    type Error = Refused;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Refused> {
        self.push(value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Items::end(self))
    }
}

impl ser::SerializeTupleStruct for Items {
    type Ok = Value;
    type Error = Refused;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Refused> {
        self.push(value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Items::end(self))
    }
}

impl ser::SerializeTupleVariant for Items {
    type Ok = Value;
    type Error = Refused;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Refused> {
        self.push(value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Items::end(self))
    }
}

/// The members of an object, or of a struct variant's, at `path`.
struct Members {
    /// The object's own path: the value's, or its variant's member.
    path: String,
    variant: Option<&'static str>,
    written: Map<String, Value>,
    /// The key of a map's member whose value comes next.
    key: Option<String>,
}

impl Members {
    fn new(path: String, variant: Option<&'static str>) -> Self {
        let path = match variant {
            Some(variant) => format!("{path}{}", token(variant)),
            None => path,
        };
        Self {
            path,
            variant,
            written: Map::new(),
            key: None,
        }
    }

    fn insert<T: Serialize + ?Sized>(&mut self, key: String, value: &T) -> Result<(), Refused> {
        let value = member(&self.path, &key, value)?;
        self.written.insert(key, value);
        Ok(())
    }

    fn end(self) -> Value {
        let members = Value::Object(self.written);
        match self.variant {
            Some(variant) => tagged(variant, members),
            None => members,
        }
    }
}

impl ser::SerializeMap for Members {
    type Ok = Value;
    type Error = Refused;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Refused> {
        let key = key
            .serialize(Key)
            .map_err(|refused| refused.within(&self.path))?;
        self.key = Some(key);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Refused> {
        let key = self
            .key
            .take()
            .expect("invariant: serde serializes a map's key before its value");
        self.insert(key, value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Members::end(self))
    }
}

impl ser::SerializeStruct for Members {
    type Ok = Value;
    type Error = Refused;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Refused> {
        self.insert(key.to_owned(), value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Members::end(self))
    }
}

impl ser::SerializeStructVariant for Members {
    type Ok = Value;
    type Error = Refused;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Refused> {
        self.insert(key.to_owned(), value)
    }

    fn end(self) -> Result<Value, Refused> {
        Ok(Members::end(self))
    }
}

/// Serializes a map's key: a string, or what writes one (a `char`, a unit
/// variant, a newtype around a string). Anything else is refused, where
/// `serde_json` would turn a number or a boolean into a string.
struct Key;

impl Key {
    fn refused(found: impl Display) -> Refused {
        Refused {
            path: None,
            found: format!("the key {found}, not a string"),
        }
    }
}

impl ser::Serializer for Key {
    type Ok = String;
    type Error = Refused;
    type SerializeSeq = Impossible<String, Refused>;
    type SerializeTuple = Impossible<String, Refused>;
    type SerializeTupleStruct = Impossible<String, Refused>;
    type SerializeTupleVariant = Impossible<String, Refused>;
    type SerializeMap = Impossible<String, Refused>;
    type SerializeStruct = Impossible<String, Refused>;
    type SerializeStructVariant = Impossible<String, Refused>;

    fn serialize_str(self, value: &str) -> Result<String, Refused> {
        Ok(value.to_owned())
    }

    fn serialize_char(self, value: char) -> Result<String, Refused> {
        Ok(value.to_string())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<String, Refused> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<String, Refused> {
        value.serialize(self)
    }

    fn serialize_bool(self, value: bool) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_i8(self, value: i8) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_i16(self, value: i16) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_i32(self, value: i32) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_i64(self, value: i64) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_i128(self, value: i128) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_u8(self, value: u8) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_u16(self, value: u16) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_u32(self, value: u32) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_u64(self, value: u64) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_u128(self, value: u128) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_f32(self, value: f32) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_f64(self, value: f64) -> Result<String, Refused> {
        Err(Self::refused(value))
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<String, Refused> {
        Err(Self::refused("of bytes"))
    }

    fn serialize_none(self) -> Result<String, Refused> {
        Err(Self::refused("none"))
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<String, Refused> {
        Err(Self::refused("of an option"))
    }

    fn serialize_unit(self) -> Result<String, Refused> {
        Err(Self::refused("()"))
    }

    fn serialize_unit_struct(self, name: &'static str) -> Result<String, Refused> {
        Err(Self::refused(name))
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _value: &T,
    ) -> Result<String, Refused> {
        Err(Self::refused(variant))
    }

    fn serialize_seq(self, _length: Option<usize>) -> Result<Self::SerializeSeq, Refused> {
        Err(Self::refused("of a sequence"))
    }

    fn serialize_tuple(self, _length: usize) -> Result<Self::SerializeTuple, Refused> {
        Err(Self::refused("of a tuple"))
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleStruct, Refused> {
        Err(Self::refused(name))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleVariant, Refused> {
        Err(Self::refused(variant))
    }

    fn serialize_map(self, _length: Option<usize>) -> Result<Self::SerializeMap, Refused> {
        Err(Self::refused("of a map"))
    }

    fn serialize_struct(
        self,
        name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStruct, Refused> {
        Err(Self::refused(name))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStructVariant, Refused> {
        Err(Self::refused(variant))
    }
}
