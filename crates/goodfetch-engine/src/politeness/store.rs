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
//! address off loopback is `rediss://`, a Sentinel's elected primary is
//! reached over TLS whenever a password is set or a sentinel is, and a
//! Cluster reached on loopback without TLS is refused when its `CLUSTER
//! SLOTS` names a node off loopback, which the client would reach without
//! TLS too.
//!
//! Size: one boundary module owning the Valkey/Redis client connection,
//! error translation (ADR 0127), key layout, and Lua script invocations.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use futures_util::StreamExt as _;
use goodfetch_protocol::politeness::store::{
    POLITENESS_STORE_LAYOUT_VERSION, PolitenessStoreConfiguration, PolitenessStoreLayout,
    StoreAddressing, StoreKeyPart, StoreNamespace, StoreScriptLayout, StoreUrl, is_loopback,
};
use redis::aio::{ConnectionLike, MultiplexedConnection, PubSubSink, PubSubStream};
use redis::cluster_async::ClusterConnection;
use redis::sentinel::{Sentinel, SentinelNodeConnectionInfo};
use redis::{ConnectionAddr, ConnectionInfo, IntoConnectionInfo as _, Script, TlsMode};
use secrecy::{ExposeSecret as _, SecretString};

use crate::dns::HostMapping;

/// Default timeout for a single Politeness store command or connection
/// attempt before it is treated as [`StoreError::Unreachable`].
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// The primary a Sentinel elected, or a node a Cluster on loopback
    /// announced, is off loopback and reached without TLS, which would
    /// carry every key in clear.
    #[error("the Politeness store names a server off loopback reached without TLS")]
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

/// What a [`PolitenessStore::delegate`] answered.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DelegationAcquire {
    /// This engine now holds the key's Delegation, with any shared state
    /// stored under the key.
    Granted {
        /// The key's shared state parts (`part_name -> json_bytes`).
        state: HashMap<String, Vec<u8>>,
    },
    /// Another engine holds the key's Delegation.
    HeldByOther {
        /// The holding engine's ID.
        holder: String,
    },
    /// Unexpired Permits are in flight in the store, so the key is shared.
    Shared,
}

/// What a [`PolitenessStore::peek_shared`] answered.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PeekShared {
    /// No other engine holds a Delegation: the key's current in-flight count
    /// and shared state parts.
    Ready {
        /// Active unexpired Permits in the store.
        in_flight: u64,
        /// The key's shared state parts (`part_name -> json_bytes`).
        state: HashMap<String, Vec<u8>>,
    },
    /// Another engine holds a live Delegation; a recall was published on
    /// the key's wakes channel.
    Recalled,
}

/// What a [`PolitenessStore::take_shared`] answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TakeShared {
    /// The Permit lease is granted; `in_flight` counts it.
    Granted {
        /// Active unexpired Permits in the store.
        in_flight: u64,
    },
    /// The key is at its cap; nothing was taken.
    Full {
        /// Active unexpired Permits in the store.
        in_flight: u64,
    },
    /// Another engine holds a live Delegation; a recall was published on
    /// the key's wakes channel.
    Recalled,
    /// GCRA rate pacing asks to wait this long before admitting a start.
    PaceWait(Duration),
}

/// What a [`PolitenessStore::give_back_shared`] answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GiveBackShared {
    /// Whether the Permit was still active when removed.
    pub given: bool,
    /// Whether no other engine holds an unexpired Permit of the key now.
    pub alone: bool,
}

/// One message arriving on a key's pub/sub wakes channel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WakeEvent {
    /// A Permit was given back.
    Wake,
    /// A recalled Delegation was given back.
    Recalled,
    /// Another engine recalls the Delegation held by `holder`.
    Recall {
        /// The engine ID being recalled.
        holder: String,
    },
    /// Another engine acquired the key's Delegation while this engine was idle.
    Delegated {
        /// The engine ID that acquired the Delegation.
        holder: String,
    },
}

/// Initial heartbeat parameters folded into [`PolitenessStore::delegate_with_heartbeat`]
/// so acquiring a Delegation costs a single `EVALSHA` (ADRs 0053 and 0142).
#[derive(Debug, Clone, Copy)]
pub struct DelegationHeartbeat {
    /// Heartbeat TTL (<= Permit lease).
    pub heartbeat_ttl: Duration,
    /// Configured `politeness.degraded_max`.
    pub degraded_max: Duration,
    /// Longest request timeout reported by this engine.
    pub timeout: Duration,
    /// Largest cap reported for the key (`None` = uncapped).
    pub cap: Option<u64>,
}

