//! The Politeness store (ADR 0053): a Valkey or Redis server the user
//! provides, under a namespace, through which engines on several machines
//! share Politeness. A Permit is a lease with an expiry, renewed while its
//! request lasts, and a key's rate is GCRA; both are Lua scripts that read
//! the server's `TIME`, never an engine's clock, so engines whose clocks
//! drift agree (ADR 0059). Every key of one Politeness key carries the
//! same hash tag, so each script runs on one Cluster slot. The scripts are
//! loaded once, when the store connects, and called by `EVALSHA`
//! (ADR 0140's `E6.script_sends`).
//!
//! This module is the translation point of the store's failures (ADR 0127):
//! the client's error becomes a [`StoreError`] here, and nothing else reads
//! its text.
//!
//! Its key and script layout is a contract format whose major is frozen,
//! as the Usage ledger's family keeps theirs (ADR 0053's Formats): every key
//! lives under `<namespace>:politeness:<major>:`.

use std::fmt;
use std::time::Duration;

use futures_util::StreamExt as _;
use redis::Script;
use redis::aio::MultiplexedConnection;

/// The layout's frozen major: every key and channel lives under it, and a
/// newer layout only adds.
pub const LAYOUT_MAJOR: u32 = 0;

/// What a [`PolitenessStore::take`] got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Lease {
    /// The Permit is held until its lease expires or it is given back;
    /// `in_flight` counts it.
    Granted {
        /// The key's Permits held, every holder's together.
        in_flight: u64,
    },
    /// The key holds its cap of Permits; nothing was taken.
    Full {
        /// The key's Permits held, every holder's together.
        in_flight: u64,
    },
}

/// What a [`PolitenessStore::pace`] answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pace {
    /// The start is admitted, and counted in the key's rate.
    Now,
    /// No start is admitted before this much time has passed; nothing was
    /// counted.
    Wait(Duration),
}

/// A failure of the Politeness store.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A namespace that is empty, or holds a brace, which would make a hash
    /// tag of its own and move keys across Cluster slots.
    #[error("the Politeness store's namespace is empty or holds `{{` or `}}`")]
    Namespace,
    /// The store could not be reached, or the connection to it dropped.
    #[error("the Politeness store is unreachable")]
    Unreachable(Party),
    /// The store answered with a failure this module does not know.
    #[error("the Politeness store failed")]
    Unrecognized(Party),
}

/// The store client's own failure, kept for logs and never matched on
/// outside this module (ADR 0127). It holds the failure's category only,
/// never its text, which may name the server.
pub struct Party {
    category: String,
}

impl fmt::Debug for Party {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.category)
    }
}

impl From<redis::RedisError> for StoreError {
    fn from(error: redis::RedisError) -> Self {
        let party = Party {
            category: format!("{:?}", error.kind()),
        };
        if error.is_io_error()
            || error.is_connection_dropped()
            || error.is_connection_refusal()
            || error.is_timeout()
        {
            Self::Unreachable(party)
        } else {
            tracing::warn!(category = %party.category, "the Politeness store failed");
            Self::Unrecognized(party)
        }
    }
}

/// The scripts, each loaded once per store.
struct Scripts {
    take: Script,
    renew: Script,
    give_back: Script,
    pace: Script,
}

impl Scripts {
    fn new() -> Self {
        Self {
            take: Script::new(include_str!("store/take.lua")),
            renew: Script::new(include_str!("store/renew.lua")),
            give_back: Script::new(include_str!("store/give_back.lua")),
            pace: Script::new(include_str!("store/pace.lua")),
        }
    }

    async fn load(&self, connection: &mut MultiplexedConnection) -> Result<(), StoreError> {
        for script in [&self.take, &self.renew, &self.give_back, &self.pace] {
            script.prepare_invoke().load_async(connection).await?;
        }
        Ok(())
    }
}

/// Where a namespace keeps a Politeness key's state.
struct Layout {
    namespace: String,
}

impl Layout {
    fn new(namespace: &str) -> Result<Self, StoreError> {
        if namespace.is_empty() || namespace.contains(['{', '}']) {
            return Err(StoreError::Namespace);
        }
        Ok(Self {
            namespace: namespace.to_owned(),
        })
    }

    /// The name of `part` of `key`'s state, whose hash tag is the key, so
    /// that every part of one key lands on one Cluster slot. A brace in the
    /// key is escaped, so that it cannot end the tag early.
    fn name(&self, key: &str, part: &str) -> String {
        let mut tag = String::with_capacity(key.len());
        for character in key.chars() {
            match character {
                '%' => tag.push_str("%25"),
                '{' => tag.push_str("%7B"),
                '}' => tag.push_str("%7D"),
                other => tag.push(other),
            }
        }
        format!(
            "{}:politeness:{LAYOUT_MAJOR}:{{{tag}}}:{part}",
            self.namespace
        )
    }
}

/// A connection to a Politeness store under one namespace, its scripts
/// loaded.
pub struct PolitenessStore {
    client: redis::Client,
    connection: MultiplexedConnection,
    layout: Layout,
    scripts: Scripts,
}

impl fmt::Debug for PolitenessStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolitenessStore")
            .finish_non_exhaustive()
    }
}

