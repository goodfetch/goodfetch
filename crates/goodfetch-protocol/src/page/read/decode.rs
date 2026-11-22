//! How a field declaration is read: from an object, in one pass, with no
//! copy of it first (issue 3324). A derived untagged or internally tagged
//! enum copies each object into serde's buffer before it reads it, which
//! was most of what reading `page.read`'s parameters cost. Every object the
//! schema allows reads as serde's derived enums read it, to the same value,
//! and each unknown, repeated, missing or ill-typed member fails in the
//! same words. What only serde's buffer let through, and the schema
//! refuses, is refused at every depth: a field or a locator given as an
//! array, a `kind` given by its index, and, through the engine's strict
//! read of params, a struct inside a field given as an array.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

use serde::de::value::{BorrowedStrDeserializer, MapAccessDeserializer, StringDeserializer};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Unexpected, Visitor};
use serde::{Deserialize, Deserializer};

use super::{FieldLocator, ListField, PageField, RowsField, ValueField};

/// The words of serde's untagged enum for a value that is none of its
/// variants.
const NO_FIELD: &str = "data did not match any variant of untagged enum PageField";

impl<'de> Deserialize<'de> for PageField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(FieldVisitor)
    }
}

/// Reads a field from an object, by its first key; any other value is none
/// of the three.
struct FieldVisitor;

impl<'de> Visitor<'de> for FieldVisitor {
    type Value = PageField;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a value field, a list or rows")
    }

    /// Only the variant its first key belongs to can read the object, as
    /// each denies the others' keys: `list` a list's, `rows` and `fields`
    /// rows', any other a value field's. Any error is the untagged enum's,
    /// one in the text itself too, such as an escape or a number the JSON
    /// reader cannot read, which the enum's copy would have met first.
    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<PageField, A::Error> {
        let Some(first) = members.next_key_seed(Key)? else {
            return Err(de::Error::custom(NO_FIELD));
        };
        let key = first.as_ref();
        let list = key == "list";
        let rows = key == "rows" || key == "fields";
        let object = MapAccessDeserializer::new(Replayed {
            first: Some(first),
            members,
        });
        let field = if list {
            ListField::deserialize(object).map(PageField::List)
        } else if rows {
            RowsField::deserialize(object).map(PageField::Rows)
        } else {
            ValueField::deserialize(object).map(PageField::Value)
        };
        field.map_err(|_: A::Error| de::Error::custom(NO_FIELD))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<PageField, A::Error> {
        Err(de::Error::custom(NO_FIELD))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_i128<E: de::Error>(self, _: i128) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_u128<E: de::Error>(self, _: u128) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_unit<E: de::Error>(self) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }

    fn visit_none<E: de::Error>(self) -> Result<PageField, E> {
        Err(E::custom(NO_FIELD))
    }
}

impl<'de> Deserialize<'de> for FieldLocator {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(LocatorVisitor)
    }
}

/// Reads a locator from an object as serde's internally tagged enum does,
/// in one pass. That enum finds the `kind` while it copies the other
/// members, failing at once on a second `kind` or on one that names no
/// variant, then reads the variant from the copy: its first wrong member
/// fails it, else a missing `selector`. Both variants hold the same one
/// member, so the members are read as they come, the first wrong one kept
/// until the `kind` is known. Any other value, an array too, is refused.
struct LocatorVisitor;

impl<'de> Visitor<'de> for LocatorVisitor {
    type Value = FieldLocator;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("internally tagged enum FieldLocator")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<FieldLocator, A::Error> {
        let mut kind = None;
        let mut selector = None;
        let mut wrong = None;
        while let Some(key) = members.next_key_seed(Key)? {
            if key == "kind" {
                if kind.is_some() {
                    return Err(de::Error::duplicate_field("kind"));
                }
                kind = Some(members.next_value_seed(KindSeed)?);
            } else if wrong.is_some() {
                members.next_value::<IgnoredAny>()?;
            } else if key == "selector" {
                let text = members.next_value_seed(Text(PhantomData))?;
                if selector.is_some() {
                    wrong = Some(de::Error::duplicate_field("selector"));
                } else {
                    match text {
                        Ok(text) => selector = Some(text),
                        Err(error) => wrong = Some(error),
                    }
                }
            } else {
                members.next_value::<IgnoredAny>()?;
                wrong = Some(de::Error::unknown_field(&key, &["selector"]));
            }
        }
        let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
        if let Some(wrong) = wrong {
            return Err(wrong);
        }
        let selector = selector.ok_or_else(|| de::Error::missing_field("selector"))?;
        Ok(match kind {
            Kind::Css => FieldLocator::Css { selector },
            Kind::Xpath => FieldLocator::Xpath { selector },
        })
    }
}