/// Parameters for [`PolitenessStore::heartbeat`] (and atomic reconnect
/// in-flight Permit registration, ADR 0053).
#[derive(Debug, Clone)]
pub struct HeartbeatParams<'a> {
    /// The Politeness key.
    pub key: &'a str,
    /// This engine's ID.
    pub engine_id: &'a str,
    /// Heartbeat TTL (no longer than the Permit lease).
    pub heartbeat_ttl: Duration,
    /// Configured `politeness.degraded_max`.
    pub degraded_max: Duration,
    /// Longest request timeout this engine reports for `key`.
    pub timeout: Duration,
    /// Largest cap any run on this engine reports for `key` (`None` = uncapped).
    pub cap: Option<u64>,
    /// Share this engine last received and now acknowledges (`0` if none yet).
    pub acked_share: u64,
    /// Permit lease duration used when registering `in_flight_tokens` on reconnect.
    pub permit_lease: Duration,
    /// In-flight Permit tokens registered atomically BEFORE freeing a stale
    /// reservation on reconnect (empty for a regular heartbeat).
    pub in_flight_tokens: &'a [String],
}

/// Acknowledgement returned by [`PolitenessStore::heartbeat`] (ADR 0053):
/// computed solely by the store's Lua script on server `TIME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartbeatAck {
    /// Degraded share granted to this engine for the key (`>= 1` when capped,
    /// `0` when uncapped).
    pub granted_share: u64,
    /// Largest cap reported by any active or stale-reserved engine for the key
    /// (`0` when uncapped).
    pub max_cap: u64,
    /// Longest request timeout reported by any engine for the key, in ms.
    pub max_timeout_ms: u64,
    /// Number of currently live engines sharing the key.
    pub live_engines: u64,
}

/// The scripts, each loaded once per store.
#[derive(Clone)]
struct Scripts {
    take: Script,
    renew: Script,
    give_back: Script,
    pace: Script,
    delegate: Script,
    renew_delegation: Script,
    release_delegation: Script,
    peek_shared: Script,
    take_shared: Script,
    give_back_shared: Script,
    save_state: Script,
    heartbeat: Script,
    leave_engine: Script,
}

impl Scripts {
    fn new() -> Self {
        Self {
            take: Script::new(include_str!("store/take.lua")),
            renew: Script::new(include_str!("store/renew.lua")),
            give_back: Script::new(include_str!("store/give_back.lua")),
            pace: Script::new(include_str!("store/pace.lua")),
            delegate: Script::new(include_str!("store/delegate.lua")),
            renew_delegation: Script::new(include_str!("store/renew_delegation.lua")),
            release_delegation: Script::new(include_str!("store/release_delegation.lua")),
            peek_shared: Script::new(include_str!("store/peek_shared.lua")),
            take_shared: Script::new(include_str!("store/take_shared.lua")),
            give_back_shared: Script::new(include_str!("store/give_back_shared.lua")),
            save_state: Script::new(include_str!("store/save_state.lua")),
            heartbeat: Script::new(include_str!("store/heartbeat.lua")),
            leave_engine: Script::new(include_str!("store/leave_engine.lua")),
        }
    }

    /// Loads each script, on every primary of a Cluster.
    async fn load(&self, connection: &mut StoreConnection) -> Result<(), StoreError> {
        for script in [
            &self.take,
            &self.renew,
            &self.give_back,
            &self.pace,
            &self.delegate,
            &self.renew_delegation,
            &self.release_delegation,
            &self.peek_shared,
            &self.take_shared,
            &self.give_back_shared,
            &self.save_state,
            &self.heartbeat,
            &self.leave_engine,
        ] {
            script.prepare_invoke().load_async(connection).await?;
        }
        Ok(())
    }
}

/// Where a namespace keeps a Politeness key's state.
#[derive(Clone)]
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

/// Parameters retained so [`PolitenessStore::reconnect`] can establish a fresh
/// connection and reload scripts after an outage (ADR 0053).
#[derive(Clone)]
struct ConnectTarget {
    configuration: PolitenessStoreConfiguration,
    password: Option<SecretString>,
    resolve: HostMapping,
}

