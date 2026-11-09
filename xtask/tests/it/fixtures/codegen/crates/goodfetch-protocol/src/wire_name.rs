//! The wire name of a unit enum's value (ADR 0038): the text its own serde
//! attributes give it, read from serde, so an enum's `as_str` and what it
//! sends are one rule and can never disagree. Each wire enum pairs it with
//! an `ALL` of its values, from which a metric label's values are derived.

use std::fmt;

use serde::Serialize;
use serde::ser::{self, Impossible};

/// The wire name serde gives `value`, a unit variant of an enum, such as
/// `"sdk_typescript"` for `Surface::SdkTypescript`.
///
/// # Panics
///
/// When `value` is no unit variant: a wire enum's `as_str` is its caller,
/// so that is a bug.
pub fn of<T: Serialize>(value: &T) -> &'static str {
    value
        .serialize(Name)
        .expect("invariant: a wire enum's value is a unit variant, which serde names")
}

/// The wire names of `values`, in their order: a metric label's values.
pub fn all<T: Serialize>(values: &[T]) -> Vec<&'static str> {
    values.iter().map(of).collect()
}

/// A serializer that answers a unit variant's name and refuses anything else.
struct Name;

/// What [`Name`] refuses: a value that is no unit variant.
#[derive(Debug)]
struct NotAUnitVariant;

impl fmt::Display for NotAUnitVariant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("not a unit variant")
    }
}

impl std::error::Error for NotAUnitVariant {}

impl ser::Error for NotAUnitVariant {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self
    }
}

type Refused = Impossible<&'static str, NotAUnitVariant>;

impl ser::Serializer for Name {
    type Ok = &'static str;
    type Error = NotAUnitVariant;
    type SerializeSeq = Refused;
    type SerializeTuple = Refused;
    type SerializeTupleStruct = Refused;
    type SerializeTupleVariant = Refused;
    type SerializeMap = Refused;
    type SerializeStruct = Refused;
    type SerializeStructVariant = Refused;

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(variant)
    }

    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_char(self, _: char) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_str(self, _: &str) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(NotAUnitVariant)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(NotAUnitVariant)
    }
}

#[cfg(test)]
pub(crate) mod tests;
