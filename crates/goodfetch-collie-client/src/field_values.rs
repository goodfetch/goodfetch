//! A page's fields read typed from their declaration (ADR 0147), as far
//! as Rust allows: `page.read(fields)` answers each declared field under
//! its name, and [`Query::typed`] reads those values into a type of the
//! program's, whose fields are named as the declaration names them. A
//! value field's value is `null` where the page did not give it, so its
//! field is an `Option`; a `number` transform gives a number, every other
//! a text; a list gives a `Vec` and rows a `Vec` of a type of their own.

use goodfetch_protocol::page::read::{PageRead, PageReadParams, UnreadField};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::answer::Answer;
use crate::call::Query;
use crate::error::Error;

/// The values of a page's fields, read into `T`, and each value the page
/// did not give, with why.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FieldValues<T> {
    /// Each declared field's value, under the field's name.
    pub values: T,
    /// Each null the page did not give, in the order read.
    pub unread: Vec<UnreadField>,
}

impl Query<Answer<PageRead>, PageReadParams> {
    /// The read whose values are read into `T`, a type whose fields are
    /// named as the declared fields are, such as a struct deriving
    /// [`serde::Deserialize`].
    ///
    /// # Errors
    ///
    /// Once sent, `internal` when the values do not fit `T`, and
    /// the errors of the read.
    pub fn typed<T: DeserializeOwned + Send + 'static>(
        self,
    ) -> Query<FieldValues<T>, PageReadParams> {
        self.try_map(typed)
    }
}

/// `read`'s values read into `T`.
fn typed<T: DeserializeOwned>(read: Answer<PageRead>) -> Result<FieldValues<T>, Error> {
    let read = read.into_inner();
    let values = serde_json::from_value(Value::Object(read.values)).map_err(Error::shape)?;
    Ok(FieldValues {
        values,
        unread: read.unread,
    })
}
