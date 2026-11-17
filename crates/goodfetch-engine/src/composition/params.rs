//! A call's params, read straight from where they are in the line they came
//! on into the type the handler takes (ADR 0140's E1), as the schema bundle
//! gives them (ADR 0038): every struct and map, the params' own and each one
//! inside them, reads only from a JSON object. Serde's derived structs also
//! read from an array, by position, which the schema's `type: object`
//! refuses; read through [`Params::read`], that is `invalid_params`. The
//! params' `_meta`, the Trace context the RPC layer takes (ADR 0132), is no
//! member of any params type, and the read skips it. The same pass notes
//! what dispatch takes of them and the page a page call names, for the
//! call's guard. Each read is a decode of the request's params and counts
//! as one (`E1.decodes`).

use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::jsonrpc::{META, ProtocolFailure};
use serde::de::value::BorrowedStrDeserializer;
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};

use serde_json::value::RawValue;

use crate::call_error::CallError;
use crate::trace::ID_FIELDS;
use crate::wire::{Key, KeySeed, text};

mod preamble;

pub(crate) use preamble::Preamble;
use preamble::{Found, Noted};

/// A request's params as they arrived, held by dispatch while the call
/// starts: their JSON text, where it is in the line that carried them, `{}`
/// for a request that sent none, the Counted work each decode of them
/// counts on, and what the read into their handler's type found of what
/// dispatch takes of them and of the page they name.
pub(crate) struct Arrived<'a> {
    json: &'a str,
    work: &'a CountedWork,
    /// What a whole read of them found, until dispatch checks it.
    found: Cell<Option<Found<'a>>>,
    /// What dispatch takes of them, once checked.
    preamble: Cell<Option<Preamble<'a>>>,
    /// The page they name, as it is written in them, once checked.
    page: Cell<Option<&'a RawValue>>,
}

impl<'a> Arrived<'a> {
    /// The params whose JSON text is `json`, or those of a request that
    /// sent none, their decodes counted on `work`.
    pub(crate) fn new(json: Option<&'a str>, work: &'a CountedWork) -> Self {
        Self {
            json: json.unwrap_or("{}"),
            work,
            // No params hold nothing for dispatch, whoever reads them.
            found: Cell::new(json.is_none().then(Found::default)),
            preamble: Cell::new(None),
            page: Cell::new(None),
        }
    }

    /// They, as a guard and a handler read them.
    pub(crate) fn params(&'a self) -> Params<'a> {
        Params(self)
    }

    /// What dispatch takes of them: what the read into their handler's
    /// type found in its pass, or, when no read went through them, as when
    /// their handler refused them, a read of its own, which counts one
    /// decode.
    ///
    /// # Errors
    ///
    /// `invalid_params` when their `_meta` holds a key other than
    /// `traceparent` and `tracestate` or a value that is no string.
    pub(crate) fn preamble(&self) -> Result<Preamble<'a>, CallError> {
        self.check()?;
        Ok(self.preamble.take().unwrap_or_default())
    }

    /// Checks what dispatch takes of them, once, and keeps the page they
    /// name: their own `page_id`, else their locator's.
    fn check(&self) -> Result<(), CallError> {
        let checked = self.preamble.take();
        if checked.is_none() {
            let found = self.found.take().unwrap_or_else(|| {
                self.work.count(Row::MessageDecodes);
                let noted = Noted::scan(self.json);
                self.page.set(noted.page());
                noted.found
            });
            self.preamble.set(Some(found.checked()?));
        } else {
            self.preamble.set(checked);
        }
        Ok(())
    }
}

/// A call's params as they arrived, as its guard and its handler read
/// them.
#[derive(Clone, Copy)]
pub struct Params<'a>(&'a Arrived<'a>);

impl fmt::Debug for Params<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Params").field(&self.0.json).finish()
    }
}

