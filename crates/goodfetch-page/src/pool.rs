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
//! `governor.browser.recycle_idle`. Recycled, it opens no new page: each of
//! its Sessions takes its next page in another browser, launched for it
//! when none has room, carrying its cookies and `localStorage` there (ADR
//! 0040: a Session is not its browser); the pages a Session left behind
//! stay open where they are, and the recycled browser closes once none is
//! left, taking along the cookies and storage of a Session that had not
//! moved yet. A browser holding a page in a Human pause, under a person in
//! the Live view or under a raw CDP attach is recycled only once that page
//! is released. A Borrowed browser never enters the pool, so it is neither
//! recycled nor counted.
#![expect(
    clippy::disallowed_types,
    reason = "the pool's lock guards its browsers for one change at a time, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

mod rules;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as SyncMutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::browser_cache::Pins;
use goodfetch_engine::governor::footprint_of;
use goodfetch_engine::home::Home;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::RunSettings;
use tokio::sync::{Mutex, OnceCell};

use crate::driver::{Carried, LocalBrowser, Origins, PageDriver, SessionDriver};
use crate::trace::CommandLog;
pub(crate) use rules::Placement;
use rules::{Limits, Seats, Use};

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
    /// What it used once its first page had run.
    warm_up: Option<u64>,
    /// Where it is in its life.
    stage: Stage,
    /// Counts each time it went from no open page to one, so an idle wait
    /// started before knows it was outlived.
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
        Self {
            shared: Arc::new(Shared {
                run,
                home,
                pins,
                limits,
                log: Arc::default(),
                browsers: SyncMutex::default(),
            }),
        }
    }

    /// A seat for a Session, placed under `placement` on its first use.
    pub(crate) fn seat(&self, placement: Placement) -> Seat {
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
                origins: Origins::default(),
                place: Mutex::new(Place::default()),
            }),
        }
    }

    /// Seats `seat` in a browser under `placement`: the first of the Run
    /// with room, or one launched for it.
    fn place(&self, seat: &Arc<SeatShared>) -> (u64, Arc<LocalBrowser>) {
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
                    )),
                    own: seat.placement == Placement::OwnProcess,
                    seated: BTreeMap::new(),
                    served: 0,
                    open: BTreeMap::new(),
                    held: 0,
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
        self.settle(&mut browsers, number);
        (number, browser)
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
    /// open or held page it closes, and with none it waits to be idle.
    fn settle(&self, browsers: &mut Browsers, number: u64) {
        let Some(pooled) = browsers.list.get_mut(&number) else {
            return;
        };
        if pooled.stage == Stage::Closing || pooled.open_pages() > 0 || pooled.held > 0 {
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

    /// Holds the page's browser from being recycled until the value drops:
    /// a Human pause, a person in the Live view, or a raw CDP attach bound
    /// to its work item's lease, on the page.
    #[expect(
        dead_code,
        reason = "the Human pause, the Live view and raw CDP attach hold a page once their tickets land (#978)"
    )]
    pub(crate) fn hold(&self) -> Hold {
        let mut browsers = lock(&self.pool.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&self.number) {
            pooled.held += 1;
        }
        Hold {
            pool: self.pool.clone(),
            number: self.number,
        }
    }
}

impl Drop for InBrowser {
    fn drop(&mut self) {
        self.pool.page_closed(self.number, self.seat);
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
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut browsers = lock(&self.pool.shared.browsers);
        if let Some(pooled) = browsers.list.get_mut(&self.number) {
            pooled.held -= 1;
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
    origins: Origins,
    place: Mutex<Place>,
}

/// The Session's driver in one browser.
type Driver = Arc<SessionDriver<Arc<LocalBrowser>>>;

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

    /// Opens a page of the Session, `navigation_timeout` its navigations'
    /// deadline, in its browser unless that one is recycled, in which case
    /// the Session moves first.
    pub(crate) async fn new_page(
        &self,
        navigation_timeout: Duration,
    ) -> Result<(PageDriver, InBrowser), CallError> {
        let mut place = self.shared.place.lock().await;
        let (number, driver) = loop {
            let (number, driver) = self.shared.seated(&mut place).await?;
            if !self.shared.pool.recycled(number).await {
                break (number, driver);
            }
            self.shared.leave(&mut place).await;
        };
        let in_browser = self.shared.pool.page_opening(number, self.shared.number);
        let page = driver.new_page(navigation_timeout).await?;
        let events = page.events().await;
        self.shared
            .pool
            .shared
            .run
            .spawn(self.shared.origins.clone().watch(events));
        Ok((page, in_browser))
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
        let mut place = self.shared.place.lock().await;
        if let Some((number, driver)) = place.at.take() {
            driver.close().await;
            self.shared.pool.leave(number, self.shared.number);
        }
        for (_, driver) in std::mem::take(&mut place.left) {
            driver.close().await;
        }
        place.carried = Carried::default();
    }
}

impl SeatShared {
    /// The browser the Session is seated in and its driver there, seating
    /// it with what it carries when it is in none.
    async fn seated(self: &Arc<Self>, place: &mut Place) -> Result<(u64, Driver), CallError> {
        if let Some((number, driver)) = &place.at {
            return Ok((*number, Arc::clone(driver)));
        }
        let (number, browser) = self.pool.place(self);
        let driver = Arc::new(SessionDriver::new(browser));
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
        Ok((number, driver))
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
