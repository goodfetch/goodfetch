//! A Run's browser pool (ADR 0040, ADR 0037's SA-09): the browsers the
//! engine launched for one Run, never for another, and the browser
//! context each of the Run's Sessions holds in one of them. A Session takes
//! a seat in the first browser of its Run with room under
//! `governor.browser.contexts_per_process`, or in a browser launched for
//! it; one of a per-process Fingerprint method gets a browser of its own.
//!
//! A browser is recycled after `governor.browser.recycle_pages` pages, past
//! its memory cap (`governor.browser.memory_cap`, else twice what it used
//! once its first page had run), or once it held no open page for
//! `governor.browser.recycle_idle`, the time a Session spends seated in it
//! or opening a page there not counted. Recycled, it opens no new page: each of
//! its Sessions takes its next page in another browser, launched for it
//! when none has room, carrying its cookies and `localStorage` there (ADR
//! 0040: a Session is not its browser); the pages a Session left behind
//! stay open where they are, and the recycled browser closes once none is
//! left, taking along the cookies and storage of a Session that had not
//! moved yet. A browser holding a page in a Human pause, under a person in
//! the Live view or under a raw CDP attach is recycled only once that page
//! is released. A Borrowed browser never enters the pool, so it is neither
//! recycled nor counted.
//!
//! Once launched, each browser joins the engine's Resource governor, which
//! counts its memory in its Run's footprint and, while memory is critical,
//! sheds it (ADR 0040's shedding ladder): a browser it sheds is recycled,
//! so one with no open page closes at once. A Session being seated or
//! opening a page counts as an open page there, so the governor never
//! sheds a browser under a Session that is opening its first page.
#![expect(
    clippy::disallowed_types,
    reason = "the pool's lock guards its browsers for one change at a time, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

mod rules;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as SyncMutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::{
    BrowserChoice, BrowserSource, Pins, Platform, browser_source,
};
use goodfetch_engine::dns::{Dns, HostMapping, ResolverConfiguration};
use goodfetch_engine::governor::{BrowserUse, GovernedBrowser, PageHold, footprint_of};
use goodfetch_engine::home::Home;
use goodfetch_engine::local_addresses::{self, Rotation, Unavailable};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::tls::{CertificateExceptions, PinBook, Trust, trusted_cas};
use goodfetch_protocol::tls::{TLS_ACCEPT_INVALID_CERTS, TLS_TRUSTED_CAS};
use tokio::sync::{Mutex, OnceCell};

use crate::borrowed::CdpUrl;
use crate::driver::{
    Behind, BorrowedBrowser, Carried, LocalBrowser, Origins, PageDriver, SessionDriver,
    SessionEgress, Visibility,
};
use crate::egress_relay::application_settings::ApplicationSettings;
use crate::egress_relay::authority::Authority;
use crate::egress_relay::{Local, RelayTls};
use crate::trace::CommandLog;
pub(crate) use rules::Placement;
use rules::{Limits, Seats, Use};

/// Why a Session's local addresses cannot be held.
#[derive(Debug)]
pub(crate) enum LocalFailure {
    /// `egress.local_addresses` gives no address now.
    Unavailable(Unavailable),
    /// The system's resolver configuration cannot be read.
    Unreadable(String),
}

/// A Run's browser pool. Clones share it.
#[derive(Clone)]
pub(crate) struct Pool {
    shared: Arc<Shared>,
}

struct Shared {
    run: Run,
    home: Home,
    pins: Pins,
    limits: Limits,
    /// The egress relay's certificate authority, when the Run lists an
    /// invalid-certificate exception: every browser of the Run trusts it,
    /// and every Session's relay signs with it (ADR 0042).
    authority: Option<Arc<Authority>>,
    /// The Run's exceptions, which every Session's relay shares, so a host
    /// stopped as a changed key stays stopped for the whole Run.
    exceptions: CertificateExceptions,
    /// The authorities the Run's `tls.trusted_cas` trusts, read once.
    trust: OnceCell<Trust>,
    /// The Run's `egress.local_addresses`, which each Session draws its
    /// addresses from (ADR 0044).
    rotation: Option<Arc<Rotation>>,
    /// The engine's resolution its Sessions with local addresses resolve
    /// through, read from the system's configuration once.
    dns: OnceCell<Result<Dns, String>>,
    /// The Run trace's CDP log, which its browsers share.
    log: Arc<OnceCell<CommandLog>>,
    browsers: SyncMutex<Browsers>,
}