impl<'a> Params<'a> {
    /// Their JSON text.
    pub fn json(&self) -> &'a str {
        self.0.json
    }

    /// They, read as `P` in one pass, each struct and map only from an
    /// object, without their `_meta`. Counts one decode. A read that goes
    /// through every member of the params also finds, in the same pass,
    /// what dispatch takes of them and the page they name. A function of
    /// its own, never inlined, so that `xtask counts`' Callgrind run can
    /// name it in `E1.handled_page_wire_share` (ADR 0140).
    ///
    /// # Errors
    ///
    /// `invalid_params` when they are no `P`.
    #[inline(never)]
    pub fn read<P: DeserializeOwned>(&self) -> Result<P, CallError> {
        self.0.work.count(Row::MessageDecodes);
        let mut noted = Noted::default();
        let mut json = serde_json::Deserializer::from_str(self.0.json);
        let params = P::deserialize(Strict {
            inner: &mut json,
            outer: Outer::Params(&mut noted),
        })
        .and_then(|params| json.end().map(|()| params))
        .map_err(|error| {
            CallError::protocol(ProtocolFailure::InvalidParams, without_position(&error))
        })?;
        // Only a pass through the params' own members found what dispatch
        // takes; after any other, as into a type that ignores them,
        // dispatch reads them itself.
        if noted.found.whole {
            self.0.page.set(noted.page());
            self.0.found.set(Some(noted.found));
        }
        Ok(params)
    }

    /// Whether dispatch takes them: `invalid_params` for a `_meta` it
    /// refuses, before the call's guard is asked and its handler called.
    pub(crate) fn taken(self) -> Result<(), CallError> {
        self.0.check()
    }

    /// The page they name, as a page call names it (`page_id`) or a
    /// locator does (`locator.page_id`), the call's own first; `None` when
    /// neither is a string. The read into the handler's type found it in
    /// its pass, which a guard comes after; params no read went through
    /// whole have it found by dispatch's own read, as for what dispatch
    /// takes of them, and `None` when dispatch refuses them.
    pub fn page(&self) -> Option<Cow<'a, str>> {
        self.0.check().ok()?;
        text(self.0.page.get()?)
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

/// The key of the member through which a page call names its page, and
/// a locator its own.
const PAGE_ID: &str = "page_id";

/// The key of a call's locator.
const LOCATOR: &str = "locator";

/// Where a read notes what is taken of the params before their handler.
enum Outer<'t, 'de> {
    /// A value inside them, which holds nothing taken.
    Inside,
    /// The params themselves: their `_meta`, their IDs and the page they
    /// name.
    Params(&'t mut Noted<'de>),
    /// Their locator: the page it names.
    Locator(&'t mut Noted<'de>),
}

/// A deserializer whose structs and maps, at every depth, read only from
/// objects; `outer` for the params themselves, whose `_meta`, IDs and page
/// it notes there, their `_meta` skipped, and for their locator.
struct Strict<'t, 'de, D> {
    inner: D,
    outer: Outer<'t, 'de>,
}

/// A visitor that reads every nested value strictly.
struct Wrap<'t, 'de, V> {
    visitor: V,
    outer: Outer<'t, 'de>,
}

/// A visitor that reads only an object, its members strictly.
struct Object<'t, 'de, V> {
    visitor: V,
    outer: Outer<'t, 'de>,
}

macro_rules! forward_strictly {
    ($($method:ident)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
                self.inner.$method(Wrap { visitor, outer: Outer::Inside })
            }
        )*
    };
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Strict<'_, 'de, D> {
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
                outer: Outer::Inside,
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
                outer: Outer::Inside,
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
                outer: Outer::Inside,
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
                outer: Outer::Inside,
            },
        )
    }

    forward_strictly! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_i128 deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_u128 deserialize_f32 deserialize_f64 deserialize_char deserialize_str
        deserialize_string deserialize_bytes deserialize_byte_buf deserialize_unit
        deserialize_seq deserialize_identifier
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        match self.outer {
            // A locator the handler's type skips still names the page.
            Outer::Locator(_) => self.inner.deserialize_any(Wrap {
                visitor,
                outer: self.outer,
            }),
            Outer::Inside | Outer::Params(_) => self.inner.deserialize_ignored_any(Wrap {
                visitor,
                outer: Outer::Inside,
            }),
        }
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

impl<'de, V: Visitor<'de>> Visitor<'de> for Wrap<'_, 'de, V> {
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
            note: None,
        })
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<V::Value, A::Error> {
        self.visitor.visit_enum(Variant(data))
    }
}

