//! The engine: an identity and a composition, checked once at start, which
//! serves Connections. An Attached engine serves the one Connection its
//! network gives it and stops when that Connection ends (ADR 0024); the
//! Daemon joins with the ticket that adds it.

use std::sync::Arc;

use crate::composition::{Composition, CompositionError};
use crate::product::Identity;
use crate::rpc::{self, Dispatch};
use crate::tasks::TaskOwner;
use crate::transport::{Connection, Network, TransportError};

/// An engine of one Product, composed and ready to serve.
#[derive(Clone)]
pub struct Engine {
    dispatch: Arc<Dispatch>,
}

impl Engine {
    /// The engine `composition` makes for the Product `identity` names.
    pub fn new(identity: Identity, composition: &Composition) -> Result<Self, CompositionError> {
        Ok(Self {
            dispatch: Arc::new(Dispatch {
                identity: Arc::new(identity),
                handlers: composition.handlers()?,
            }),
        })
    }

    /// What it runs as.
    pub fn identity(&self) -> &Identity {
        &self.dispatch.identity
    }

    /// Serves `connection` until it ends, its tasks those of `owner`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves its calls to `owner`, which stops them.
    pub async fn serve(&self, connection: Connection, owner: &TaskOwner) {
        rpc::serve(Arc::clone(&self.dispatch), connection, owner).await;
    }

    /// Runs as an Attached engine: serves the one Connection `network`
    /// gives it, then stops every task it started.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels nothing it started; run it to its end.
    pub async fn run_attached(&self, mut network: Network) -> Result<(), TransportError> {
        let connection = network.attached_connection().await?;
        let owner = TaskOwner::root();
        self.serve(connection, &owner).await;
        owner.stop().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