impl PolitenessStore {
    /// Connects to the store at `url` under `namespace` and loads its
    /// scripts.
    ///
    /// # Errors
    ///
    /// [`StoreError::Namespace`] for a namespace that cannot be one, else
    /// the store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it drops the connection; scripts it loaded stay loaded.
    #[expect(
        clippy::disallowed_methods,
        reason = "the Politeness store's client opens its own sockets, which the Simulation never carries (ADR 0060)"
    )]
    pub async fn connect(url: &str, namespace: &str) -> Result<Self, StoreError> {
        let layout = Layout::new(namespace)?;
        let client = redis::Client::open(url)?;
        let mut connection = client.get_multiplexed_async_connection().await?;
        let scripts = Scripts::new();
        scripts.load(&mut connection).await?;
        Ok(Self {
            client,
            connection,
            layout,
            scripts,
        })
    }

    /// Renews `holder`'s lease of a Permit of `key` for `lease` from now,
    /// while its request lasts. `false` means the lease expired first and
    /// the Permit is lost: its room may be another holder's already.
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave the lease renewed or not.
    pub async fn renew(
        &self,
        key: &str,
        holder: &str,
        lease: Duration,
    ) -> Result<bool, StoreError> {
        Ok(self
            .scripts
            .renew
            .key(self.layout.name(key, "permits"))
            .arg(holder)
            .arg(milliseconds(lease))
            .invoke_async(&mut self.connection.clone())
            .await?)
    }

    /// Counts a start of `key` in its rate, one start per `interval` with
    /// `burst` more admitted at once, or says how long to wait before one
    /// is admitted: a rate wait sleeps until that deadline.
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave the start counted.
    pub async fn pace(
        &self,
        key: &str,
        interval: Duration,
        burst: u32,
    ) -> Result<Pace, StoreError> {
        let (admitted, wait): (bool, u64) = self
            .scripts
            .pace
            .key(self.layout.name(key, "pace"))
            .arg(milliseconds(interval))
            .arg(burst)
            .invoke_async(&mut self.connection.clone())
            .await?;
        Ok(if admitted {
            Pace::Now
        } else {
            Pace::Wait(Duration::from_millis(wait))
        })
    }

    /// Listens for the Permits of `key` given back, on a connection of its
    /// own.
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it drops the subscription.
    pub async fn wakes(&self, key: &str) -> Result<Wakes, StoreError> {
        let mut subscription = self.client.get_async_pubsub().await?;
        subscription
            .subscribe(self.layout.name(key, "wakes"))
            .await?;
        Ok(Wakes {
            messages: subscription.into_on_message(),
        })
    }

    /// Takes a Permit of `key` for `holder`, leased for `lease`, unless the
    /// key holds `cap` Permits already.
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave a Permit taken, which its lease frees.
    pub async fn take(
        &self,
        key: &str,
        holder: &str,
        cap: u64,
        lease: Duration,
    ) -> Result<Lease, StoreError> {
        let (granted, in_flight): (bool, u64) = self
            .scripts
            .take
            .key(self.layout.name(key, "permits"))
            .arg(holder)
            .arg(cap)
            .arg(milliseconds(lease))
            .invoke_async(&mut self.connection.clone())
            .await?;
        Ok(if granted {
            Lease::Granted { in_flight }
        } else {
            Lease::Full { in_flight }
        })
    }

    /// Gives `holder`'s Permit of `key` back, which wakes the key's
    /// waiters. A Permit whose lease expired gives nothing back.
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave the Permit held, which its lease frees.
    pub async fn give_back(&self, key: &str, holder: &str) -> Result<(), StoreError> {
        let _given: bool = self
            .scripts
            .give_back
            .key(self.layout.name(key, "permits"))
            .arg(holder)
            .arg(self.layout.name(key, "wakes"))
            .invoke_async(&mut self.connection.clone())
            .await?;
        Ok(())
    }
}

/// The Permits of one key given back, as the store publishes them.
pub struct Wakes {
    messages: redis::aio::PubSubStream,
}

impl fmt::Debug for Wakes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Wakes").finish_non_exhaustive()
    }
}

impl Wakes {
    /// Waits for the next Permit of the key given back; `None` once the
    /// connection ended.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe: a wake not yet received stays queued.
    pub async fn next(&mut self) -> Option<()> {
        self.messages.next().await.map(|_message| ())
    }
}

/// `duration` in whole milliseconds, at least 1, so a lease never expires
/// as it is taken.
fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::{Layout, StoreError};

    /// Every part of a key shares one hash tag, the whole key, even when
    /// the key holds a brace that would otherwise end the tag early.
    #[test]
    fn every_part_of_a_key_shares_one_hash_tag() {
        let layout = Layout::new("crawl").unwrap();

        assert_eq!(
            layout.name("example.com", "permits"),
            "crawl:politeness:0:{example.com}:permits"
        );
        assert_eq!(
            layout.name("a}b{c%", "wakes"),
            "crawl:politeness:0:{a%7Db%7Bc%25}:wakes"
        );
    }

    /// A namespace holding a brace would make a hash tag of its own.
    #[test]
    fn a_namespace_with_a_brace_or_none_is_refused() {
        assert!(matches!(Layout::new(""), Err(StoreError::Namespace)));
        assert!(matches!(Layout::new("a{b}"), Err(StoreError::Namespace)));
    }
}