#[derive(Default)]
struct Browsers {
    /// Each browser by its number, in launch order.
    list: BTreeMap<u64, Pooled>,
    /// The highest number a browser took.
    last: u64,
    /// The highest number a seat took.
    seats: u64,
}

/// A browser of the pool.
struct Pooled {
    browser: Arc<LocalBrowser>,
    /// Whether it is a Session's own process.
    own: bool,
    /// The seats in it, by their number.
    seated: BTreeMap<u64, Weak<SeatShared>>,
    /// How many pages it opened.
    served: u64,
    /// Its open pages, by the number of the seat that opened each.
    open: BTreeMap<u64, usize>,
    /// How many of its pages are held.
    held: usize,
    /// How many of its held pages a person controls in the Live view.
    persons: usize,
    /// Its place in the Resource governor, once launched.
    governed: Option<GovernedBrowser>,
    /// How many Sessions are being seated in it or opening a page there,
    /// which keeps it from being idle or closing.
    opening: usize,
    /// What it used once its first page had run.
    warm_up: Option<u64>,
    /// Where it is in its life.
    stage: Stage,
    /// Counts each time it went from no open or opening page to one, so an
    /// idle wait started before knows it was outlived.
    busy: u64,
    /// Whether an idle wait is on.
    waiting: bool,
}

/// Where a browser of the pool is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// It opens pages.
    Serving,
    /// It is recycled: it opens no new page, and closes once none is open.
    Retiring,
    /// It closes.
    Closing,
}

impl Pooled {
    fn open_pages(&self) -> usize {
        self.open.values().sum()
    }

    /// Reports its use to the Resource governor, which sheds it at
    /// critical unless it is held.
    fn report(&self) {
        let Some(governed) = &self.governed else {
            return;
        };
        let held = if self.persons > 0 {
            PageHold::Person
        } else if self.held > 0 {
            PageHold::Held
        } else {
            PageHold::None
        };
        governed.report(BrowserUse {
            open_pages: self.open_pages() + self.opening,
            held,
        });
    }
}

/// The ALPS payload of the build the Run's browsers launch, captured when
/// it was pinned: none for an installed browser, which no capture names.
fn application_settings_of(pins: &Pins) -> ApplicationSettings {
    match browser_source(
        Platform::current(),
        &BrowserChoice::Default,
        &BTreeMap::new(),
    ) {
        BrowserSource::Pinned { name, platform, .. } => pins
            .browser(name, platform)
            .map(|pinned| pinned.application_settings.clone())
            .unwrap_or_default(),
        BrowserSource::Installed { .. } => ApplicationSettings::new(),
    }
}

