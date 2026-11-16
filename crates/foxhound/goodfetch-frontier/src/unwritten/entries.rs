//! The entries memory holds (#2169), each in a slot of its own: the text
//! of the crawl request it was visited for and its identity's, in one
//! allocation, its space, its state, and the place it holds in the queue
//! its state names. Each space's table and each queue keeps slot numbers,
//! a table finding an identity by its hash, so an entry costs its text,
//! its slot, a bucket of a few bytes and a place in a queue, about 95
//! bytes for a URL of 46 at the default budget.
//!
//! A place in a queue stands while its slot's state names that queue and
//! the slot holds the place's position, of which it keeps the low 32
//! bits: a queue never holds 2^32 places at once, so a place a later one
//! left behind, or one whose slot was freed and used again, never stands.
//! A slot freed while a writer job runs is used again once the next job
//! begins, so the job's journal never names another entry's slot.

use std::collections::VecDeque;
use std::hash::{BuildHasher, RandomState};

use hashbrown::HashTable;

use super::Space;

/// A slot's number.
pub(super) type Id = u32;

/// No slot: the end of the free list, or a journal's change of an entry
/// that left memory.
pub(super) const NONE: Id = Id::MAX;

/// The slots a chunk of the slab holds once it is full. A chunk never
/// moves, so the slab grows with no copy of what it holds; only the first
/// grows by doubling, so a few entries take a few slots.
const CHUNK: usize = 1024;

/// What a slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    /// No entry; `at` is the next free slot.
    Free,
    /// A chain's end held until its visit job runs, in no queue.
    Pending,
    /// An entry in no queue yet, for as long as one lock is held.
    Unplaced,
    /// An entry in the written queue.
    Written,
    /// An entry in the resident queue.
    Resident,
    /// An entry moved as memory gave back room; `at` is its new slot.
    Moved,
}

#[derive(Debug)]
struct Slot {
    /// The crawl request's text, then the identity's.
    text: Box<str>,
    /// The low 32 bits of the position of its place in its queue, or the
    /// next free slot, or the slot it moved to.
    at: u32,
    /// The length of the crawl request's text.
    visitor: u16,
    space: Space,
    state: State,
}

const _: () = assert!(size_of::<Slot>() == 2 * size_of::<usize>() + 8);

impl Slot {
    fn free(next: Id) -> Self {
        Self {
            text: Box::default(),
            at: next,
            visitor: 0,
            space: Space::Visited,
            state: State::Free,
        }
    }

    fn identity(&self) -> &str {
        &self.text[usize::from(self.visitor)..]
    }

    fn live(&self) -> bool {
        !matches!(self.state, State::Free | State::Moved)
    }
}

/// What a writer job's change found of an entry, which its failure puts
/// back.
#[derive(Clone, Copy, Debug)]
pub(super) enum Before {
    /// The job added it, or it was a chain's pending end: it leaves.
    Absent,
    /// It held a place in the queue the state names, at that position.
    Placed(State, u32),
}

/// The slots, in chunks.
#[derive(Debug)]
struct Slab {
    chunks: Vec<Vec<Slot>>,
    /// The slots made so far.
    used: usize,
    /// The first free slot, or [`NONE`].
    free: Id,
    /// The slots freed since the running writer job began.
    freed: Vec<Id>,
    /// The slots holding an entry.
    live: usize,
    /// The bytes the entries' texts allocate.
    texts: usize,
    hasher: RandomState,
}

impl Slab {
    fn slot(&self, id: Id) -> Option<&Slot> {
        let index = usize::try_from(id).ok()?;
        self.chunks.get(index / CHUNK)?.get(index % CHUNK)
    }

    fn get(&self, id: Id) -> &Slot {
        self.slot(id)
            .expect("invariant: an entry's slot is in the slab")
    }

    fn get_mut(&mut self, id: Id) -> &mut Slot {
        let index = usize::try_from(id).expect("invariant: a slot's number is a usize");
        self.chunks
            .get_mut(index / CHUNK)
            .and_then(|chunk| chunk.get_mut(index % CHUNK))
            .expect("invariant: an entry's slot is in the slab")
    }

    fn hash(&self, identity: &str) -> u64 {
        self.hasher.hash_one(identity)
    }

    fn hash_of(&self, id: Id) -> u64 {
        self.hash(self.get(id).identity())
    }

