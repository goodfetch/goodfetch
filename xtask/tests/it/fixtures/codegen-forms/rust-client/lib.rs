//! A stand-in for Collie's client crate with the generated handles (ADR
//! 0099): the call builders and the host a handle reaches its engine
//! through, with the real crate's signatures and bounds but no I/O, so
//! `cargo check` reads what the generator writes against them.

use std::marker::PhantomData;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

mod generated {
    pub(crate) mod blocking;
    pub(crate) mod handles;
}

/// What a program writes against the reads (ADR 0099): each answers its
/// value at once, from the answer that opened its handle, by a method of
/// the async handle and of its blocking twin.
#[expect(dead_code, reason = "rustc checks the reads' types; nothing runs them")]
fn reads(run: &generated::handles::Run, blocking: &generated::blocking::Run) {
    let _: String = run.label();
    let _: bool = run.is_resumed();
    let _: String = blocking.label();
    let _: bool = blocking.is_resumed();
}

mod markers {
    /// ADR 0030's run-time warning of a deprecated or Experimental entry.
    pub(crate) fn warn(
        _name: &'static str,
        _experimental: bool,
        _deprecation: Option<&'static str>,
    ) {
    }
}

/// What a call fails with.
#[derive(Debug)]
pub struct Error;

/// A Connection to one engine.
#[derive(Clone, Debug)]
pub struct Connection;

/// A call that waits for its reply, `T`, with its params, `P`.
#[must_use]
pub struct Call<T, P = Value> {
    params: P,
    reply: PhantomData<fn() -> T>,
}

impl<T, P> Call<T, P> {
    /// Fails the call if its reply has not come within `timeout`.
    pub fn timeout(self, _timeout: Duration) -> Self {
        self
    }

    /// Waits for the reply as long as the Connection lasts.
    pub fn no_timeout(self) -> Self {
        self
    }

    pub(crate) fn wire_timeout(self) -> Self {
        self
    }

    pub(crate) fn edit(mut self, edit: impl FnOnce(&mut P)) -> Self {
        edit(&mut self.params);
        self
    }

    pub(crate) fn map<U>(self, _map: impl FnOnce(T) -> U + Send + 'static) -> Call<U, P>
    where
        T: 'static,
    {
        Call {
            params: self.params,
            reply: PhantomData,
        }
    }
}

/// A whole result: its typed view and the JSON it came from.
pub struct Answer<T>(PhantomData<fn() -> T>);

impl<'de, T: DeserializeOwned> serde::Deserialize<'de> for Answer<T> {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(PhantomData))
    }
}

/// A raw call of an operation, its params and its result the wire's JSON.
#[must_use]
pub struct RawCall;

/// A question the engine answers at once.
#[must_use]
pub struct Query<T, P = Value> {
    params: P,
    reply: PhantomData<fn() -> T>,
}

impl<T, P> Query<T, P> {
    pub(crate) fn edit(mut self, edit: impl FnOnce(&mut P)) -> Self {
        edit(&mut self.params);
        self
    }

    pub(crate) fn map<U>(self, _map: impl FnOnce(T) -> U + Send + 'static) -> Query<U, P>
    where
        T: 'static,
    {
        Query {
            params: self.params,
            reply: PhantomData,
        }
    }
}

pub(crate) mod host {
    use super::{Call, Connection, DeserializeOwned, PhantomData, Query, RawCall, Serialize};

    /// What a handle reaches its engine through.
    #[derive(Clone, Debug)]
    pub(crate) struct Host<Id> {
        connection: Connection,
        id: Id,
    }

    impl<Id> Host<Id> {
        pub(crate) fn id(&self) -> &Id {
            &self.id
        }

        pub(crate) fn connection(&self) -> &Connection {
            &self.connection
        }

        pub(crate) fn child<C>(&self, id: C) -> Host<C> {
            Host {
                connection: self.connection.clone(),
                id,
            }
        }

        pub(crate) fn call<T, P>(&self, _method: &'static str, params: P) -> Call<T, P>
        where
            T: DeserializeOwned + Send + 'static,
            P: Serialize + Send + 'static,
        {
            Call {
                params,
                reply: PhantomData,
            }
        }

        pub(crate) fn raw(
            &self,
            _operation: &str,
            _namespace: &'static str,
            _param: &'static str,
        ) -> RawCall
        where
            Id: Serialize,
        {
            RawCall
        }

        pub(crate) fn query<T, P>(&self, _method: &'static str, params: P) -> Query<T, P>
        where
            T: DeserializeOwned + Send + 'static,
            P: Serialize + Send + 'static,
        {
            Query {
                params,
                reply: PhantomData,
            }
        }

        pub(crate) fn close<P>(&self, _method: &'static str, params: P) -> Call<(), P>
        where
            P: Serialize + Send + 'static,
        {
            Call {
                params,
                reply: PhantomData,
            }
        }
    }
}

/// The API for a program that does not use async.
pub mod blocking {
    use std::time::Duration;

    use serde_json::Value;

    use crate::{Connection, Error};

    /// A call that waits for its reply, sent by `send()`.
    #[must_use]
    pub struct Call<T, P = Value> {
        call: crate::Call<T, P>,
    }

    impl<T, P> Call<T, P> {
        pub(crate) fn new(call: crate::Call<T, P>, _connection: &Connection) -> Self {
            Self { call }
        }

        pub(crate) fn edit(self, edit: impl FnOnce(&mut P)) -> Self {
            Self {
                call: self.call.edit(edit),
            }
        }

        /// Fails the call if its reply has not come within `timeout`.
        pub fn timeout(self, timeout: Duration) -> Self {
            Self {
                call: self.call.timeout(timeout),
            }
        }

        /// Waits for the reply as long as the Connection lasts.
        pub fn no_timeout(self) -> Self {
            Self {
                call: self.call.no_timeout(),
            }
        }

        /// Sends the call and blocks until its reply.
        pub fn send(self) -> Result<T, Error> {
            Err(Error)
        }
    }

    /// A raw call, sent by `send()`.
    #[must_use]
    pub struct RawCall {
        raw: crate::RawCall,
    }

    impl RawCall {
        pub(crate) fn new(raw: crate::RawCall, _connection: &Connection) -> Self {
            Self { raw }
        }
    }

    /// A question the engine answers at once, sent by `send()`.
    #[must_use]
    pub struct Query<T, P = Value> {
        query: crate::Query<T, P>,
    }

    impl<T, P> Query<T, P> {
        pub(crate) fn new(query: crate::Query<T, P>, _connection: &Connection) -> Self {
            Self { query }
        }

        pub(crate) fn edit(self, edit: impl FnOnce(&mut P)) -> Self {
            Self {
                query: self.query.edit(edit),
            }
        }

        /// Sends the query and blocks until its answer.
        pub fn send(self) -> Result<T, Error> {
            Err(Error)
        }
    }
}
