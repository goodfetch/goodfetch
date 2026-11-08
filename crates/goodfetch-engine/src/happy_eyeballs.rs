//! The Happy Eyeballs race the engine's HTTP transports run to connect to a
//! host (ADR 0118; RFC 8305): its IPv6 and IPv4 answers are awaited
//! together, an IPv4 answer that comes first waits the resolution delay for
//! IPv6, and the addresses are tried one at a time, alternating families
//! from IPv6, each next attempt starting after the attempt delay or at once
//! when one fails, until one connects. A Session that holds a local address
//! of one family only tries that family (ADR 0044). The race is a pure
//! function of the `Clock`, the answers and the attempts it is given: what
//! an attempt is, a socket or a test's stand-in, is its caller's.

use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::{Pin, pin};
use std::task::Poll;
use std::time::Duration;

use goodfetch_protocol::dns::{
    ADDRESS_FAMILY_UNAVAILABLE, ATTEMPT_DELAY, AddressFamilyUnavailableDetails, IpFamily,
    RESOLUTION_DELAY,
};

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::dns::{LookupError, family_of};
use crate::settings::{Resolved, catalog_default};

#[cfg(test)]
mod tests;

/// The race's two delays: `resolver.resolution_delay` and
/// `resolver.attempt_delay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delays {
    /// How long an IPv4 answer that came first waits for IPv6's.
    pub resolution: Duration,
    /// How long an attempt runs before the next starts beside it.
    pub attempt: Duration,
}

impl Delays {
    /// The delays `settings` give.
    pub fn of(settings: &Resolved) -> Self {
        Self {
            resolution: settings.get(&RESOLUTION_DELAY),
            attempt: settings.get(&ATTEMPT_DELAY),
        }
    }
}

/// The catalog's defaults, RFC 8305's: 50 ms and 250 ms.
impl Default for Delays {
    fn default() -> Self {
        Self {
            resolution: catalog_default(&RESOLUTION_DELAY),
            attempt: catalog_default(&ATTEMPT_DELAY),
        }
    }
}

/// The IP families a Session may connect from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Families {
    /// Both: it holds no local address, or one of each family.
    Both,
    /// Only this one: it holds a local address of this family alone.
    Only(IpFamily),
}

impl Families {
    fn hold(self, family: IpFamily) -> bool {
        match self {
            Self::Both => true,
            Self::Only(held) => held == family,
        }
    }
}

/// How the race runs.
#[derive(Clone, Copy, Debug)]
pub struct Race {
    /// What it waits on.
    pub clock: Clock,
    /// Its delays.
    pub delays: Delays,
    /// The families it may try.
    pub families: Families,
}

/// The attempt that connected first.
#[derive(Debug)]
pub struct Won<C> {
    /// What it connected.
    pub connection: C,
    /// The address it connected to.
    pub address: SocketAddr,
}

/// Why a race connected nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum RaceError {
    /// No family the Session holds gave an address: the lookup's error, a
    /// failure of the name servers over a name with no address.
    Lookup(LookupError),
    /// The host has addresses only in the family the Session lacks.
    FamilyUnavailable(FamilyUnavailable),
    /// Every address tried failed to connect, in the order tried.
    Unreachable {
        /// The addresses tried.
        tried: Vec<SocketAddr>,
    },
}

/// A host with no address in the one family a Session holds
/// (`address_family_unavailable`, ADR 0044).
#[derive(Debug, PartialEq, Eq)]
pub struct FamilyUnavailable {
    /// The host's name.
    pub name: String,
    /// The family the Session holds.
    pub held: IpFamily,
}

impl From<FamilyUnavailable> for CallError {
    fn from(unavailable: FamilyUnavailable) -> Self {
        let (held, other) = match unavailable.held {
            IpFamily::Ipv4 => ("IPv4", "IPv6"),
            IpFamily::Ipv6 => ("IPv6", "IPv4"),
        };
        Self::new(
            &ADDRESS_FAMILY_UNAVAILABLE,
            format!(
                "{} has only {other} addresses, and the Session holds a local address of {held} alone",
                unavailable.name
            ),
            &AddressFamilyUnavailableDetails {
                name: unavailable.name,
                held: unavailable.held,
            },
        )
    }
}

type InFlight<F> = Vec<(SocketAddr, Pin<Box<F>>)>;

