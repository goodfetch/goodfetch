//! The in-memory form of an open value type (ADR 0099): its wire string
//! held inline, at most 63 bytes, so a value read from the wire equals its
//! constant with `==` and in a `match`, whichever crate defines the
//! constant, and a value the program does not know stays a value. The
//! [`open_value_type`] macro gives each such type, the Error code, the
//! Plugin kind and the Hook stage, the same constructors, accessor and
//! serde form.

/// The longest value an open value type holds, in bytes, which codegen
/// checks on every value of the catalog (ADR 0099).
pub(crate) const MAX_LEN: usize = 63;

/// A string of at most [`MAX_LEN`] bytes, held inline.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Inline {
    /// How many of `bytes` the value takes; the rest are zero, so two equal
    /// values are equal byte for byte.
    len: u8,
    bytes: [u8; MAX_LEN],
}

impl Inline {
    /// `text`, or none when it is longer than [`MAX_LEN`] bytes.
    #[must_use]
    pub(crate) const fn of(text: &str) -> Option<Self> {
        let source = text.as_bytes();
        if source.len() > MAX_LEN {
            return None;
        }
        let mut bytes = [0; MAX_LEN];
        let mut at = 0;
        while at < source.len() {
            bytes[at] = source[at];
            at += 1;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the length is at most MAX_LEN, 63, checked above"
        )]
        let len = source.len() as u8;
        Some(Self { len, bytes })
    }

    /// The string it holds.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        let bytes = &self.bytes[..usize::from(self.len)];
        std::str::from_utf8(bytes).expect("invariant: an Inline holds the bytes of a str")
    }
}

/// Defines an open value type `$name`, a newtype over [`Inline`], with
/// `from_static` for its constants, `new`, `as_str`, an order by its string,
/// `Display`, a `Debug` that names it, and its wire string as its serde form. `$what` names its
/// values in the panic and the error a value too long gets.
macro_rules! open_value_type {
    ($(#[$attribute:meta])* $name:ident, $what:literal) => {
        $(#[$attribute])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name($crate::inline_value::Inline);

        impl $name {
            /// The longest value it holds, in bytes, which codegen checks
            /// on every value of the catalog (ADR 0099).
            pub const MAX_LEN: usize = $crate::inline_value::MAX_LEN;

            /// The value `value`, for a constant.
            ///
            /// # Panics
            ///
            /// When `value` is longer than `MAX_LEN` bytes, which in a
            /// constant fails the build.
            #[must_use]
            pub const fn from_static(value: &'static str) -> Self {
                match $crate::inline_value::Inline::of(value) {
                    Some(inline) => Self(inline),
                    None => panic!(concat!($what, " is at most 63 bytes")),
                }
            }

            /// The value `value`, or none when it is longer than
            /// `MAX_LEN` bytes, which no catalog value is.
            #[must_use]
            pub fn new(value: &str) -> Option<Self> {
                $crate::inline_value::Inline::of(value).map(Self)
            }

            /// Its `snake_case` string.
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::cmp::PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        /// Ordered by its string, as a sorted listing shows it.
        impl ::std::cmp::Ord for $name {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                self.as_str().cmp(other.as_str())
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.as_str())
                    .finish()
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let value = ::std::borrow::Cow::<str>::deserialize(deserializer)?;
                Self::new(&value).ok_or_else(|| {
                    ::serde::de::Error::custom(format_args!(
                        concat!($what, " is at most {} bytes"),
                        Self::MAX_LEN
                    ))
                })
            }
        }
    };
}

pub(crate) use open_value_type;
