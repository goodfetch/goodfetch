//! A call's params, read straight from where they are in the line they came
//! on into the type the handler takes (ADR 0140's E1), as the schema bundle
//! gives them (ADR 0038): every struct and map, the params' own and each one
//! inside them, reads only from a JSON object. Serde's derived structs also
//! read from an array, by position, which the schema's `type: object`
//! refuses; read through [`Params::read`], that is `invalid_params`. The
//! params' `_meta`, the Trace context the RPC layer takes (ADR 0132), is no
//! member of any params type, and the read skips it.

use std::borrow::Cow;
use std::fmt;

use goodfetch_protocol::jsonrpc::{META, ProtocolFailure};
use serde::de::value::BorrowedStrDeserializer;
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IgnoredAny,
    IntoDeserializer, MapAccess, SeqAccess, VariantAccess, Visitor,
};

use crate::call_error::CallError;
use crate::wire::{Key, KeySeed};

/// A call's params as they arrived: their JSON text, where it is in the
/// line that carried them, `{}` for a call that sent none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params<'a> {
    json: &'a str,
}

impl<'a> Params<'a> {
    /// The params whose JSON text is `json`.
    pub fn new(json: &'a str) -> Self {
        Self { json }
    }

    /// The params of a call that sent none: `{}`.
    pub fn none() -> Self {
        Self { json: "{}" }
    }

    /// Their JSON text.
    pub fn json(&self) -> &'a str {
        self.json
    }

    /// They, read as `P` in one pass, each struct and map only from an
    /// object, without their `_meta`.
    ///
    /// # Errors
    ///
    /// `invalid_params` when they are no `P`.
    pub fn read<P: DeserializeOwned>(&self) -> Result<P, CallError> {
        let mut json = serde_json::Deserializer::from_str(self.json);
        P::deserialize(Strict {
            inner: &mut json,
            outer: true,
        })
        .and_then(|params| json.end().map(|()| params))
        .map_err(|error| {
            CallError::protocol(ProtocolFailure::InvalidParams, without_position(&error))
        })
    }
}

/// What `error` says, without where in the params it is: the params are
/// read as one value, as the wire's error has always told it.
pub(crate) fn without_position(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let position = format!(" at line {} column {}", error.line(), error.column());
    match text.strip_suffix(&position) {
        Some(text) if error.line() > 0 => text.to_owned(),
        _ => text,
    }
}

/// A deserializer whose structs and maps, at every depth, read only from
/// objects; `outer` for the params themselves, whose `_meta` it skips.
struct Strict<D> {
    inner: D,
    outer: bool,
}

/// A visitor that reads every nested value strictly.
struct Wrap<V> {
    visitor: V,
    outer: bool,
}

/// A visitor that reads only an object, its members strictly.
struct Object<V> {
    visitor: V,
    outer: bool,
}

macro_rules! forward_strictly {
    ($($method:ident)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
                self.inner.$method(Wrap { visitor, outer: false })
            }
        )*
    };
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Strict<D> {
    type Error = D::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.inner.deserialize_any(Wrap {
            visitor,
            outer: self.outer,
        })
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.inner.deserialize_option(Wrap {
            visitor,
            outer: self.outer,
        })
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.inner.deserialize_newtype_struct(
            name,
            Wrap {
                visitor,
                outer: self.outer,
            },
        )
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.inner.deserialize_any(Object {
            visitor,
            outer: self.outer,
        })
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.inner.deserialize_unit_struct(
            name,
            Wrap {
                visitor,
                outer: false,
            },
        )
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.inner.deserialize_tuple(
            length,
            Wrap {
                visitor,
                outer: false,
            },
        )
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.inner.deserialize_tuple_struct(
            name,
            length,
            Wrap {
                visitor,
                outer: false,
            },
        )
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.inner.deserialize_enum(
            name,
            variants,
            Wrap {
                visitor,
                outer: false,
            },
        )
    }

    forward_strictly! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_i128 deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_u128 deserialize_f32 deserialize_f64 deserialize_char deserialize_str
        deserialize_string deserialize_bytes deserialize_byte_buf deserialize_unit
        deserialize_seq deserialize_identifier deserialize_ignored_any
    }

    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}