    /// Puts `slot` in a free slot, or a new one; answers its number.
    fn put(&mut self, slot: Slot) -> Id {
        self.live += 1;
        self.texts += slot.text.len();
        if self.free != NONE {
            let id = self.free;
            let free = self.get_mut(id);
            let next = free.at;
            *free = slot;
            self.free = next;
            return id;
        }
        let id = Id::try_from(self.used)
            .ok()
            .filter(|&id| id != NONE)
            .expect("invariant: memory holds fewer than 2^32 entries");
        if self.chunks.last().is_none_or(|chunk| chunk.len() == CHUNK) {
            let first = self.chunks.is_empty();
            self.chunks
                .push(Vec::with_capacity(if first { 4 } else { CHUNK }));
        }
        let chunk = self
            .chunks
            .last_mut()
            .expect("invariant: the slab has a chunk with room");
        if chunk.len() == chunk.capacity() {
            chunk.reserve_exact((2 * chunk.len()).clamp(4, CHUNK) - chunk.len());
        }
        chunk.push(slot);
        self.used += 1;
        id
    }

    /// Empties slot `id`, used again once the next writer job begins.
    fn take(&mut self, id: Id) {
        let slot = std::mem::replace(self.get_mut(id), Slot::free(NONE));
        self.live -= 1;
        self.texts -= slot.text.len();
        self.freed.push(id);
    }

    /// Frees the slots freed while the job that ran before held them.
    fn reuse(&mut self) {
        let mut freed = std::mem::take(&mut self.freed);
        for id in freed.drain(..) {
            self.get_mut(id).at = self.free;
            self.free = id;
        }
        self.freed = freed;
    }

    /// The slots the chunks have room for.
    fn room(&self) -> usize {
        self.chunks.iter().map(Vec::capacity).sum()
    }

    /// The slots the chunks would keep holding `live` entries: [`ample`]
    /// of them in the first chunk, and whole chunks past it.
    fn least(live: usize) -> (usize, usize) {
        let slots = ample(live);
        let chunks = slots.div_ceil(CHUNK);
        (if chunks > 1 { chunks * CHUNK } else { slots }, chunks)
    }
}

/// A queue of places, each told by its position: the head's, then one
/// more for each place behind it.
#[derive(Debug)]
struct Queue<T> {
    places: VecDeque<T>,
    head: u64,
}

impl<T> Queue<T> {
    fn new() -> Self {
        Self {
            places: VecDeque::new(),
            // Room below for the places read as the crawl opens, which
            // go in front.
            head: 1 << 63,
        }
    }

    /// The place at the position whose low 32 bits are `at`, if it is
    /// in the queue.
    fn at(&self, at: u32) -> Option<&T> {
        self.places
            .get(usize::try_from(at.wrapping_sub(low(self.head))).ok()?)
    }

    /// The position of the next place behind the others.
    fn back(&self) -> u64 {
        self.head + u64::try_from(self.places.len()).unwrap_or(u64::MAX)
    }
}

/// The low 32 bits of a place's position, which its slot keeps.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a place is told by its position's low 32 bits"
)]
const fn low(position: u64) -> u32 {
    position as u32
}

/// The entries memory holds, each space's table finding them by
/// identity, and the queues of those the writer wrote, in the order it
/// did, and of those the file holds, the first to leave first.
#[derive(Debug)]
pub(super) struct Entries {
    slab: Slab,
    visited: HashTable<Id>,
    /// Each learned identity with the rule that took its last parameter
    /// away.
    learned: HashTable<(Id, i64)>,
    seeds: HashTable<Id>,
    /// Each entry a writer job wrote, with the commits acknowledged as it
    /// did.
    written: Queue<(u64, Id)>,
    resident: Queue<Id>,
}

impl Entries {
    pub(super) fn new() -> Self {
        Self {
            slab: Slab {
                chunks: Vec::new(),
                used: 0,
                free: NONE,
                freed: Vec::new(),
                live: 0,
                texts: 0,
                hasher: RandomState::new(),
            },
            visited: HashTable::new(),
            learned: HashTable::new(),
            seeds: HashTable::new(),
            written: Queue::new(),
            resident: Queue::new(),
        }
    }

    /// The entry held for `identity` in `space`.
    pub(super) fn find(&self, space: Space, identity: &str) -> Option<Id> {
        let hash = self.slab.hash(identity);
        let slab = &self.slab;
        let is = |id: Id| slab.get(id).identity() == identity;
        match space {
            Space::Visited => self.visited.find(hash, |&id| is(id)).copied(),
            Space::Learned => self
                .learned
                .find(hash, |&(id, _)| is(id))
                .map(|&(id, _)| id),
            Space::Seed => self.seeds.find(hash, |&id| is(id)).copied(),
        }
    }

