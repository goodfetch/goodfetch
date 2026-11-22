//! A connection to this machine's Daemon that may refuse it by Product or
//! protocol version (ADR 0098): it waits for the pick a Daemon hands over
//! to, within the same `timeout.connect`, links its own engine in the
//! Daemon's place at [`EngineChoice::Auto`](crate::EngineChoice::Auto), and fails with the refusal
//! otherwise, its `fix` naming the install of its Product's executable
//! first. A remote Daemon's refusal fails it unchanged.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::time::Duration;

use goodfetch_collie_client::logging::CLIENT;
use goodfetch_collie_client::{ConnectTimeout, ConnectionRuntime, Error, ErrorCode};
use goodfetch_engine::Channel;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::{RefusedClient, RefusedExecutable, refusal_fix};
use goodfetch_engine::home::Home;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{
    DeadlineExceededDetails, HelloParams, NotInProductDetails, ProtocolVersionUnsupportedDetails,
    WaitedFor,
};
use goodfetch_protocol::settings::TIMEOUT_CONNECT;
use tokio::runtime::Handle;
use tokio::time::Instant;
use tokio_util::task::TaskTracker;

use super::{Asked, Composed, ProductConnection, dependencies, version};
use crate::engine_choice::{self, DaemonReach, EngineFallback};

/// How often a connection whose Daemon handed over looks for its major's
/// pick.
const HANDOVER_POLL: Duration = Duration::from_millis(50);

/// A connection opening to this machine's Daemon or a remote one, with
/// what it opens with: the runtime each engine it tries shares and that
/// runtime's handle, its `hello`, the Goodfetch home, and at
/// [`EngineChoice::Auto`](crate::EngineChoice::Auto) the environment of the linked engine it falls
/// back on.
pub(super) struct Refusable {
    pub(super) runtime: ConnectionRuntime,
    pub(super) handle: Handle,
    pub(super) engine: Composed,
    pub(super) hello: HelloParams,
    pub(super) home: Option<Home>,
    pub(super) auto: Option<BTreeMap<OsString, OsString>>,
    pub(super) asked: Asked,
    pub(super) clock: Clock,
}

impl Refusable {
    /// Opens the connection to `daemon`, or to the pick it hands over to,
    /// both within one `timeout.connect`. When this machine's Daemon
    /// refuses it by Product or protocol version and cannot hand over, it
    /// links its own engine in its place at [`EngineChoice::Auto`](crate::EngineChoice::Auto),
    /// whose `hello` waits a whole `timeout.connect` of its own,
    /// warning once on [`CLIENT`], and fails with the refusal otherwise,
    /// its `fix` naming what installs and restarts this Product's
    /// executable. A remote Daemon's refusal fails it unchanged.
    pub(super) async fn open(self, mut daemon: DaemonReach) -> Result<ProductConnection, Error> {
        let started = self.clock.now();
        loop {
            let timeout = self.rest(started)?;
            let opened = ProductConnection::open_daemon(
                self.runtime.clone(),
                self.handle.clone(),
                &daemon,
                self.home.as_ref(),
                self.hello.clone(),
                timeout,
                self.asked.listener_queue,
            )
            .await;
            let refused = match opened {
                Ok(opened) => return opened.reached(self.asked).await,
                Err(error) => past_deadline(self.asked.timeout, error),
            };
            let (Some(handed_over), Some(home)) = (daemon.pid(), self.home.as_ref()) else {
                return Err(refused);
            };
            let Some(handover) = refusal(&refused) else {
                return Err(refused);
            };
            if handover {
                daemon = self.pick_of(home, handed_over, started).await?;
                continue;
            }
            return self.instead_of(&daemon, refused).await;
        }
    }

    /// Links this Product's engine in place of `daemon`, which `refused`
    /// it, at [`EngineChoice::Auto`](crate::EngineChoice::Auto); fails with the refusal, its `fix`
    /// completed, otherwise. The linked engine's `hello` waits a whole
    /// `timeout.connect`, as it would with no Daemon.
    async fn instead_of(
        self,
        daemon: &DaemonReach,
        refused: Error,
    ) -> Result<ProductConnection, Error> {
        let product = self.engine.product;
        let given = refused.fix().unwrap_or_default();
        let fix = format!("{given} {}", linked_fix(product)).trim().to_owned();
        let Some(environment) = self.auto.clone() else {
            return Err(refused.with_fix(fix));
        };
        let dependencies = dependencies(environment)?;
        let url = daemon.url();
        let (code, message) = (refused.code(), refused.message());
        tracing::warn!(
            target: CLIENT,
            "{} runs on its linked engine instead of the Daemon at {url}, which refused it with {code}: {message} {fix}",
            product.name(),
        );
        let fallback = EngineFallback {
            cause: format!("The Daemon at {url} refused this Client with {code}: {message}"),
            fix,
        };
        ProductConnection::open(
            self.runtime,
            self.handle,
            self.engine,
            self.hello,
            dependencies,
            self.asked,
            Some(fallback),
        )
        .await
    }