/// Locks `browsers`, whose state no panic leaves half-written.
fn lock(browsers: &SyncMutex<Browsers>) -> MutexGuard<'_, Browsers> {
    browsers.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pool {
    /// The browser pool of `run`, whose browsers come from the Browser
    /// cache of `home` and the builds `pins` names.
    pub(crate) fn new(run: Run, home: Home, pins: Pins) -> Self {
        let limits = Limits::of(run.settings().map(RunSettings::resolved));
        let exceptions = exceptions_of(&run);
        let authority = authority_of(&run, &exceptions);
        let rotation = run
            .settings()
            .and_then(|settings| Rotation::of(settings.resolved()))
            .map(Arc::new);
        Self {
            shared: Arc::new(Shared {
                run,
                home,
                pins,
                limits,
                authority,
                exceptions,
                trust: OnceCell::new(),
                rotation,
                dns: OnceCell::new(),
                log: Arc::default(),
                browsers: SyncMutex::default(),
            }),
        }
    }

    /// What the Sessions' egress relays take of the Run's TLS settings:
    /// its authority, when it lists an exception, its exceptions and the
    /// authorities it trusts.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the trust's files to the next call.
    pub(crate) async fn relay_tls(&self) -> RelayTls {
        let shared = &self.shared;
        let trust = shared
            .trust
            .get_or_init(|| async {
                let setting = shared
                    .run
                    .settings()
                    .map(|settings| settings.resolved().value(TLS_TRUSTED_CAS.name).clone())
                    .unwrap_or_default();
                trusted_cas(&setting, shared.run.owner()).await
            })
            .await
            .clone();
        RelayTls {
            authority: shared.authority.clone(),
            exceptions: shared.exceptions.clone(),
            trust,
            application_settings: Arc::new(application_settings_of(&shared.pins)),
        }
    }

    /// The local addresses the Session at `position` of the Run holds for
    /// its life, and the engine's resolution its relay resolves through
    /// from them; none when the Run sets no `egress.local_addresses`. The
    /// same position draws the same addresses again, as a Session reopened
    /// from run state does.
    ///
    /// # Errors
    ///
    /// [`LocalFailure`]: the addresses give none now, such as an interface
    /// that lost its addresses, or the system's resolver configuration
    /// cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves a started read of the host to finish.
    pub(crate) async fn local(&self, position: u32) -> Result<Option<Local>, LocalFailure> {
        let shared = &self.shared;
        let Some(rotation) = &shared.rotation else {
            return Ok(None);
        };
        let rotation = Arc::clone(rotation);
        let ordinal = u64::from(position.saturating_sub(1));
        let addresses = shared
            .run
            .owner()
            .blocking(move || rotation.session(&local_addresses::System, ordinal))
            .await
            .map_err(LocalFailure::Unavailable)?;
        let clock = shared.run.clock();
        let dns = shared
            .dns
            .get_or_init(|| async {
                shared
                    .run
                    .owner()
                    .blocking(ResolverConfiguration::system)
                    .await
                    .map(|configuration| Dns::new(HostMapping::default(), configuration, clock))
                    .map_err(|unreadable| unreadable.to_string())
            })
            .await
            .clone()
            .map_err(LocalFailure::Unreadable)?;
        Ok(Some(Local { addresses, dns }))
    }

    /// A seat for a Session whose requests leave through `egress`, its
    /// proxy and its egress relay, in every browser it is seated in, placed
    /// under `placement` on its first use.
    pub(crate) fn seat(&self, placement: Placement, egress: SessionEgress) -> Seat {
        self.seat_of(placement, egress, None)
    }

    /// A seat for a Session on the Borrowed browser at `cdp_url`, which
    /// never enters the pool: it is neither recycled, nor counted, nor
    /// closed (ADR 0056).
    pub(crate) fn borrowed_seat(&self, cdp_url: CdpUrl) -> Seat {
        let browser = BorrowedBrowser::new(
            self.shared.run.clone(),
            cdp_url,
            Arc::clone(&self.shared.log),
        );
        self.seat_of(
            Placement::Shared,
            SessionEgress::default(),
            Some(Arc::new(browser)),
        )
    }

    fn seat_of(
        &self,
        placement: Placement,
        egress: SessionEgress,
        borrowed: Option<Arc<BorrowedBrowser>>,
    ) -> Seat {
        let number = {
            let mut browsers = lock(&self.shared.browsers);
            browsers.seats += 1;
            browsers.seats
        };
        Seat {
            shared: Arc::new(SeatShared {
                pool: self.clone(),
                number,
                placement,
                egress,
                borrowed,
                origins: Origins::default(),
                place: Mutex::new(Place::default()),
            }),
        }
    }

    /// Seats `seat` in a browser under `placement`: the first of the Run
    /// with room, or one launched for it, which is not idle until the
    /// returned [`Opening`] drops.
    fn place(&self, seat: &Arc<SeatShared>) -> (u64, Arc<LocalBrowser>, Opening) {
        let mut browsers = lock(&self.shared.browsers);
        let found = rules::place(
            browsers.list.iter().map(|(number, pooled)| {
                (
                    *number,
                    Seats {
                        contexts: u64::try_from(pooled.seated.len()).unwrap_or(u64::MAX),
                        retiring: pooled.stage != Stage::Serving,
                        own: pooled.own,
                    },
                )
            }),
            seat.placement,
            &self.shared.limits,
        );
        let number = found.unwrap_or_else(|| {
            browsers.last += 1;
            let number = browsers.last;
            browsers.list.insert(
                number,
                Pooled {
                    browser: Arc::new(LocalBrowser::new(
                        self.shared.run.clone(),
                        self.shared.home.clone(),
                        self.shared.pins.clone(),
                        number,
                        Arc::clone(&self.shared.log),
                        self.shared.authority.clone(),
                    )),
                    own: seat.placement == Placement::OwnProcess,
                    seated: BTreeMap::new(),
                    served: 0,
                    open: BTreeMap::new(),
                    held: 0,
                    persons: 0,
                    governed: None,
                    opening: 0,
                    warm_up: None,
                    stage: Stage::Serving,
                    busy: 0,
                    waiting: false,
                },
            );
            number
        });
        let pooled = browsers
            .list
            .get_mut(&number)
            .expect("invariant: the browser was just found or made");
        pooled.seated.insert(seat.number, Arc::downgrade(seat));
        let browser = Arc::clone(&pooled.browser);
        let opening = self.opening_in(&mut browsers, number);
        self.settle(&mut browsers, number);
        (number, browser, opening)
    }

    /// A Session is seated in the browser `number` or opens a page there:
    /// the browser is not idle, nor does it close, until the value drops.
    fn opening(&self, number: u64) -> Opening {
        self.opening_in(&mut lock(&self.shared.browsers), number)
    }

    fn opening_in(&self, browsers: &mut Browsers, number: u64) -> Opening {
        if let Some(pooled) = browsers.list.get_mut(&number) {
            if pooled.open_pages() == 0 && pooled.opening == 0 {
                pooled.busy += 1;
            }
            pooled.opening += 1;
            pooled.report();
        }
        Opening {
            pool: self.clone(),
            number,
        }
    }

    /// Whether the browser `number` is recycled now: it was already, or it
    /// reached its pages or passed its memory cap, measured now.
    async fn recycled(&self, number: u64) -> bool {
        let (process, served) = {
            let browsers = lock(&self.shared.browsers);
            let Some(pooled) = browsers.list.get(&number) else {
                return true;
            };
            if pooled.stage != Stage::Serving {
                return true;
            }
            (pooled.browser.process(), pooled.served)
        };
        let footprint = match process {
            Some(process) if served > 0 => Some(
                self.shared
                    .run
                    .owner()
                    .blocking(move || footprint_of(process))
                    .await,
            ),
            _ => None,
        };
        let mut browsers = lock(&self.shared.browsers);
        let Some(pooled) = browsers.list.get_mut(&number) else {
            return true;
        };
        if pooled.warm_up.is_none() && served > 0 {
            pooled.warm_up = footprint;
        }
        let browser = Use {
            served: pooled.served,
            footprint,
            warm_up: pooled.warm_up,
            held: pooled.held > 0,
        };
        let Some(why) = rules::recycle(&browser, &self.shared.limits) else {
            return pooled.stage != Stage::Serving;
        };
        tracing::info!(
            browser = number,
            ?why,
            ?footprint,
            "a browser of the Run is recycled"
        );
        self.shared.run.metrics().browser_recycled(why.as_str());
        pooled.stage = Stage::Retiring;
        self.settle(&mut browsers, number);
        true
    }

    /// A page of the seat `seat` opens in the browser `number`.
    fn page_opening(&self, number: u64, seat: u64) -> InBrowser {
        let mut browsers = lock(&self.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&number) {
            if pooled.open_pages() == 0 {
                pooled.busy += 1;
            }
            pooled.served += 1;
            *pooled.open.entry(seat).or_default() += 1;
            pooled.report();
        }
        InBrowser {
            pool: self.clone(),
            number,
            seat,
        }
    }

    /// A page of the seat `seat` in the browser `number` closed.
    fn page_closed(&self, number: u64, seat: u64) {
        let mut browsers = lock(&self.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&number)
            && let Some(open) = pooled.open.get_mut(&seat)
        {
            *open -= 1;
            if *open == 0 {
                pooled.open.remove(&seat);
            }
        }
        self.settle(&mut browsers, number);
    }

    /// Whether the seat `seat` has an open page in the browser `number`.
    fn has_pages(&self, number: u64, seat: u64) -> bool {
        lock(&self.shared.browsers)
            .list
            .get(&number)
            .is_some_and(|pooled| pooled.open.contains_key(&seat))
    }

    /// The seat `seat` left the browser `number`.
    fn leave(&self, number: u64, seat: u64) {
        let mut browsers = lock(&self.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&number) {
            pooled.seated.remove(&seat);
        }
        self.settle(&mut browsers, number);
    }

    /// Acts on what changed in the browser `number`: once recycled with no
    /// open, held or opening page it closes, and with none it waits to be
    /// idle.
    fn settle(&self, browsers: &mut Browsers, number: u64) {
        let Some(pooled) = browsers.list.get_mut(&number) else {
            return;
        };
        pooled.report();
        if pooled.stage == Stage::Closing
            || pooled.open_pages() > 0
            || pooled.held > 0
            || pooled.opening > 0
        {
            return;
        }
        if pooled.stage == Stage::Retiring {
            pooled.stage = Stage::Closing;
            let pool = self.clone();
            self.shared
                .run
                .spawn(async move { pool.close(number).await });
        } else if !pooled.waiting {
            pooled.waiting = true;
            let (pool, busy) = (self.clone(), pooled.busy);
            let stopping = self.shared.run.cancellation().clone();
            let (clock, idle) = (self.shared.run.clock(), self.shared.limits.idle);
            self.shared.run.spawn(async move {
                tokio::select! {
                    () = stopping.cancelled() => {}
                    () = clock.sleep(idle) => pool.idle(number, busy, idle),
                }
            });
        }
    }

    /// Has the Resource governor count and shed the browser `number`, once
    /// launched: shed, it is recycled (ADR 0040).
    fn govern(&self, number: u64) {
        let mut browsers = lock(&self.shared.browsers);
        let Some(pooled) = browsers.list.get_mut(&number) else {
            return;
        };
        if pooled.governed.is_some() {
            return;
        }
        let Some(process) = pooled.browser.process() else {
            return;
        };
        let governed = self.shared.run.governor().browser(process);
        let shed = governed.shed();
        pooled.governed = Some(governed);
        pooled.report();
        let pool = self.clone();
        let stopping = self.shared.run.cancellation().clone();
        self.shared.run.spawn(async move {
            tokio::select! {
                () = stopping.cancelled() => {}
                () = shed => pool.shed(number),
            }
        });
    }

    /// The Resource governor shed the browser `number` while memory is
    /// critical: it is recycled, and closes at once with no open page.
    fn shed(&self, number: u64) {
        let mut browsers = lock(&self.shared.browsers);
        let Some(pooled) = browsers.list.get_mut(&number) else {
            return;
        };
        if pooled.stage == Stage::Serving {
            tracing::info!(
                browser = number,
                "memory is critical: the governor recycles a browser of the Run"
            );
            self.shared.run.metrics().browser_recycled("shed");
            pooled.stage = Stage::Retiring;
        }
        self.settle(&mut browsers, number);
    }

    /// The browser `number` held no open page for `idle` unless it was
    /// busy again since `busy`: it is recycled.
    fn idle(&self, number: u64, busy: u64, idle: Duration) {
        let mut browsers = lock(&self.shared.browsers);
        let Some(pooled) = browsers.list.get_mut(&number) else {
            return;
        };
        pooled.waiting = false;
        if pooled.busy == busy && pooled.open_pages() == 0 && pooled.stage == Stage::Serving {
            tracing::info!(
                browser = number,
                ?idle,
                "an idle browser of the Run is recycled"
            );
            self.shared.run.metrics().browser_recycled("idle");
            pooled.stage = Stage::Retiring;
        }
        self.settle(&mut browsers, number);
    }

    /// Closes the browser `number`, each Session still seated in it taking
    /// its cookies and storage along for the browser it goes to next.
    async fn close(&self, number: u64) {
        let seated: Vec<Arc<SeatShared>> = lock(&self.shared.browsers)
            .list
            .get(&number)
            .map(|pooled| pooled.seated.values().filter_map(Weak::upgrade).collect())
            .unwrap_or_default();
        for seat in seated {
            seat.evacuate(number).await;
        }
        if let Some(pooled) = lock(&self.shared.browsers).list.remove(&number) {
            pooled.browser.close();
        }
    }
}