/// A connection to a Politeness store under one namespace, its scripts
/// loaded.
#[derive(Clone)]
pub struct PolitenessStore {
    /// The client a key's waiters subscribe through: the server, the
    /// primary a Sentinel elected, or a Cluster's first node, since a
    /// Cluster carries every publication to each of its nodes.
    subscriber: redis::Client,
    connection: StoreConnection,
    layout: Layout,
    scripts: Scripts,
    target: ConnectTarget,
    command_timeout: Duration,
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
    /// [`StoreError::Plaintext`] for a Sentinel's primary, or a node a
    /// Cluster on loopback announces, off loopback without TLS, else the
    /// store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it drops the connection; scripts it loaded stay loaded.
    pub async fn connect(
        configuration: &PolitenessStoreConfiguration,
        password: Option<&SecretString>,
    ) -> Result<Self, StoreError> {
        Self::connect_with_resolve(configuration, password, &HostMapping::default()).await
    }

    /// Connects to the store `configuration` names, resolving host names
    /// through `resolve` (`network.resolve`, ADR 0059), with `password`, and
    /// loads its scripts.
    ///
    /// # Errors
    ///
    /// As [`PolitenessStore::connect`].
    ///
    /// # Cancel safety
    ///
    /// Dropping it drops the connection; scripts it loaded stay loaded.
    pub async fn connect_with_resolve(
        configuration: &PolitenessStoreConfiguration,
        password: Option<&SecretString>,
        resolve: &HostMapping,
    ) -> Result<Self, StoreError> {
        Self::connect_inner(configuration, password, resolve, DEFAULT_COMMAND_TIMEOUT).await
    }

    async fn connect_inner(
        configuration: &PolitenessStoreConfiguration,
        password: Option<&SecretString>,
        resolve: &HostMapping,
        command_timeout: Duration,
    ) -> Result<Self, StoreError> {
        let authentication = Authentication {
            username: configuration.username.as_deref(),
            password,
            resolve,
        };
        let (subscriber, mut connection) = tokio::time::timeout(
            command_timeout,
            reach(&configuration.addressing, &authentication),
        )
        .await
        .map_err(|_| timeout_error())??;
        let scripts = Scripts::new();
        tokio::time::timeout(command_timeout, scripts.load(&mut connection))
            .await
            .map_err(|_| timeout_error())??;
        Ok(Self {
            subscriber,
            connection,
            layout: Layout::new(&configuration.namespace),
            scripts,
            target: ConnectTarget {
                configuration: configuration.clone(),
                password: password.cloned(),
                resolve: resolve.clone(),
            },
            command_timeout,
        })
    }

