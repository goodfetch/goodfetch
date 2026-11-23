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
//!
//! The store is one server, a Sentinel deployment or a Cluster, as
//! `politeness.store` names it. Its password, resolved from the setting's
//! Secret reference (ADR 0015), never travels in clear off loopback: an
//! address off loopback is `rediss://`, and a Sentinel's elected primary is
//! reached over TLS whenever a password is set or a sentinel is.

use std::fmt;
use std::time::Duration;

use futures_util::StreamExt as _;
use goodfetch_protocol::politeness::store::{
    PolitenessStoreConfiguration, StoreAddressing, StoreNamespace, StoreUrl, is_loopback,
};
use redis::aio::{ConnectionLike, MultiplexedConnection};
use redis::cluster_async::ClusterConnection;
use redis::sentinel::{Sentinel, SentinelNodeConnectionInfo};
use redis::{ConnectionAddr, ConnectionInfo, IntoConnectionInfo as _, Script, TlsMode};
use secrecy::{ExposeSecret as _, SecretString};

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
    /// The store could not be reached, or the connection to it dropped.
    #[error("the Politeness store is unreachable")]
    Unreachable(Party),
    /// The store refused the engine: no password, another password, or a
    /// user it denies a command.
    #[error("the Politeness store refused its user or password")]
    Refused(Party),
    /// The primary a Sentinel elected is off loopback and reached without
    /// TLS, which would carry every key in clear.
    #[error("the Politeness store's Sentinel elected a primary off loopback without TLS")]
    Plaintext,
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
        if matches!(error.kind(), redis::ErrorKind::AuthenticationFailed)
            || matches!(error.code(), Some("NOAUTH" | "WRONGPASS" | "NOPERM"))
        {
            Self::Refused(party)
        } else if error.is_io_error()
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

    /// Loads each script, on every primary of a Cluster.
    async fn load(&self, connection: &mut Connection) -> Result<(), StoreError> {
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
    /// The layout under `namespace`, which holds no brace, so its keys'
    /// only hash tag is their Politeness key's.
    fn new(namespace: &StoreNamespace) -> Self {
        Self {
            namespace: namespace.as_str().to_owned(),
        }
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
    /// The client a key's waiters subscribe through: the server, the
    /// primary a Sentinel elected, or a Cluster's first node, since a
    /// Cluster carries every publication to each of its nodes.
    subscriber: redis::Client,
    connection: Connection,
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
    /// Connects to the store `configuration` names, with `password`, which
    /// its Secret reference resolved to, and loads its scripts.
    ///
    /// # Errors
    ///
    /// [`StoreError::Refused`] when the store refuses the user or password,
    /// [`StoreError::Plaintext`] for a Sentinel's primary off loopback
    /// without TLS, else the store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it drops the connection; scripts it loaded stay loaded.
    pub async fn connect(
        configuration: &PolitenessStoreConfiguration,
        password: Option<&SecretString>,
    ) -> Result<Self, StoreError> {
        let authentication = Authentication {
            username: configuration.username.as_deref(),
            password,
        };
        let (subscriber, mut connection) =
            reach(&configuration.addressing, &authentication).await?;
        let scripts = Scripts::new();
        scripts.load(&mut connection).await?;
        Ok(Self {
            subscriber,
            connection,
            layout: Layout::new(&configuration.namespace),
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
        let mut subscription = self.subscriber.get_async_pubsub().await?;
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

/// The connection the store's scripts run on: one server's, or a
/// Cluster's, which sends each call to its key's slot.
#[derive(Clone)]
enum Connection {
    One(MultiplexedConnection),
    Cluster(ClusterConnection),
}

impl ConnectionLike for Connection {
    fn req_packed_command<'a>(
        &'a mut self,
        command: &'a redis::Cmd,
    ) -> redis::RedisFuture<'a, redis::Value> {
        match self {
            Self::One(connection) => connection.req_packed_command(command),
            Self::Cluster(connection) => connection.req_packed_command(command),
        }
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        pipeline: &'a redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> redis::RedisFuture<'a, Vec<redis::Value>> {
        match self {
            Self::One(connection) => connection.req_packed_commands(pipeline, offset, count),
            Self::Cluster(connection) => connection.req_packed_commands(pipeline, offset, count),
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            Self::One(connection) => connection.get_db(),
            Self::Cluster(connection) => connection.get_db(),
        }
    }
}

/// The user and password the store is reached with.
struct Authentication<'a> {
    username: Option<&'a str>,
    password: Option<&'a SecretString>,
}

impl Authentication<'_> {
    /// `settings` with this user and password.
    fn on(&self, mut settings: redis::RedisConnectionInfo) -> redis::RedisConnectionInfo {
        if let Some(username) = self.username {
            settings = settings.set_username(username);
        }
        if let Some(password) = self.password {
            settings = settings.set_password(password.expose_secret());
        }
        settings
    }

    /// `url`'s connection, with this user and password.
    fn at(&self, url: &StoreUrl) -> Result<ConnectionInfo, StoreError> {
        let info = url.as_str().into_connection_info()?;
        let settings = self.on(info.redis_settings().clone());
        Ok(info.set_redis_settings(settings))
    }
}

/// Reaches the store `addressing` names: the client a key's waiters
/// subscribe through, and the connection its scripts run on.
#[expect(
    clippy::disallowed_methods,
    reason = "the Politeness store's client opens its own sockets, which the Simulation never carries (ADR 0060)"
)]
async fn reach(
    addressing: &StoreAddressing,
    authentication: &Authentication<'_>,
) -> Result<(redis::Client, Connection), StoreError> {
    match addressing {
        StoreAddressing::Server { url } => {
            let client = redis::Client::open(authentication.at(url)?)?;
            let connection = client.get_multiplexed_async_connection().await?;
            Ok((client, Connection::One(connection)))
        }
        StoreAddressing::Sentinel { service, sentinels } => {
            let mut sentinel = Sentinel::build(sentinels.iter().map(StoreUrl::as_str).collect())?;
            let mut primary = SentinelNodeConnectionInfo::default().set_redis_connection_info(
                authentication.on(redis::RedisConnectionInfo::default()),
            );
            if authentication.password.is_some() || sentinels.iter().any(StoreUrl::tls) {
                primary = primary.set_tls_mode(TlsMode::Secure);
            }
            let client = sentinel.async_master_for(service, Some(&primary)).await?;
            if plaintext_off_loopback(client.get_connection_info().addr()) {
                return Err(StoreError::Plaintext);
            }
            let connection = client.get_multiplexed_async_connection().await?;
            Ok((client, Connection::One(connection)))
        }
        StoreAddressing::Cluster { nodes } => {
            let seeds = nodes
                .iter()
                .map(|node| authentication.at(node))
                .collect::<Result<Vec<_>, _>>()?;
            let cluster = redis::cluster::ClusterClient::builder(seeds.clone()).build()?;
            let first = seeds
                .into_iter()
                .next()
                .expect("invariant: a Cluster client is built from one node at least");
            let client = redis::Client::open(first)?;
            let connection = cluster.get_async_connection().await?;
            Ok((client, Connection::Cluster(connection)))
        }
    }
}