    /// The Daemon of `home` that the idle one of process `handed_over`
    /// hands over to, its major's pick, once it serves. Taking the
    /// Daemon's start lock, as the CLI does, would hold a blocking thread
    /// with no deadline, since the lock has no timed wait; so, as the
    /// TypeScript Client does, the runtime file is read again every 50 ms,
    /// each read on a blocking thread of the connection's runtime.
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` naming `timeout.connect` when none serves within
    /// it, its `fix` the Daemon's log, and `daemon_unreachable` when the
    /// Daemon's files cannot be read.
    async fn pick_of(
        &self,
        home: &Home,
        handed_over: u32,
        started: Instant,
    ) -> Result<DaemonReach, Error> {
        let tasks = TaskTracker::new();
        loop {
            let read = tasks.spawn_blocking_on(
                {
                    let home = home.clone();
                    move || engine_choice::running(&home, &version())
                },
                &self.handle,
            );
            // The read ends without an answer only when the runtime shut
            // down.
            let pick = read
                .await
                .unwrap_or_else(|_| Err(Error::closed()))?
                .filter(|pick| pick.pid() != Some(handed_over));
            if let Some(pick) = pick {
                return Ok(pick);
            }
            if let Err(exceeded) = self.rest(started) {
                let fix = format!(
                    "Raise `connect`'s timeout, or find why the Daemon did not start in its log in {}.",
                    home.logs().display()
                );
                let awaited = "the Daemon an idle one handed over to served";
                return Err(self
                    .asked
                    .timeout
                    .exceeded_awaiting(awaited, fix)
                    .unwrap_or(exceeded));
            }
            // The timer is the connection's runtime's, so the wait runs on
            // any executor (ADR 0081).
            let poll = {
                let _entered = self.handle.enter();
                tokio::time::sleep(HANDOVER_POLL)
            };
            poll.await;
        }
    }

    /// What is left of `timeout.connect` since `started`.
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` naming it when nothing is.
    fn rest(&self, started: Instant) -> Result<ConnectTimeout, Error> {
        self.asked
            .timeout
            .rest(self.clock.now().duration_since(started))
    }
}

/// `error`, unless it is the `deadline_exceeded` of what was left of
/// `connect`, the program's `timeout.connect`: then `connect`'s own, as
/// the program set it.
fn past_deadline(connect: ConnectTimeout, error: Error) -> Error {
    let of_connect = error.code() == ErrorCode::DEADLINE_EXCEEDED
        && error
            .details::<DeadlineExceededDetails>()
            .is_some_and(|details| {
                details.waited_for == WaitedFor::HELLO && details.limit.key == TIMEOUT_CONNECT.name
            });
    match connect.exceeded() {
        Some(exceeded) if of_connect => exceeded,
        _ => error,
    }
}

/// Whether `error` is a Daemon's refusal of `hello` by Product or protocol
/// version, and if so whether it hands over to its major's pick; `None`
/// for any other failure.
fn refusal(error: &Error) -> Option<bool> {
    let code = error.code();
    if code == ErrorCode::PROTOCOL_VERSION_UNSUPPORTED {
        Some(
            error
                .details::<ProtocolVersionUnsupportedDetails>()
                .is_some_and(|details| details.handover),
        )
    } else if code == ErrorCode::NOT_IN_PRODUCT {
        Some(
            error
                .details::<NotInProductDetails>()
                .is_some_and(|details| details.handover),
        )
    } else {
        None
    }
}

/// What a refusal's `fix` adds for a program linking `product`'s engine,
/// as the engine words it: the install of its executable from crates.io,
/// then its `daemon restart`.
fn linked_fix(product: Product) -> String {
    refusal_fix(&RefusedClient {
        product,
        version: &version(),
        executable: RefusedExecutable::Linked,
        channel: Channel::CratesIo,
        would_be_pick: product == Product::Foxhound,
    })
}