    /// Sets the timeout for store commands and reconnection attempts.
    #[must_use]
    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout.max(Duration::from_millis(50));
        self
    }

    /// Opens a fresh connection to the same store and reloads all scripts
    /// after an outage (ADR 0053).
    ///
    /// # Errors
    ///
    /// As [`PolitenessStore::connect`].
    ///
    /// # Cancel safety
    ///
    /// Dropping it drops the new connection attempt and leaves the caller's
    /// existing handle unchanged.
    pub async fn reconnect(&self) -> Result<Self, StoreError> {
        Self::connect_inner(
            &self.target.configuration,
            self.target.password.as_ref(),
            &self.target.resolve,
            self.command_timeout,
        )
        .await
    }

    async fn timed<T, F>(&self, future: F) -> Result<T, StoreError>
    where
        F: std::future::Future<Output = Result<T, redis::RedisError>>,
    {
        tokio::time::timeout(self.command_timeout, future)
            .await
            .map_err(|_| timeout_error())?
            .map_err(StoreError::from)
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
        let mut connection = self.connection.clone();
        self.timed(
            self.scripts
                .renew
                .key(self.layout.name(key, "permits"))
                .arg(holder)
                .arg(milliseconds(lease))
                .invoke_async(&mut connection),
        )
        .await
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

    /// Describes this store's frozen-major key and script layout for the
    /// Compatibility corpus (ADRs 0053 and 0066).
    #[must_use]
    pub fn layout_description() -> PolitenessStoreLayout {
        let part = |name: &str, kind: &str, purpose: &str| StoreKeyPart {
            part: name.to_owned(),
            kind: kind.to_owned(),
            purpose: purpose.to_owned(),
        };
        let script = |name: &str, parts: &[&str], uses_server_time: bool| StoreScriptLayout {
            name: name.to_owned(),
            key_parts: parts.iter().map(|part| (*part).to_owned()).collect(),
            uses_server_time,
        };
        PolitenessStoreLayout {
            version: POLITENESS_STORE_LAYOUT_VERSION,
            layout_major: LAYOUT_MAJOR,
            key_template: "<namespace>:politeness:<major>:{<key>}:<part>".to_owned(),
            parts: vec![
                part(
                    "permits",
                    "zset",
                    "Active Permit lease tokens scored by expiry in server milliseconds.",
                ),
                part(
                    "pace",
                    "string",
                    "GCRA theoretical arrival time in server milliseconds.",
                ),
                part(
                    "delegation",
                    "string",
                    "Engine ID holding the key's Delegation, with PX expiry.",
                ),
                part(
                    "state",
                    "hash",
                    "Per-contribution shared key state JSON values (domain limits, proxy/provider limits, robots.txt).",
                ),
                part(
                    "wakes",
                    "pubsub",
                    "Wake notifications for returned Permits and Delegation recalls.",
                ),
                part(
                    "engines",
                    "hash",
                    "Per-engine heartbeat records, stale reservations and granted degraded shares.",
                ),
            ],
            scripts: vec![
                script("take", &["permits"], true),
                script("renew", &["permits"], true),
                script("give_back", &["permits", "wakes"], false),
                script("pace", &["pace"], true),
                script(
                    "delegate",
                    &["delegation", "permits", "state", "engines"],
                    true,
                ),
                script("renew_delegation", &["delegation"], true),
                script(
                    "release_delegation",
                    &["delegation", "permits", "state", "wakes"],
                    true,
                ),
                script(
                    "peek_shared",
                    &["delegation", "permits", "state", "engines", "wakes"],
                    true,
                ),
                script(
                    "take_shared",
                    &["delegation", "permits", "pace", "state", "engines", "wakes"],
                    true,
                ),
                script(
                    "give_back_shared",
                    &["permits", "state", "engines", "wakes"],
                    true,
                ),
                script("save_state", &["state"], false),
                script("heartbeat", &["engines", "permits", "delegation"], true),
                script("leave_engine", &["engines"], false),
            ],
        }
    }

    /// Acquires a Delegation of `key` for `engine_id` with expiry `ttl`,
    /// returning any shared state stored under `key` when granted.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn delegate(
        &self,
        key: &str,
        engine_id: &str,
        ttl: Duration,
    ) -> Result<DelegationAcquire, StoreError> {
        self.delegate_inner(key, engine_id, ttl, None).await
    }

    /// Acquires a Delegation of `key` for `engine_id` with expiry `ttl`, and
    /// atomically registers this engine's initial heartbeat record in the same
    /// round trip (1 `EVALSHA`, ADRs 0053 and 0142).
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave the Delegation acquired or not.
    pub async fn delegate_with_heartbeat(
        &self,
        key: &str,
        engine_id: &str,
        ttl: Duration,
        heartbeat: DelegationHeartbeat,
    ) -> Result<DelegationAcquire, StoreError> {
        self.delegate_inner(key, engine_id, ttl, Some(heartbeat))
            .await
    }

    async fn delegate_inner(
        &self,
        key: &str,
        engine_id: &str,
        ttl: Duration,
        heartbeat: Option<DelegationHeartbeat>,
    ) -> Result<DelegationAcquire, StoreError> {
        let mut invocation = self.scripts.delegate.prepare_invoke();
        invocation
            .key(self.layout.name(key, "delegation"))
            .key(self.layout.name(key, "permits"))
            .key(self.layout.name(key, "state"))
            .key(self.layout.name(key, "engines"))
            .arg(engine_id)
            .arg(milliseconds(ttl));
        if let Some(hb) = heartbeat {
            invocation
                .arg(milliseconds(hb.heartbeat_ttl))
                .arg(milliseconds(hb.degraded_max))
                .arg(milliseconds(hb.timeout))
                .arg(hb.cap.unwrap_or(0))
                .arg(self.layout.name(key, "wakes"));
        }
        let mut connection = self.connection.clone();
        let (granted, values): (bool, Vec<Vec<u8>>) =
            self.timed(invocation.invoke_async(&mut connection)).await?;
        if granted {
            Ok(DelegationAcquire::Granted {
                state: decode_hash_pairs(values),
            })
        } else {
            let holder = values
                .into_iter()
                .next()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
            if holder.is_empty() {
                Ok(DelegationAcquire::Shared)
            } else {
                Ok(DelegationAcquire::HeldByOther { holder })
            }
        }
    }

    /// Renews `engine_id`'s Delegation of `key` for `ttl` on the server's clock.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn renew_delegation(
        &self,
        key: &str,
        engine_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError> {
        let mut connection = self.connection.clone();
        self.timed(
            self.scripts
                .renew_delegation
                .key(self.layout.name(key, "delegation"))
                .arg(engine_id)
                .arg(milliseconds(ttl))
                .invoke_async(&mut connection),
        )
        .await
    }

    /// Releases `engine_id`'s Delegation of `key`, promoting any in-flight
    /// `permits` to store leases of `lease`, saving `state` parts with
    /// `state_ttl`, and publishing `'recalled'` on `:wakes`.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn release_delegation(
        &self,
        key: &str,
        engine_id: &str,
        lease: Duration,
        state_ttl: Duration,
        permits: &[String],
        state: &[(&str, &[u8])],
    ) -> Result<bool, StoreError> {
        let mut invocation = self.scripts.release_delegation.prepare_invoke();
        invocation
            .key(self.layout.name(key, "delegation"))
            .key(self.layout.name(key, "permits"))
            .key(self.layout.name(key, "state"))
            .arg(engine_id)
            .arg(milliseconds(lease))
            .arg(self.layout.name(key, "wakes"))
            .arg(milliseconds(state_ttl))
            .arg(permits.len());
        for token in permits {
            invocation.arg(token);
        }
        for (part, value) in state {
            invocation.arg(*part).arg(*value);
        }
        let mut connection = self.connection.clone();
        self.timed(invocation.invoke_async(&mut connection)).await
    }

    /// Peeks at `key`'s active Permit count (including stale engines' reserved
    /// shares) and shared state hash, or recalls another engine's live
    /// Delegation through pub/sub.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn peek_shared(&self, key: &str, engine_id: &str) -> Result<PeekShared, StoreError> {
        let mut connection = self.connection.clone();
        let (status, in_flight, values): (u64, u64, Vec<Vec<u8>>) = self
            .timed(
                self.scripts
                    .peek_shared
                    .key(self.layout.name(key, "delegation"))
                    .key(self.layout.name(key, "permits"))
                    .key(self.layout.name(key, "state"))
                    .key(self.layout.name(key, "engines"))
                    .arg(engine_id)
                    .arg(self.layout.name(key, "wakes"))
                    .invoke_async(&mut connection),
            )
            .await?;
        if status == 2 {
            Ok(PeekShared::Recalled)
        } else {
            Ok(PeekShared::Ready {
                in_flight,
                state: decode_hash_pairs(values),
            })
        }
    }

    /// Takes a shared Permit of `key` for `engine_id` under `token`, checking
    /// `cap` (`None` = uncapped) against active Permits plus stale engines'
    /// reservations, optional GCRA `pace`, and optionally saving one `state`
    /// part; recalls another engine's live Delegation via pub/sub.
    ///
    /// # Errors
    ///
    /// The store's failure.
    #[expect(
        clippy::too_many_arguments,
        reason = "maps 1:1 to take_shared.lua's atomic parameters on one Cluster slot"
    )]
    pub async fn take_shared(
        &self,
        key: &str,
        engine_id: &str,
        token: &str,
        cap: Option<u64>,
        lease: Duration,
        pace: Option<(Duration, u32)>,
        state_ttl: Duration,
        state: Option<(&str, &[u8])>,
    ) -> Result<TakeShared, StoreError> {
        let (pace_interval, pace_burst) = match pace {
            Some((interval, burst)) => (milliseconds(interval), burst),
            None => (0, 0),
        };
        let (state_part, state_value): (&str, &[u8]) = state.unwrap_or(("", b""));
        let mut connection = self.connection.clone();
        let (status, value): (u64, u64) = self
            .timed(
                self.scripts
                    .take_shared
                    .key(self.layout.name(key, "delegation"))
                    .key(self.layout.name(key, "permits"))
                    .key(self.layout.name(key, "pace"))
                    .key(self.layout.name(key, "state"))
                    .key(self.layout.name(key, "engines"))
                    .arg(engine_id)
                    .arg(token)
                    .arg(cap.unwrap_or(0))
                    .arg(milliseconds(lease))
                    .arg(self.layout.name(key, "wakes"))
                    .arg(pace_interval)
                    .arg(pace_burst)
                    .arg(milliseconds(state_ttl))
                    .arg(state_part)
                    .arg(state_value)
                    .invoke_async(&mut connection),
            )
            .await?;
        Ok(match status {
            1 => TakeShared::Granted { in_flight: value },
            2 => TakeShared::Recalled,
            3 => TakeShared::PaceWait(Duration::from_millis(value)),
            _ => TakeShared::Full { in_flight: value },
        })
    }

    /// Gives `token`'s shared Permit of `key` back, optionally persisting one
    /// `state` part, publishing a wake, and reporting whether any other engine
    /// still holds an active Permit or heartbeat of `key`.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn give_back_shared(
        &self,
        key: &str,
        engine_id: &str,
        token: &str,
        state_ttl: Duration,
        state: Option<(&str, &[u8])>,
    ) -> Result<GiveBackShared, StoreError> {
        let (state_part, state_value): (&str, &[u8]) = state.unwrap_or(("", b""));
        let prefix = format!("{engine_id}:");
        let mut connection = self.connection.clone();
        let (given, alone): (bool, bool) = self
            .timed(
                self.scripts
                    .give_back_shared
                    .key(self.layout.name(key, "permits"))
                    .key(self.layout.name(key, "state"))
                    .key(self.layout.name(key, "engines"))
                    .arg(prefix)
                    .arg(token)
                    .arg(self.layout.name(key, "wakes"))
                    .arg(milliseconds(state_ttl))
                    .arg(state_part)
                    .arg(state_value)
                    .arg(engine_id)
                    .invoke_async(&mut connection),
            )
            .await?;
        Ok(GiveBackShared { given, alone })
    }

    /// Records a heartbeat for `params.key` on server `TIME`, atomically
    /// registering any `in_flight_tokens` as Permit leases first (on
    /// reconnect), and returns the store-computed degraded share (ADR 0053).
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave the heartbeat recorded or not.
    pub async fn heartbeat(
        &self,
        params: &HeartbeatParams<'_>,
    ) -> Result<HeartbeatAck, StoreError> {
        let mut invocation = self.scripts.heartbeat.prepare_invoke();
        invocation
            .key(self.layout.name(params.key, "engines"))
            .key(self.layout.name(params.key, "permits"))
            .key(self.layout.name(params.key, "delegation"))
            .arg(params.engine_id)
            .arg(milliseconds(params.heartbeat_ttl))
            .arg(milliseconds(params.degraded_max))
            .arg(milliseconds(params.timeout))
            .arg(params.cap.unwrap_or(0))
            .arg(params.acked_share)
            .arg(milliseconds(params.permit_lease))
            .arg(params.in_flight_tokens.len());
        for token in params.in_flight_tokens {
            invocation.arg(token);
        }
        let mut connection = self.connection.clone();
        let (granted_share, max_cap, max_timeout_ms, live_engines): (u64, u64, u64, u64) =
            self.timed(invocation.invoke_async(&mut connection)).await?;
        Ok(HeartbeatAck {
            granted_share,
            max_cap,
            max_timeout_ms,
            live_engines,
        })
    }

    /// Removes `engine_id`'s heartbeat record for `key` on clean exit (ADR 0053).
    ///
    /// # Errors
    ///
    /// The store's failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping it may leave the record removed or still present until expiry.
    pub async fn leave_engine(&self, key: &str, engine_id: &str) -> Result<bool, StoreError> {
        let mut connection = self.connection.clone();
        self.timed(
            self.scripts
                .leave_engine
                .key(self.layout.name(key, "engines"))
                .arg(engine_id)
                .invoke_async(&mut connection),
        )
        .await
    }

    /// Saves one `part` of `key`'s shared state hash in the store.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn save_state(
        &self,
        key: &str,
        state_ttl: Duration,
        part: &str,
        value: &[u8],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.clone();
        let _saved: u64 = self
            .timed(
                self.scripts
                    .save_state
                    .key(self.layout.name(key, "state"))
                    .arg(milliseconds(state_ttl))
                    .arg(part)
                    .arg(value)
                    .invoke_async(&mut connection),
            )
            .await?;
        Ok(())
    }

    /// Loads one `part` of `key`'s shared state hash from the store.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn load_state(&self, key: &str, part: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let mut connection = self.connection.clone();
        self.timed(
            redis::cmd("HGET")
                .arg(self.layout.name(key, "state"))
                .arg(part)
                .query_async(&mut connection),
        )
        .await
    }

    /// Opens one multiplexed pub/sub connection for this engine's wakes and
    /// Delegation recalls across every Politeness key (`E6.opens = 0`).
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn open_wake_bus(&self) -> Result<(WakeSubscriptions, WakeEvents), StoreError> {
        let pubsub = self.timed(self.subscriber.get_async_pubsub()).await?;
        let (sink, stream) = pubsub.split();
        Ok((
            WakeSubscriptions {
                sink,
                layout: self.layout.clone(),
                command_timeout: self.command_timeout,
            },
            WakeEvents { stream },
        ))
    }
}