/// Whether `address` is reached without TLS off loopback.
fn plaintext_off_loopback(address: &ConnectionAddr) -> bool {
    matches!(address, ConnectionAddr::Tcp(host, _port) if !is_loopback(host))
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
    use goodfetch_protocol::politeness::store::StoreNamespace;
    use redis::ConnectionAddr;

    use super::{Layout, plaintext_off_loopback};

    /// Every part of a key shares one hash tag, the whole key, even when
    /// the key holds a brace that would otherwise end the tag early.
    #[test]
    fn every_part_of_a_key_shares_one_hash_tag() {
        let layout = Layout::new(&StoreNamespace::parse("crawl").unwrap());

        assert_eq!(
            layout.name("example.com", "permits"),
            "crawl:politeness:0:{example.com}:permits"
        );
        assert_eq!(
            layout.name("a}b{c%", "wakes"),
            "crawl:politeness:0:{a%7Db%7Bc%25}:wakes"
        );
    }

    /// A primary a Sentinel elects is taken without TLS on loopback only,
    /// since every key would travel in clear (ADR 0053).
    #[test]
    fn an_elected_primary_without_tls_is_taken_on_loopback_only() {
        let tcp = |host: &str| ConnectionAddr::Tcp(host.to_owned(), 6379);
        let tls = ConnectionAddr::TcpTls {
            host: "valkey.example".to_owned(),
            port: 6379,
            insecure: false,
            tls_params: None,
        };

        assert!(plaintext_off_loopback(&tcp("10.0.0.5")));
        assert!(plaintext_off_loopback(&tcp("valkey.example")));
        assert!(!plaintext_off_loopback(&tcp("127.0.0.1")));
        assert!(!plaintext_off_loopback(&tcp("::1")));
        assert!(!plaintext_off_loopback(&tls));
    }
}