impl Race {
    /// Connects to `name` on `port` from its `ipv6` and `ipv4` answers,
    /// each attempt made by `attempt`. An answer of a family the Session
    /// does not hold is awaited only to tell, once no address is left,
    /// whether the host is out of its reach.
    ///
    /// # Errors
    ///
    /// [`RaceError`]: no address, none in a family the Session holds, or
    /// none that connected.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops every attempt in flight.
    pub async fn connect<C, Ipv6, Ipv4, Attempt, Attempting>(
        &self,
        name: &str,
        port: u16,
        ipv6: Ipv6,
        ipv4: Ipv4,
        mut attempt: Attempt,
    ) -> Result<Won<C>, RaceError>
    where
        Ipv6: Future<Output = Result<Vec<IpAddr>, LookupError>>,
        Ipv4: Future<Output = Result<Vec<IpAddr>, LookupError>>,
        Attempt: FnMut(SocketAddr) -> Attempting,
        Attempting: Future<Output = io::Result<C>>,
    {
        let clock = self.clock;
        let mut ipv6 = pin!(ipv6);
        let mut ipv4 = pin!(ipv4);
        let mut awaiting_ipv6 = self.families.hold(IpFamily::Ipv6);
        let mut awaiting_ipv4 = self.families.hold(IpFamily::Ipv4);
        let mut addresses = Addresses::default();
        let mut errors: Vec<LookupError> = Vec::new();
        let mut in_flight: InFlight<Attempting> = Vec::new();
        let mut tried = Vec::new();
        let mut started_any = false;
        let mut ipv6_awaited_until = None;
        let mut next_start = clock.now();
        loop {
            let now = clock.now();
            let waiting_for_ipv6 =
                awaiting_ipv6 && ipv6_awaited_until.is_some_and(|until| now < until);
            if !waiting_for_ipv6
                && now >= next_start
                && let Some(address) = addresses.next()
            {
                in_flight.push((address, Box::pin(attempt(address))));
                started_any = true;
                next_start = now + self.delays.attempt;
                continue;
            }
            if in_flight.is_empty() && addresses.is_empty() && !awaiting_ipv6 && !awaiting_ipv4 {
                break;
            }
            let wake = (!addresses.is_empty()).then_some(match ipv6_awaited_until {
                Some(until) if waiting_for_ipv6 => until,
                _ => next_start,
            });
            tokio::select! {
                biased;
                (index, connected) = finished(&mut in_flight), if !in_flight.is_empty() => {
                    let (address, _) = in_flight.swap_remove(index);
                    if let Ok(connection) = connected {
                        return Ok(Won { connection, address });
                    }
                    tried.push(address);
                    next_start = clock.now();
                }
                answer = &mut ipv6, if awaiting_ipv6 => {
                    awaiting_ipv6 = false;
                    match answer {
                        Ok(found) => addresses.add(IpFamily::Ipv6, &found, port),
                        Err(error) => errors.push(error),
                    }
                }
                answer = &mut ipv4, if awaiting_ipv4 => {
                    awaiting_ipv4 = false;
                    match answer {
                        Ok(found) => addresses.add(IpFamily::Ipv4, &found, port),
                        Err(error) => errors.push(error),
                    }
                    if awaiting_ipv6 && !started_any && !addresses.is_empty() {
                        ipv6_awaited_until = Some(clock.now() + self.delays.resolution);
                    }
                }
                () = clock.sleep_until(wake.unwrap_or(now)), if wake.is_some() => {}
            }
        }
        if !tried.is_empty() {
            return Err(RaceError::Unreachable { tried });
        }
        if let Families::Only(held) = self.families {
            let other = match held {
                IpFamily::Ipv4 => ipv6.await,
                IpFamily::Ipv6 => ipv4.await,
            };
            if other.is_ok_and(|found| !found.is_empty()) {
                return Err(RaceError::FamilyUnavailable(FamilyUnavailable {
                    name: name.to_owned(),
                    held,
                }));
            }
        }
        match errors.into_iter().reduce(LookupError::or) {
            Some(error) => Err(RaceError::Lookup(error)),
            None => Err(RaceError::Unreachable { tried }),
        }
    }
}

/// The index and outcome of the first attempt in flight to end.
async fn finished<F, C>(in_flight: &mut InFlight<F>) -> (usize, io::Result<C>)
where
    F: Future<Output = io::Result<C>>,
{
    poll_fn(|context| {
        for (index, (_, attempt)) in in_flight.iter_mut().enumerate() {
            if let Poll::Ready(outcome) = attempt.as_mut().poll(context) {
                return Poll::Ready((index, outcome));
            }
        }
        Poll::Pending
    })
    .await
}

/// The addresses not yet tried, by family, and the family whose turn is
/// next: IPv6 first, then each in turn (RFC 8305 section 4).
#[derive(Default)]
struct Addresses {
    ipv6: VecDeque<SocketAddr>,
    ipv4: VecDeque<SocketAddr>,
    ipv4_next: bool,
}

impl Addresses {
    fn add(&mut self, family: IpFamily, found: &[IpAddr], port: u16) {
        let queue = match family {
            IpFamily::Ipv6 => &mut self.ipv6,
            IpFamily::Ipv4 => &mut self.ipv4,
        };
        queue.extend(
            found
                .iter()
                .filter(|address| family_of(**address) == family)
                .map(|address| SocketAddr::new(*address, port)),
        );
    }

    fn is_empty(&self) -> bool {
        self.ipv6.is_empty() && self.ipv4.is_empty()
    }

    fn next(&mut self) -> Option<SocketAddr> {
        let (turn, other) = if self.ipv4_next {
            (&mut self.ipv4, &mut self.ipv6)
        } else {
            (&mut self.ipv6, &mut self.ipv4)
        };
        let (address, took_turn) = match turn.pop_front() {
            Some(address) => (address, true),
            None => (other.pop_front()?, false),
        };
        if took_turn {
            self.ipv4_next = !self.ipv4_next;
        }
        Some(address)
    }
}