/// A locator's variant.
enum Kind {
    Css,
    Xpath,
}

/// The variants' names, in order.
const KINDS: &[&str] = &["css", "xpath"];

/// Reads a `kind` by its name, in the words of serde's variant identifier.
struct KindSeed;

impl<'de> DeserializeSeed<'de> for KindSeed {
    type Value = Kind;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Kind, D::Error> {
        deserializer.deserialize_identifier(self)
    }
}

impl Visitor<'_> for KindSeed {
    type Value = Kind;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("variant identifier")
    }

    fn visit_str<E: de::Error>(self, name: &str) -> Result<Kind, E> {
        match name {
            "css" => Ok(Kind::Css),
            "xpath" => Ok(Kind::Xpath),
            _ => Err(E::unknown_variant(name, KINDS)),
        }
    }
}

/// Reads a `selector`: its text, or the error a string's reader gives any
/// other value, which the locator keeps while it reads on. Any other value
/// is read whole, so the members after it can still be read.
struct Text<E>(PhantomData<E>);

impl<'de, E: de::Error> DeserializeSeed<'de> for Text<E> {
    type Value = Result<String, E>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<E: de::Error> Text<E> {
    /// The error a string's reader gives a value of another type.
    fn no_string(unexpected: Unexpected<'_>) -> Result<String, E> {
        Err(E::invalid_type(unexpected, &"a string"))
    }
}

impl<'de, E: de::Error> Visitor<'de> for Text<E> {
    type Value = Result<String, E>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string")
    }

    fn visit_str<F: de::Error>(self, text: &str) -> Result<Self::Value, F> {
        Ok(Ok(text.to_owned()))
    }

    fn visit_string<F: de::Error>(self, text: String) -> Result<Self::Value, F> {
        Ok(Ok(text))
    }

    fn visit_bool<F: de::Error>(self, value: bool) -> Result<Self::Value, F> {
        Ok(Self::no_string(Unexpected::Bool(value)))
    }

    fn visit_i64<F: de::Error>(self, value: i64) -> Result<Self::Value, F> {
        Ok(Self::no_string(Unexpected::Signed(value)))
    }

    fn visit_u64<F: de::Error>(self, value: u64) -> Result<Self::Value, F> {
        Ok(Self::no_string(Unexpected::Unsigned(value)))
    }

    fn visit_f64<F: de::Error>(self, value: f64) -> Result<Self::Value, F> {
        Ok(Self::no_string(Unexpected::Float(value)))
    }

    fn visit_unit<F: de::Error>(self) -> Result<Self::Value, F> {
        Ok(Self::no_string(Unexpected::Unit))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<Self::Value, A::Error> {
        while members.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Self::no_string(Unexpected::Map))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Self::Value, A::Error> {
        while items.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Self::no_string(Unexpected::Seq))
    }
}

/// Reads an object's key, where it is when it holds no escape.
struct Key;

impl<'de> DeserializeSeed<'de> for Key {
    type Value = Cow<'de, str>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for Key {
    type Value = Cow<'de, str>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a key")
    }

    fn visit_borrowed_str<E: de::Error>(self, key: &'de str) -> Result<Self::Value, E> {
        Ok(Cow::Borrowed(key))
    }

    fn visit_str<E: de::Error>(self, key: &str) -> Result<Self::Value, E> {
        Ok(Cow::Owned(key.to_owned()))
    }

    fn visit_string<E: de::Error>(self, key: String) -> Result<Self::Value, E> {
        Ok(Cow::Owned(key))
    }
}

/// An object's members, its first key, read already, given again first.
struct Replayed<'de, A> {
    first: Option<Cow<'de, str>>,
    members: A,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Replayed<'de, A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        match self.first.take() {
            Some(Cow::Borrowed(key)) => seed
                .deserialize(BorrowedStrDeserializer::new(key))
                .map(Some),
            Some(Cow::Owned(key)) => seed.deserialize(StringDeserializer::new(key)).map(Some),
            None => self.members.next_key_seed(seed),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.members.next_value_seed(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        let first = usize::from(self.first.is_some());
        self.members.size_hint().map(|rest| rest + first)
    }
}