macro_rules! forward_visits {
    ($($method:ident: $type:ty)*) => {
        $(
            fn $method<E: de::Error>(self, value: $type) -> Result<V::Value, E> {
                self.visitor.$method(value)
            }
        )*
    };
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Wrap<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.visitor.expecting(formatter)
    }

    forward_visits! {
        visit_bool: bool visit_i8: i8 visit_i16: i16 visit_i32: i32 visit_i64: i64
        visit_i128: i128 visit_u8: u8 visit_u16: u16 visit_u32: u32 visit_u64: u64
        visit_u128: u128 visit_f32: f32 visit_f64: f64 visit_char: char
        visit_str: &str visit_borrowed_str: &'de str visit_string: String
        visit_bytes: &[u8] visit_borrowed_bytes: &'de [u8] visit_byte_buf: Vec<u8>
    }

    fn visit_none<E: de::Error>(self) -> Result<V::Value, E> {
        self.visitor.visit_none()
    }

    fn visit_unit<E: de::Error>(self) -> Result<V::Value, E> {
        self.visitor.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, inner: D) -> Result<V::Value, D::Error> {
        self.visitor.visit_some(Strict {
            inner,
            outer: self.outer,
        })
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, inner: D) -> Result<V::Value, D::Error> {
        self.visitor.visit_newtype_struct(Strict {
            inner,
            outer: self.outer,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, items: A) -> Result<V::Value, A::Error> {
        self.visitor.visit_seq(Items(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, members: A) -> Result<V::Value, A::Error> {
        self.visitor.visit_map(Members {
            inner: members,
            outer: self.outer,
        })
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<V::Value, A::Error> {
        self.visitor.visit_enum(Variant(data))
    }
}

/// Every value but an object is refused, as serde words what it did not
/// expect.
impl<'de, V: Visitor<'de>> Visitor<'de> for Object<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.visitor.expecting(formatter)
    }

    fn visit_map<A: MapAccess<'de>>(self, members: A) -> Result<V::Value, A::Error> {
        self.visitor.visit_map(Members {
            inner: members,
            outer: self.outer,
        })
    }
}

/// A seed that reads its value strictly.
struct Seed<S>(S);

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, inner: D) -> Result<S::Value, D::Error> {
        self.0.deserialize(Strict {
            inner,
            outer: false,
        })
    }
}

/// An array's items, each read strictly.
struct Items<A>(A);

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for Items<A> {
    type Error = A::Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, A::Error> {
        self.0.next_element_seed(Seed(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

/// An object's members, each value read strictly; the params' own skip
/// their `_meta`.
struct Members<A> {
    inner: A,
    outer: bool,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Members<A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        if !self.outer {
            return self.inner.next_key_seed(seed);
        }
        loop {
            let Some(Key(key)) = self.inner.next_key_seed(KeySeed)? else {
                return Ok(None);
            };
            if key == META {
                self.inner.next_value::<IgnoredAny>()?;
                continue;
            }
            return match key {
                Cow::Borrowed(key) => seed.deserialize(BorrowedStrDeserializer::new(key)),
                Cow::Owned(key) => seed.deserialize(key.into_deserializer()),
            }
            .map(Some);
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.inner.next_value_seed(Seed(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.inner.size_hint()
    }
}

/// An enum's variant, its content read strictly.
struct Variant<A>(A);

impl<'de, A: EnumAccess<'de>> EnumAccess<'de> for Variant<A> {
    type Error = A::Error;
    type Variant = Content<A::Variant>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), A::Error> {
        self.0
            .variant_seed(seed)
            .map(|(variant, content)| (variant, Content(content)))
    }
}

/// A variant's content, read strictly.
struct Content<A>(A);

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for Content<A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, A::Error> {
        self.0.newtype_variant_seed(Seed(seed))
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.tuple_variant(
            length,
            Wrap {
                visitor,
                outer: false,
            },
        )
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.struct_variant(
            fields,
            Object {
                visitor,
                outer: false,
            },
        )
    }
}

#[cfg(test)]
mod tests;