    /// The rule, crawl request and state of the learned identity
    /// `identity`.
    pub(super) fn learned(&self, identity: &str) -> Option<(i64, &str, State)> {
        let hash = self.slab.hash(identity);
        let &(id, rule) = self
            .learned
            .find(hash, |&(id, _)| self.slab.get(id).identity() == identity)?;
        Some((rule, self.visitor(id), self.state(id)))
    }

    /// Each learned identity held, with its state.
    pub(super) fn every_learned(&self) -> impl Iterator<Item = (Id, &str, State)> {
        self.learned.iter().map(|&(id, _)| {
            let slot = self.slab.get(id);
            (id, slot.identity(), slot.state)
        })
    }

    /// The crawl request entry `id` was visited for; empty for a seed's
    /// measure.
    pub(super) fn visitor(&self, id: Id) -> &str {
        let slot = self.slab.get(id);
        &slot.text[..usize::from(slot.visitor)]
    }

    pub(super) fn state(&self, id: Id) -> State {
        self.slab.get(id).state
    }

    /// Holds `identity` in `space`, with `rule` for a learned identity,
    /// as visited for `crawl_request`, in `state`, which names no queue.
    pub(super) fn insert(
        &mut self,
        space: Space,
        (identity, rule): (&str, i64),
        crawl_request: &str,
        state: State,
    ) -> Id {
        let visitor = u16::try_from(crawl_request.len())
            .expect("invariant: a crawl request's text is shorter than 64 KiB");
        let mut text = String::with_capacity(crawl_request.len() + identity.len());
        text.push_str(crawl_request);
        text.push_str(identity);
        let id = self.slab.put(Slot {
            text: text.into_boxed_str(),
            at: 0,
            visitor,
            space,
            state,
        });
        let hash = self.slab.hash(identity);
        let slab = &self.slab;
        match space {
            Space::Visited => {
                self.visited.insert_unique(hash, id, |&id| slab.hash_of(id));
            }
            Space::Learned => {
                self.learned
                    .insert_unique(hash, (id, rule), |&(id, _)| slab.hash_of(id));
            }
            Space::Seed => {
                self.seeds.insert_unique(hash, id, |&id| slab.hash_of(id));
            }
        }
        id
    }

    /// Takes entry `id` out of memory, if it is held.
    pub(super) fn remove(&mut self, id: Id) {
        self.take_out(id);
    }

    /// Takes entry `id` out of memory, if it is held: answers its space.
    fn take_out(&mut self, id: Id) -> Option<Space> {
        let slot = self.slab.slot(id).filter(|slot| slot.live())?;
        let (space, hash) = (slot.space, self.slab.hash(slot.identity()));
        let found = match space {
            Space::Visited => self
                .visited
                .find_entry(hash, |&held| held == id)
                .map(|entry| entry.remove().0)
                .is_ok(),
            Space::Learned => self
                .learned
                .find_entry(hash, |&(held, _)| held == id)
                .map(|entry| entry.remove().0.0)
                .is_ok(),
            Space::Seed => self
                .seeds
                .find_entry(hash, |&held| held == id)
                .map(|entry| entry.remove().0)
                .is_ok(),
        };
        assert!(found, "invariant: a held entry is in its space's table");
        self.slab.take(id);
        Some(space)
    }

    /// Makes a chain's pending end an entry in no queue yet.
    pub(super) fn unplace(&mut self, id: Id) {
        self.slab.get_mut(id).state = State::Unplaced;
    }

    /// Where entry `id` is, which a writer job's failure puts back.
    pub(super) fn before(&self, id: Id) -> Before {
        let slot = self.slab.get(id);
        match slot.state {
            State::Written | State::Resident => Before::Placed(slot.state, slot.at),
            State::Free | State::Pending | State::Unplaced | State::Moved => Before::Absent,
        }
    }

    /// Puts entry `id` back in the place `before` names, if it is held,
    /// or behind the others the file holds when that place left its
    /// queue: the file holds it by then.
    pub(super) fn restore(&mut self, id: Id, before: Before) {
        let Before::Placed(state, at) = before else {
            return;
        };
        if !self.slab.slot(id).is_some_and(Slot::live) {
            return;
        }
        let stands = match state {
            State::Written => self.written.at(at).is_some_and(|&(_, held)| held == id),
            State::Resident => self.resident.at(at).is_some_and(|&held| held == id),
            State::Free | State::Pending | State::Unplaced | State::Moved => false,
        };
        if stands {
            let slot = self.slab.get_mut(id);
            slot.state = state;
            slot.at = at;
        } else {
            self.push_resident(id);
        }
    }