/// Manages subscriptions on an engine's single shared pub/sub connection.
pub struct WakeSubscriptions {
    sink: PubSubSink,
    layout: Layout,
    command_timeout: Duration,
}

impl WakeSubscriptions {
    /// Subscribes to `key`'s wakes and recall channel, returning the channel name.
    ///
    /// # Errors
    ///
    /// The store's failure.
    pub async fn subscribe(&mut self, key: &str) -> Result<String, StoreError> {
        let channel = self.layout.name(key, "wakes");
        tokio::time::timeout(self.command_timeout, self.sink.subscribe(&channel))
            .await
            .map_err(|_| timeout_error())??;
        Ok(channel)
    }
}

/// Receives wakes and Delegation recalls on an engine's shared pub/sub connection.
pub struct WakeEvents {
    stream: PubSubStream,
}

impl WakeEvents {
    /// Next `(channel_name, WakeEvent)` from the store, or `None` when closed.
    pub async fn next(&mut self) -> Option<(String, WakeEvent)> {
        let message = self.stream.next().await?;
        let channel = message.get_channel_name().to_owned();
        let payload = message.get_payload::<String>().unwrap_or_default();
        let event = if let Some(holder) = payload.strip_prefix("recall:") {
            WakeEvent::Recall {
                holder: holder.to_owned(),
            }
        } else if let Some(holder) = payload.strip_prefix("delegated:") {
            WakeEvent::Delegated {
                holder: holder.to_owned(),
            }
        } else if payload == "recalled" {
            WakeEvent::Recalled
        } else {
            WakeEvent::Wake
        };
        Some((channel, event))
    }
}