/// A page open in a browser of the pool, until this value drops.
pub(crate) struct InBrowser {
    pool: Pool,
    number: u64,
    seat: u64,
}

impl InBrowser {
    /// The number of the page's browser in its Run's pool.
    pub(crate) fn browser(&self) -> u64 {
        self.number
    }

    /// Holds the page's browser from being recycled or shed until the
    /// value drops: a Human pause or a raw CDP attach bound to its work
    /// item's lease, held by `PageHold::Held`, or a person in the Live view,
    /// by `PageHold::Person`, whose Run the governor's last resort stops
    /// last (ADR 0040).
    #[expect(
        dead_code,
        reason = "the Human pause, the Live view and raw CDP attach hold a page once their tickets land (#978)"
    )]
    pub(crate) fn hold(&self, by: PageHold) -> Hold {
        let mut browsers = lock(&self.pool.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&self.number) {
            pooled.held += 1;
            if by == PageHold::Person {
                pooled.persons += 1;
            }
            pooled.report();
        }
        Hold {
            pool: self.pool.clone(),
            number: self.number,
            by,
        }
    }
}

impl Drop for InBrowser {
    fn drop(&mut self) {
        self.pool.page_closed(self.number, self.seat);
    }
}

/// A Session being seated in a browser of the pool or opening a page there,
/// until this value drops.
struct Opening {
    pool: Pool,
    number: u64,
}