/// Every value but an object is refused, as serde words what it did not
/// expect.
impl<'de, V: Visitor<'de>> Visitor<'de> for Object<'_, 'de, V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.visitor.expecting(formatter)
    }

    fn visit_map<A: MapAccess<'de>>(self, members: A) -> Result<V::Value, A::Error> {
        self.visitor.visit_map(Members {
            inner: members,
            outer: self.outer,
            note: None,
        })
    }
}

/// A seed that reads its value strictly, noting in it what `outer` says.
struct Seed<'t, 'de, S> {
    seed: S,
    outer: Outer<'t, 'de>,
}

impl<S> Seed<'_, '_, S> {
    /// `seed`, read strictly, for a value that holds nothing taken.
    fn inside(seed: S) -> Self {
        Self {
            seed,
            outer: Outer::Inside,
        }
    }
}

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<'_, 'de, S> {
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, inner: D) -> Result<S::Value, D::Error> {
        self.seed.deserialize(Strict {
            inner,
            outer: self.outer,
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
        self.0.next_element_seed(Seed::inside(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

/// An object's members, each value read strictly; the params' own skip
/// their `_meta`, noting it, their IDs and the page they name where they
/// are taken, and their locator's note the page it names.
struct Members<'t, 'de, A> {
    inner: A,
    outer: Outer<'t, 'de>,
    /// What the member whose key was just read is noted as, if anything.
    note: Option<Note>,
}

/// A member noted where it is taken.
#[derive(Clone, Copy)]
enum Note {
    /// One of the IDs, by its place in [`ID_FIELDS`].
    Id(usize),
    /// The page the params name themselves.
    Page,
    /// Their locator.
    Locator,
    /// The page their locator names.
    Located,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Members<'_, 'de, A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        let (noted, params) = match &mut self.outer {
            Outer::Inside => return self.inner.next_key_seed(seed),
            Outer::Params(noted) => (noted, true),
            Outer::Locator(noted) => (noted, false),
        };
        loop {
            let Some(Key(key)) = self.inner.next_key_seed(KeySeed)? else {
                noted.found.whole |= params;
                return Ok(None);
            };
            if params && key == META {
                noted.found.meta = Some(self.inner.next_value()?);
                continue;
            }
            self.note = match (params, key.as_ref()) {
                (true, PAGE_ID) => Some(Note::Page),
                (true, LOCATOR) => Some(Note::Locator),
                (true, key) => ID_FIELDS
                    .iter()
                    .position(|field| *field == key)
                    .map(Note::Id),
                (false, PAGE_ID) => Some(Note::Located),
                (false, _) => None,
            };
            return match key {
                Cow::Borrowed(key) => seed.deserialize(BorrowedStrDeserializer::new(key)),
                Cow::Owned(key) => seed.deserialize(key.into_deserializer()),
            }
            .map(Some);
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        let (Some(note), Outer::Params(noted) | Outer::Locator(noted)) =
            (self.note.take(), &mut self.outer)
        else {
            return self.inner.next_value_seed(Seed::inside(seed));
        };
        if let Note::Locator = note {
            return self.inner.next_value_seed(Seed {
                seed,
                outer: Outer::Locator(noted),
            });
        }
        // A noted value is taken where it is and read on into the
        // handler's type from there, as serde_json reads any value: the
        // pass goes through the params once, and through its own text once
        // more.
        let value: &'de RawValue = self.inner.next_value()?;
        match note {
            Note::Id(at) => noted.found.ids[at] = text(value),
            Note::Page => noted.page = Some(value),
            Note::Located => noted.located = Some(value),
            Note::Locator => {}
        }
        let mut json = serde_json::Deserializer::from_str(value.get());
        seed.deserialize(Strict {
            inner: &mut json,
            outer: Outer::Inside,
        })
        .map_err(|error| de::Error::custom(without_position(&error)))
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
        self.0.newtype_variant_seed(Seed::inside(seed))
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
                outer: Outer::Inside,
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
                outer: Outer::Inside,
            },
        )
    }
}

#[cfg(test)]
mod tests;