    /// Queues entry `id` as written under `tag`.
    pub(super) fn push_written(&mut self, id: Id, tag: u64) {
        let at = low(self.written.back());
        self.written.places.push_back((tag, id));
        let slot = self.slab.get_mut(id);
        slot.state = State::Written;
        slot.at = at;
    }

    /// Takes back the last entry queued as written.
    pub(super) fn pop_written(&mut self) {
        self.written.places.pop_back();
    }

    /// Queues entry `id` behind the others the file holds.
    pub(super) fn push_resident(&mut self, id: Id) {
        let at = low(self.resident.back());
        self.resident.places.push_back(id);
        let slot = self.slab.get_mut(id);
        slot.state = State::Resident;
        slot.at = at;
    }

    /// Queues entry `id` before the others the file holds.
    pub(super) fn push_resident_front(&mut self, id: Id) {
        self.resident.head -= 1;
        self.resident.places.push_front(id);
        let slot = self.slab.get_mut(id);
        slot.state = State::Resident;
        slot.at = low(self.resident.head);
    }

    /// Whether the place at `position` stands for entry `id` in `state`.
    fn stands(&self, id: Id, state: State, position: u64) -> bool {
        self.slab
            .slot(id)
            .is_some_and(|slot| slot.state == state && slot.at == low(position))
    }

    /// Moves each entry written under a tag below `committed`, which the
    /// file holds, behind those it holds.
    pub(super) fn prune(&mut self, committed: u64) {
        while let Some(&(tag, id)) = self.written.places.front()
            && tag < committed
        {
            self.written.places.pop_front();
            let position = self.written.head;
            self.written.head += 1;
            if self.stands(id, State::Written, position) {
                self.push_resident(id);
            }
        }
    }

    /// Takes the oldest entry the file holds out of memory, with the
    /// places before its own that no longer stand: answers its space, or
    /// none when memory holds none the file holds.
    pub(super) fn evict(&mut self) -> Option<Space> {
        loop {
            let id = self.resident.places.pop_front()?;
            let position = self.resident.head;
            self.resident.head += 1;
            if self.stands(id, State::Resident, position) {
                return self.take_out(id);
            }
        }
    }

    /// Frees the slots freed while the writer job that ran before held
    /// them, as the next begins.
    pub(super) fn reuse(&mut self) {
        self.slab.reuse();
    }

    /// The bytes the entries' texts allocate.
    pub(super) fn texts(&self) -> u64 {
        u64::try_from(self.slab.texts).unwrap_or(u64::MAX)
    }

    /// The bytes the slab, tables and queues holding the entries take, by
    /// the room each has ([`table`]).
    pub(super) fn containers(&self) -> u64 {
        let slab = self.slab.room() * size_of::<Slot>()
            + self.slab.chunks.capacity() * size_of::<Vec<Slot>>()
            + self.slab.freed.capacity() * size_of::<Id>();
        let bytes = slab
            + table::<Id>(self.visited.capacity())
            + table::<(Id, i64)>(self.learned.capacity())
            + table::<Id>(self.seeds.capacity())
            + self.written.places.capacity() * size_of::<(u64, Id)>()
            + self.resident.places.capacity() * size_of::<Id>();
        u64::try_from(bytes).unwrap_or(u64::MAX)
    }

    /// The bytes they would take keeping only the room they need
    /// ([`ample`]), with `queued` more in the resident queue.
    pub(super) fn least(&self, queued: usize) -> u64 {
        let (slots, chunks) = Slab::least(self.slab.live);
        let bytes = slots * size_of::<Slot>()
            + chunks * size_of::<Vec<Slot>>()
            + table::<Id>(room(ample(self.visited.len())))
            + table::<(Id, i64)>(room(ample(self.learned.len())))
            + table::<Id>(room(ample(self.seeds.len())))
            + ample(self.written.places.len()) * size_of::<(u64, Id)>()
            + ample(self.resident.places.len() + queued) * size_of::<Id>();
        u64::try_from(bytes).unwrap_or(u64::MAX)
    }

    /// Gives back the room the slab, tables and queues keep past what
    /// they need ([`ample`]), moving entries to fill the slots freed, and
    /// the slot of each entry `journal` names with them.
    pub(super) fn shrink(&mut self, journal: &mut [(Id, Before)]) {
        if self.slab.room() > Slab::least(self.slab.live).0 {
            self.compact(journal);
        }
        let slab = &self.slab;
        self.visited
            .shrink_to(ample(self.visited.len()), |&id| slab.hash_of(id));
        self.learned
            .shrink_to(ample(self.learned.len()), |&(id, _)| slab.hash_of(id));
        self.seeds
            .shrink_to(ample(self.seeds.len()), |&id| slab.hash_of(id));
        self.written
            .places
            .shrink_to(ample(self.written.places.len()));
        self.resident
            .places
            .shrink_to(ample(self.resident.places.len()));
    }