impl Drop for Opening {
    fn drop(&mut self) {
        let mut browsers = lock(&self.pool.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&self.number) {
            pooled.opening -= 1;
        }
        self.pool.settle(&mut browsers, self.number);
    }
}

/// A held page's hold on its browser.
#[expect(
    dead_code,
    reason = "the Human pause, the Live view and raw CDP attach hold a page once their tickets land (#978)"
)]
pub(crate) struct Hold {
    pool: Pool,
    number: u64,
    by: PageHold,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut browsers = lock(&self.pool.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&self.number) {
            pooled.held -= 1;
            if self.by == PageHold::Person {
                pooled.persons -= 1;
            }
        }
        self.pool.settle(&mut browsers, self.number);
    }
}

/// A Session's seat in its Run's browser pool: the browser context it
/// holds now, in which browser, and what it carries between browsers.
pub(crate) struct Seat {
    shared: Arc<SeatShared>,
}

struct SeatShared {
    pool: Pool,
    number: u64,
    placement: Placement,
    /// The Session's one proxy and its egress relay, which it keeps in
    /// every browser (SA-22).
    egress: SessionEgress,
    /// The Borrowed browser it is seated in for good, outside the pool.
    borrowed: Option<Arc<BorrowedBrowser>>,
    origins: Origins,
    place: Mutex<Place>,
}

