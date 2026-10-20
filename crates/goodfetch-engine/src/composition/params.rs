//! A call's params read as the schema bundle gives them (ADR 0038): every
//! struct and map, the params' own and each one inside them, reads only
//! from a JSON object. Serde's derived structs also read from an array, by
//! position, and `serde_json`'s map from `null`, both of which the schema's
//! `type: object` refuses; read through [`read`], they are
//! `invalid_params`.

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, Unexpected, VariantAccess, Visitor,
};
use serde_json::{Error, Map, Value};

/// `params` read as `P`, with each struct and map read only from an object.
pub(super) fn read<P: DeserializeOwned>(params: Value) -> Result<P, Error> {
    P::deserialize(Strict(params))
}

/// A JSON value whose structs and maps, at every depth, read only from
/// objects.
struct Strict(Value);

impl<'de> Deserializer<'de> for Strict {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Array(items) => visitor.visit_seq(Items(items.into_iter())),
            Value::Object(members) => visitor.visit_map(Members {
                members: members.into_iter(),
                value: None,
            }),
            scalar => scalar.deserialize_any(visitor),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            value => visitor.visit_some(Self(value)),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Object(_) => self.deserialize_any(visitor),
            other => Err(de::Error::invalid_type(unexpected(&other), &visitor)),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.0 {
            Value::String(variant) => visitor.visit_enum(Variant {
                variant,
                value: None,
            }),
            Value::Object(members) if members.len() == 1 => {
                let (variant, value) = members
                    .into_iter()
                    .next()
                    .expect("invariant: the object has one member");
                visitor.visit_enum(Variant {
                    variant,
                    value: Some(value),
                })
            }
            other => other.deserialize_enum(name, variants, visitor),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct identifier
        ignored_any
    }
}

/// What `value` is, as serde names what it did not expect.
fn unexpected(value: &Value) -> Unexpected<'_> {
    match value {
        Value::Null => Unexpected::Unit,
        Value::Bool(value) => Unexpected::Bool(*value),
        Value::Number(number) => number.as_u64().map_or_else(
            || {
                number
                    .as_i64()
                    .map_or(Unexpected::Other("number"), Unexpected::Signed)
            },
            Unexpected::Unsigned,
        ),
        Value::String(text) => Unexpected::Str(text),
        Value::Array(_) => Unexpected::Seq,
        Value::Object(_) => Unexpected::Map,
    }
}

/// An array's items, each read strictly.
struct Items(std::vec::IntoIter<Value>);

impl<'de> SeqAccess<'de> for Items {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        self.0
            .next()
            .map(|item| seed.deserialize(Strict(item)))
            .transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

/// An object's members, each value read strictly.
struct Members {
    members: <Map<String, Value> as IntoIterator>::IntoIter,
    value: Option<Value>,
}

impl<'de> MapAccess<'de> for Members {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        let Some((key, value)) = self.members.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(key.into_deserializer()).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| de::Error::custom("a value read before its key"))?;
        seed.deserialize(Strict(value))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.members.len())
    }
}

/// An enum's variant, by its name, and its content, if it has one.
struct Variant {
    variant: String,
    value: Option<Value>,
}

impl<'de> EnumAccess<'de> for Variant {
    type Error = Error;
    type Variant = Content;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Content), Error> {
        let variant = seed.deserialize(self.variant.into_deserializer())?;
        Ok((variant, Content(self.value)))
    }
}

/// A variant's content.
struct Content(Option<Value>);

impl<'de> VariantAccess<'de> for Content {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        match self.0 {
            None | Some(Value::Null) => Ok(()),
            Some(_) => Err(de::Error::invalid_type(
                Unexpected::Other("content"),
                &"a unit variant",
            )),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(Strict(self.0.unwrap_or(Value::Null)))
    }

    fn tuple_variant<V: Visitor<'de>>(self, _length: usize, visitor: V) -> Result<V::Value, Error> {
        Strict(self.0.unwrap_or(Value::Null)).deserialize_seq(visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        Strict(self.0.unwrap_or(Value::Null)).deserialize_struct("", fields, visitor)
    }
}