    /// Moves the last entries into the free slots below them, until the
    /// first `live` slots hold every entry, then gives back the slots
    /// past them.
    fn compact(&mut self, journal: &mut [(Id, Before)]) {
        // A change of an entry that left memory changes nothing, and its
        // slot may now take another entry.
        for (id, _) in journal.iter_mut() {
            if !self.slab.slot(*id).is_some_and(Slot::live) {
                *id = NONE;
            }
        }
        let number =
            |index: usize| Id::try_from(index).expect("invariant: a slot's number is an Id");
        let (mut low_end, mut high_end) = (0, self.slab.used);
        loop {
            while low_end < high_end && self.slab.get(number(low_end)).live() {
                low_end += 1;
            }
            while high_end > low_end && !self.slab.get(number(high_end - 1)).live() {
                high_end -= 1;
            }
            if low_end >= high_end {
                break;
            }
            let (from, to) = (number(high_end - 1), number(low_end));
            let moved = std::mem::replace(
                self.slab.get_mut(from),
                Slot {
                    state: State::Moved,
                    ..Slot::free(to)
                },
            );
            let (space, hash) = (moved.space, self.slab.hash(moved.identity()));
            *self.slab.get_mut(to) = moved;
            let found = match space {
                Space::Visited => self
                    .visited
                    .find_mut(hash, |&id| id == from)
                    .map(|id| *id = to),
                Space::Learned => self
                    .learned
                    .find_mut(hash, |&(id, _)| id == from)
                    .map(|(id, _)| *id = to),
                Space::Seed => self
                    .seeds
                    .find_mut(hash, |&id| id == from)
                    .map(|id| *id = to),
            };
            found.expect("invariant: a held entry is in its space's table");
        }
        let slab = &self.slab;
        let forward = |id: &mut Id| {
            if let Some(slot) = slab.slot(*id)
                && slot.state == State::Moved
            {
                *id = slot.at;
            }
        };
        self.written
            .places
            .iter_mut()
            .for_each(|(_, id)| forward(id));
        self.resident.places.iter_mut().for_each(forward);
        for (id, _) in journal.iter_mut() {
            forward(id);
        }
        let live = self.slab.live;
        let chunks = live.div_ceil(CHUNK);
        self.slab.chunks.truncate(chunks);
        self.slab.chunks.shrink_to(ample(chunks));
        if let Some(last) = self.slab.chunks.last_mut() {
            last.truncate(live - (chunks - 1) * CHUNK);
            if chunks == 1 {
                last.shrink_to(ample(live));
            }
        }
        self.slab.used = live;
        self.slab.free = NONE;
        self.slab.freed = Vec::new();
    }
}

/// The bytes a hash table with room for `capacity` entries of `T` takes,
/// as hashbrown's lays it out, std's too: for each bucket a slot and a
/// control byte, one bucket in eight kept empty past seven, and a group
/// of control bytes more. A table never gives back the room it grew to,
/// and grows past what it holds as entries leave and others come, so its
/// room, not its entries, is what it costs.
pub(super) fn table<T>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = if capacity < 8 {
        capacity + 1
    } else {
        capacity / 7 * 8
    };
    buckets * (size_of::<T>() + 1) + CONTROL_GROUP
}

/// The control bytes a hash table holds past its buckets.
const CONTROL_GROUP: usize = 16;

/// The room a container holding `len` entries keeps once it gives back
/// what it does not need: an eighth more, and one, so the next entries
/// fill it before it grows. Without it a full memory would give back its
/// room at one lock and grow again at the next admission, rebuilding a
/// table each time; with it, at most once in an eighth of what it holds.
pub(super) fn ample(len: usize) -> usize {
    if len == 0 { 0 } else { len + len / 8 + 1 }
}

/// The room a hash table asked for room for `len` entries keeps, as
/// hashbrown sizes it: a power of two of buckets, one in eight kept empty
/// past eight.
fn room(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let buckets = match len {
        1..4 => 4,
        4..8 => 8,
        _ => (len.saturating_mul(8) / 7).next_power_of_two(),
    };
    if buckets <= 8 {
        buckets - 1
    } else {
        buckets / 8 * 7
    }
}