/// The Session's driver in one browser.
type Driver = Arc<SessionDriver<Behind>>;

/// The number a Borrowed browser goes by, which no browser of the pool
/// takes: the pool counts nothing under it.
const BORROWED: u64 = 0;

#[derive(Default)]
struct Place {
    /// The browser it is seated in, by number, and its driver there.
    at: Option<(u64, Driver)>,
    /// What it takes to the next browser it is seated in.
    carried: Carried,
    /// Its browser contexts in recycled browsers, which still hold pages it
    /// opened there.
    left: Vec<(u64, Driver)>,
}

impl Seat {
    /// Makes the Session's browser context, launching a browser first when
    /// none of the Run has room: a missing build or a browser that does
    /// not start raises its code here.
    pub(crate) async fn open(&self) -> Result<(), CallError> {
        let mut place = self.shared.place.lock().await;
        self.shared.seated(&mut place).await.map(drop)
    }

    /// Opens a page of the Session in its browser, of `visibility`,
    /// unless that one is recycled, in which case the Session moves first.
    pub(crate) async fn new_page(
        &self,
        visibility: Visibility,
    ) -> Result<(PageDriver, InBrowser), CallError> {
        let mut place = self.shared.place.lock().await;
        let (number, driver, opening) = loop {
            let (number, driver, opening) = self.shared.seated(&mut place).await?;
            if self.shared.borrowed.is_some() || !self.shared.pool.recycled(number).await {
                break (number, driver, opening);
            }
            drop(opening);
            self.shared.leave(&mut place).await;
        };
        let in_browser = self.shared.pool.page_opening(number, self.shared.number);
        drop(opening);
        let page = driver.new_page(visibility).await?;
        let events = page.events();
        self.shared
            .pool
            .shared
            .run
            .spawn(self.shared.origins.clone().watch(events));
        Ok((page, in_browser))
    }