/// Decodes an alternating `[field, value, ...]` `HGETALL` list into a map.
fn decode_hash_pairs(values: Vec<Vec<u8>>) -> HashMap<String, Vec<u8>> {
    let mut map = HashMap::with_capacity(values.len() / 2);
    let mut iter = values.into_iter();
    while let (Some(key), Some(val)) = (iter.next(), iter.next()) {
        map.insert(String::from_utf8_lossy(&key).into_owned(), val);
    }
    map
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
enum StoreConnection {
    One(MultiplexedConnection),
    Cluster(ClusterConnection),
}

impl ConnectionLike for StoreConnection {
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

/// The user, password, and `network.resolve` mapping the store is reached with.
struct Authentication<'a> {
    username: Option<&'a str>,
    password: Option<&'a SecretString>,
    resolve: &'a HostMapping,
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

    /// `url`'s connection, with this user, password, and `network.resolve`
    /// host override applied (ADR 0059).
    fn at(&self, url: &StoreUrl) -> Result<ConnectionInfo, StoreError> {
        let info = url.as_str().into_connection_info()?;
        let settings = self.on(info.redis_settings().clone());
        let addr = match info.addr().clone() {
            ConnectionAddr::Tcp(host, port) => {
                if let Some(addresses) = self.resolve.fixed(&host)
                    && let Some(first) = addresses.first()
                {
                    if !first.is_loopback() {
                        return Err(StoreError::Plaintext);
                    }
                    ConnectionAddr::Tcp(first.to_string(), port)
                } else {
                    ConnectionAddr::Tcp(host, port)
                }
            }
            ConnectionAddr::TcpTls {
                host,
                port,
                insecure,
                tls_params,
            } => {
                if let Some(addresses) = self.resolve.fixed(&host)
                    && let Some(first) = addresses.first()
                {
                    ConnectionAddr::TcpTls {
                        host: first.to_string(),
                        port,
                        insecure,
                        tls_params,
                    }
                } else {
                    ConnectionAddr::TcpTls {
                        host,
                        port,
                        insecure,
                        tls_params,
                    }
                }
            }
            other => other,
        };
        Ok(info.set_addr(addr).set_redis_settings(settings))
    }
}

fn timeout_error() -> StoreError {
    StoreError::Unreachable(Party {
        category: "Timeout".to_owned(),
    })
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
) -> Result<(redis::Client, StoreConnection), StoreError> {
    match addressing {
        StoreAddressing::Server { url } => {
            let client = redis::Client::open(authentication.at(url)?)?;
            let connection = client.get_multiplexed_async_connection().await?;
            Ok((client, StoreConnection::One(connection)))
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
            Ok((client, StoreConnection::One(connection)))
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
            // Every node is reached as the first: without TLS, the Cluster
            // is taken only while it announces none off loopback.
            if !nodes.first().is_some_and(StoreUrl::tls) {
                let mut seed = client.get_multiplexed_async_connection().await?;
                let slots: redis::Value = redis::cmd("CLUSTER")
                    .arg("SLOTS")
                    .query_async(&mut seed)
                    .await?;
                if announces_off_loopback(&slots) {
                    return Err(StoreError::Plaintext);
                }
            }
            let connection = cluster.get_async_connection().await?;
            Ok((client, StoreConnection::Cluster(connection)))
        }
    }
}

/// Whether `address` is reached without TLS off loopback.
fn plaintext_off_loopback(address: &ConnectionAddr) -> bool {
    matches!(address, ConnectionAddr::Tcp(host, _port) if !is_loopback(host))
}

/// Whether a `CLUSTER SLOTS` answer names a node off loopback, primary or
/// replica. A node named by no host, `?` or a wildcard address is the
/// answering node's own, or none, as redis reads it.
fn announces_off_loopback(slots: &redis::Value) -> bool {
    let redis::Value::Array(ranges) = slots else {
        return false;
    };
    ranges
        .iter()
        .filter_map(|range| match range {
            redis::Value::Array(range) => range.get(2..),
            _ => None,
        })
        .flatten()
        .filter_map(|node| match node {
            redis::Value::Array(node) => match node.first() {
                Some(redis::Value::BulkString(host)) => Some(String::from_utf8_lossy(host)),
                _ => None,
            },
            _ => None,
        })
        .any(|host| {
            let answering = host.is_empty()
                || host == "?"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_unspecified());
            !answering && !is_loopback(&host)
        })
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
    use redis::{ConnectionAddr, Value};

    use super::{Layout, announces_off_loopback, plaintext_off_loopback};

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

    /// A plaintext Cluster reaches every node it announces as it reaches
    /// its first, without TLS, so its `CLUSTER SLOTS` answer is refused
    /// when it names a node off loopback, primary or replica; a node named
    /// by no host, `?` or a wildcard is the answering node's own, which
    /// redis reads as the loopback node asked (ADR 0053).
    #[test]
    fn a_plaintext_cluster_announcing_a_node_off_loopback_is_refused() {
        let node = |host: &str| {
            Value::Array(vec![
                Value::BulkString(host.as_bytes().to_vec()),
                Value::Int(7000),
                Value::BulkString(b"id".to_vec()),
            ])
        };
        let slots = |nodes: Vec<Value>| {
            let range = [vec![Value::Int(0), Value::Int(16_383)], nodes].concat();
            Value::Array(vec![Value::Array(range)])
        };

        assert!(!announces_off_loopback(&slots(vec![
            node("127.0.0.1"),
            node("::1"),
            node("localhost"),
        ])));
        assert!(!announces_off_loopback(&slots(vec![
            node(""),
            node("?"),
            node("0.0.0.0"),
            node("::"),
        ])));
        assert!(announces_off_loopback(&slots(vec![node("10.0.0.5")])));
        assert!(announces_off_loopback(&slots(vec![
            node("127.0.0.1"),
            node("node-2.example"),
        ])));
    }
}