    /// Takes in a popup, `popup`, that a page of the Session opened in the
    /// browser `number`, its opener's: it counts there as a page the
    /// Session opened, and its documents' origins are carried as theirs.
    pub(crate) fn popup_in(&self, number: u64, popup: &PageDriver) -> InBrowser {
        let in_browser = self.shared.pool.page_opening(number, self.shared.number);
        let events = popup.events();
        self.shared
            .pool
            .shared
            .run
            .spawn(self.shared.origins.clone().watch(events));
        in_browser
    }

    /// Whether the Session is seated in the browser `number` now.
    pub(crate) async fn is_in(&self, number: u64) -> bool {
        self.shared
            .place
            .lock()
            .await
            .at
            .as_ref()
            .is_some_and(|(at, _)| *at == number)
    }

    /// Closes the Session's browser contexts, and with them every page in
    /// them; its browsers stay, for the Run's other Sessions.
    pub(crate) async fn close(&self) {
        // Its contexts are taken out of its place under the lock, which is
        // let go before they close (STYLE.md: no lock across an `.await`).
        let (at, left) = {
            let mut place = self.shared.place.lock().await;
            place.carried = Carried::default();
            (place.at.take(), std::mem::take(&mut place.left))
        };
        if let Some((number, driver)) = at {
            driver.close().await;
            self.shared.pool.leave(number, self.shared.number);
        }
        for (_, driver) in left {
            driver.close().await;
        }
    }
}

impl SeatShared {
    /// The browser the Session is seated in and its driver there, seating
    /// it with what it carries when it is in none; the browser is not idle
    /// while the [`Opening`] lives.
    async fn seated(
        self: &Arc<Self>,
        place: &mut Place,
    ) -> Result<(u64, Driver, Opening), CallError> {
        if let Some((number, driver)) = &place.at {
            return Ok((*number, Arc::clone(driver), self.pool.opening(*number)));
        }
        if let Some(borrowed) = &self.borrowed {
            let driver = Arc::new(SessionDriver::new(
                Behind::Borrowed(Arc::clone(borrowed)),
                SessionEgress::default(),
            ));
            driver.open().await?;
            place.at = Some((BORROWED, Arc::clone(&driver)));
            return Ok((BORROWED, driver, self.pool.opening(BORROWED)));
        }
        let (number, browser, opening) = self.pool.place(self);
        let driver = Arc::new(SessionDriver::new(
            Behind::Launched(browser),
            self.egress.clone(),
        ));
        let opened = match driver.open().await {
            Ok(()) => driver.restore(&place.carried).await,
            Err(error) => Err(error),
        };
        if let Err(error) = opened {
            self.pool.leave(number, self.number);
            return Err(error);
        }
        place.carried = Carried::default();
        place.at = Some((number, Arc::clone(&driver)));
        self.pool.govern(number);
        Ok((number, driver, opening))
    }

    /// The Session leaves its browser, recycled, taking its cookies and
    /// storage along; its context stays while it holds pages it opened
    /// there.
    async fn leave(&self, place: &mut Place) {
        let Some((number, driver)) = place.at.take() else {
            return;
        };
        place.carried = match driver.capture(&self.origins).await {
            Ok(carried) => carried,
            Err(error) => {
                tracing::warn!(%error, "a Session's cookies and storage are not carried over");
                Carried::default()
            }
        };
        if self.pool.has_pages(number, self.number) {
            place.left.push((number, driver));
        } else {
            driver.close().await;
        }
        self.pool.leave(number, self.number);
    }

    /// The browser `number` closes: the Session leaves it if seated there,
    /// and its contexts left there go with it.
    async fn evacuate(&self, number: u64) {
        let mut place = self.place.lock().await;
        place.left.retain(|(left, _)| *left != number);
        if place.at.as_ref().is_some_and(|(at, _)| *at == number) {
            self.leave(&mut place).await;
        }
    }
}

/// The egress relay's certificate authority of `run`, made in memory now,
/// constrained to the domains its `tls.accept_invalid_certs` and
/// `tls.trusted_cas` list, its key's
/// hash and its domains recorded in the Run's Provenance: none when it
/// lists none. A Run whose authority cannot be made has none: its
/// listed hosts are tunnelled, and the browser refuses their certificates.
fn authority_of(run: &Run, exceptions: &CertificateExceptions) -> Option<Arc<Authority>> {
    let mut domains = exceptions.domains();
    let trusted = run
        .settings()
        .map(|settings| settings.resolved().value(TLS_TRUSTED_CAS.name))
        .and_then(serde_json::Value::as_object);
    for domain in trusted.into_iter().flat_map(serde_json::Map::keys) {
        let domain = domain.trim_end_matches('.').to_ascii_lowercase();
        if !domains.contains(&domain) {
            domains.push(domain);
        }
    }
    if domains.is_empty() {
        return None;
    }
    match Authority::new(domains.clone()) {
        Ok(authority) => {
            // Its key's hash alone is recorded: neither its key nor its
            // certificate is written anywhere.
            let entry = serde_json::json!({
                "action": "relay_authority",
                "spki_sha256": authority.spki_sha256(),
                "domains": domains,
            });
            let recorded = run.clone();
            run.spawn(async move { crate::trace::provenance(&recorded, &entry).await });
            Some(Arc::new(authority))
        }
        Err(error) => {
            tracing::warn!(%error, "the egress relay has no certificate authority, so no host is terminated");
            None
        }
    }
}

/// The invalid-certificate exceptions of `run`'s `tls.accept_invalid_certs`,
/// pinning in the person's known-certificates store; none, logged, when an
/// entry names no host.
fn exceptions_of(run: &Run) -> CertificateExceptions {
    let setting = run
        .settings()
        .map(|settings| {
            settings
                .resolved()
                .value(TLS_ACCEPT_INVALID_CERTS.name)
                .clone()
        })
        .unwrap_or_default();
    let book = PinBook {
        store: run.known_certificates(),
        person: run.person().to_owned(),
        run_id: Some(run.id().clone()),
        audit: Arc::new(run.audit_log()),
        owner: Arc::new(run.owner().child()),
        clock: run.clock(),
    };
    CertificateExceptions::listed(&setting, book, run.secrets().exception_hosts())
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "`tls.accept_invalid_certs` names no host, so it makes no exception");
            CertificateExceptions::default()
        })
}
