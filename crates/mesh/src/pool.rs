//! Array storage for one element type, with generational IDs and change recording.
//!
//! Slots are private, and `IndexMut`, `insert_with` and `remove` are the only ways
//! to change one. All three save the old value when recording is on, so no caller
//! can make an unrecorded change.

use std::marker::PhantomData;
use std::mem::{size_of, swap};
use std::num::NonZeroU32;
use std::ops::{Index, IndexMut};

use crate::id::RawId;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Slot<T> {
    Free { next: Option<u32> },
    Occupied { generation: NonZeroU32, value: T },
}

/// State saved while recording is on.
#[derive(Debug, Clone)]
struct Journal<T> {
    /// One bit per slot that existed when recording started: already saved?
    saved: Vec<u64>,
    /// Old values of the slots changed so far.
    records: Vec<(u32, Slot<T>)>,
    old_len: u32,
    old_free_head: Option<u32>,
    old_live: u32,
}

/// Saved difference between two states of a pool. Applying it swaps the pool to
/// the other state, and swaps this value so that applying it again goes back.
#[derive(Debug, Clone)]
pub(crate) struct PoolDelta<T> {
    records: Vec<(u32, Slot<T>)>,
    other_len: u32,
    /// Slots that exist only in the longer state, while the pool is in the shorter one.
    tail: Vec<Slot<T>>,
    /// Slot count of the tail, counted in the memory use in both directions.
    tail_len: u32,
    other_free_head: Option<u32>,
    other_live: u32,
}

impl<T> PoolDelta<T> {
    pub(crate) fn is_empty(&self) -> bool {
        self.records.is_empty() && self.tail_len == 0
    }

    /// Bytes this delta costs. Stays the same when undone or redone, so the
    /// history's memory cap holds in both directions. New slots are counted even
    /// while they still live in the pool, because undo moves them here.
    pub(crate) fn memory_bytes(&self) -> usize {
        size_of::<Self>()
            + self.records.capacity() * size_of::<(u32, Slot<T>)>()
            + self.tail_len as usize * size_of::<Slot<T>>()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Pool<T, I> {
    slots: Vec<Slot<T>>,
    free_head: Option<u32>,
    live: u32,
    /// Generation for the next new element. Only ever goes up, and is not part of
    /// undo, so an ID is never issued twice. Zero means all have been used.
    next_generation: u32,
    journal: Option<Journal<T>>,
    id: PhantomData<fn() -> I>,
}

impl<T: Copy + PartialEq, I: RawId> Pool<T, I> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: None,
            live: 0,
            next_generation: 1,
            journal: None,
            id: PhantomData,
        }
    }

    pub(crate) fn live_count(&self) -> usize {
        self.live as usize
    }

    pub(crate) fn get(&self, id: I) -> Option<&T> {
        match self.slots.get(id.slot() as usize) {
            Some(Slot::Occupied { generation, value }) if *generation == id.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    pub(crate) fn contains(&self, id: I) -> bool {
        self.get(id).is_some()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| match slot {
                Slot::Occupied { generation, value } => {
                    Some((I::from_parts(i as u32, *generation), value))
                }
                Slot::Free { .. } => None,
            })
    }

    /// True if `count` more elements can be created. Edits check this up front
    /// so that running out of IDs cannot leave an edit half finished.
    pub(crate) fn can_allocate(&self, count: usize) -> bool {
        let generations_left = match self.next_generation {
            0 => 0,
            next => u64::from(u32::MAX - next) + 1,
        };
        // Each new slot uses up a generation, and there are u32::MAX generations, so
        // the highest slot ever used is u32::MAX - 1. CornerId::dangling relies on this.
        (count as u64) <= generations_left
    }

    /// Creates an element. The closure receives the new ID, for self-links.
    ///
    /// # Panics
    /// If IDs have run out. Callers check [`Self::can_allocate`] first.
    pub(crate) fn insert_with(&mut self, make: impl FnOnce(I) -> T) -> I {
        assert!(self.can_allocate(1), "element IDs exhausted");
        let generation =
            NonZeroU32::new(self.next_generation).expect("can_allocate rules out zero");
        self.next_generation = self.next_generation.wrapping_add(1);

        let index = match self.free_head {
            Some(index) => {
                self.save(index);
                let Slot::Free { next } = self.slots[index as usize] else {
                    unreachable!("free list points at an occupied slot");
                };
                self.free_head = next;
                index
            }
            None => {
                self.slots.push(Slot::Free { next: None });
                (self.slots.len() - 1) as u32
            }
        };

        let id = I::from_parts(index, generation);
        self.slots[index as usize] = Slot::Occupied {
            generation,
            value: make(id),
        };
        self.live += 1;
        id
    }

    /// # Panics
    /// If the ID is stale. Callers check first; a panic here means a bug in the mesh code.
    pub(crate) fn remove(&mut self, id: I) -> T {
        let value = *self.index(id);
        let index = id.slot();
        self.save(index);
        self.slots[index as usize] = Slot::Free {
            next: self.free_head,
        };
        self.free_head = Some(index);
        self.live -= 1;
        value
    }

    fn save(&mut self, index: u32) {
        let Some(journal) = &mut self.journal else {
            return;
        };
        // Slots added during this recording vanish on undo, so need no saving.
        if index >= journal.old_len {
            return;
        }
        let word = (index / 64) as usize;
        let bit = 1u64 << (index % 64);
        if journal.saved[word] & bit == 0 {
            journal.saved[word] |= bit;
            journal.records.push((index, self.slots[index as usize]));
        }
    }

    pub(crate) fn begin_journal(&mut self) {
        assert!(self.journal.is_none(), "a recording is already running");
        let old_len = self.slots.len() as u32;
        self.journal = Some(Journal {
            saved: vec![0; (old_len as usize).div_ceil(64)],
            records: Vec::new(),
            old_len,
            old_free_head: self.free_head,
            old_live: self.live,
        });
    }

    /// Ends recording and returns the difference, dropping saved values that
    /// ended up equal to the current ones (for example a vertex dragged back to
    /// where it started).
    pub(crate) fn finish_journal(&mut self) -> PoolDelta<T> {
        let journal = self.journal.take().expect("no recording is running");
        let mut records = journal.records;
        records.retain(|(index, old)| self.slots[*index as usize] != *old);
        records.shrink_to_fit();
        PoolDelta {
            records,
            other_len: journal.old_len,
            tail: Vec::new(),
            tail_len: self.slots.len() as u32 - journal.old_len,
            other_free_head: journal.old_free_head,
            other_live: journal.old_live,
        }
    }

    /// Ends recording and restores the state from when it started. The generation
    /// counter is left alone, so IDs from the cancelled edit stay stale forever.
    pub(crate) fn rollback_journal(&mut self) {
        let journal = self.journal.take().expect("no recording is running");
        for (index, old) in journal.records {
            self.slots[index as usize] = old;
        }
        self.slots.truncate(journal.old_len as usize);
        self.free_head = journal.old_free_head;
        self.live = journal.old_live;
    }

    /// Swaps the pool with the other state stored in `delta`.
    pub(crate) fn swap_delta(&mut self, delta: &mut PoolDelta<T>) {
        assert!(self.journal.is_none(), "cannot undo during a recording");
        for (index, slot) in &mut delta.records {
            swap(&mut self.slots[*index as usize], slot);
        }
        let current_len = self.slots.len() as u32;
        if delta.other_len < current_len {
            delta.tail = self.slots.split_off(delta.other_len as usize);
        } else if delta.other_len > current_len {
            self.slots.append(&mut delta.tail);
            delta.tail = Vec::new();
        }
        delta.other_len = current_len;
        swap(&mut self.free_head, &mut delta.other_free_head);
        swap(&mut self.live, &mut delta.other_live);
    }

    /// Bytes reserved by this pool's array.
    pub(crate) fn memory_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot<T>>()
    }

    /// Checks the free list, the live count and the generation numbers.
    pub(crate) fn check_integrity(&self, kind: &str) -> Result<(), String> {
        let len = self.slots.len();
        let mut on_free_list = vec![false; len];
        let mut cursor = self.free_head;
        while let Some(index) = cursor {
            let index = index as usize;
            match self.slots.get(index) {
                None => return Err(format!("{kind} free list points past the array: {index}")),
                Some(Slot::Occupied { .. }) => {
                    return Err(format!("{kind} free list points at occupied slot {index}"));
                }
                Some(Slot::Free { next }) => {
                    if on_free_list[index] {
                        return Err(format!("{kind} free list has a cycle at slot {index}"));
                    }
                    on_free_list[index] = true;
                    cursor = *next;
                }
            }
        }

        let mut generations = Vec::with_capacity(self.live as usize);
        for (index, slot) in self.slots.iter().enumerate() {
            match slot {
                Slot::Free { .. } if !on_free_list[index] => {
                    return Err(format!("{kind} free slot {index} is not on the free list"));
                }
                Slot::Free { .. } => {}
                Slot::Occupied { generation, .. } => {
                    let exhausted = self.next_generation == 0;
                    if !exhausted && generation.get() >= self.next_generation {
                        return Err(format!(
                            "{kind} slot {index} has generation {generation}, which has not \
                             been issued yet"
                        ));
                    }
                    generations.push(generation.get());
                }
            }
        }

        if generations.len() != self.live as usize {
            return Err(format!(
                "{kind} live count is {}, but {} slots are occupied",
                self.live,
                generations.len()
            ));
        }
        generations.sort_unstable();
        if let Some(pair) = generations.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(format!("{kind} generation {} is used twice", pair[0]));
        }
        Ok(())
    }
}

impl<T: Copy + PartialEq, I: RawId> Index<I> for Pool<T, I> {
    type Output = T;

    fn index(&self, id: I) -> &T {
        match self.get(id) {
            Some(value) => value,
            None => panic!(
                "internal mesh bug: stale ID (slot {}, generation {})",
                id.slot(),
                id.generation()
            ),
        }
    }
}

impl<T: Copy + PartialEq, I: RawId> IndexMut<I> for Pool<T, I> {
    fn index_mut(&mut self, id: I) -> &mut T {
        assert!(self.contains(id), "internal mesh bug: stale ID");
        self.save(id.slot());
        match &mut self.slots[id.slot() as usize] {
            Slot::Occupied { value, .. } => value,
            Slot::Free { .. } => unreachable!("checked by contains"),
        }
    }
}

/// Two pools are equal if they hold the same contents. Recording state and the
/// generation counter are left out because undo deliberately does not rewind them.
impl<T: PartialEq, I> PartialEq for Pool<T, I> {
    fn eq(&self, other: &Self) -> bool {
        self.slots == other.slots && self.free_head == other.free_head && self.live == other.live
    }
}

#[cfg(test)]
impl<T: Copy + PartialEq, I: RawId> Pool<T, I> {
    pub(crate) fn test_set_next_generation(&mut self, value: u32) {
        self.next_generation = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::VertexId;

    type TestPool = Pool<i32, VertexId>;

    fn assert_ok(pool: &TestPool) {
        pool.check_integrity("test").unwrap();
    }

    #[test]
    fn insert_get_remove() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 10);
        let b = pool.insert_with(|_| 20);
        assert_eq!(pool[a], 10);
        assert_eq!(pool[b], 20);
        assert_eq!(pool.live_count(), 2);
        assert_eq!(pool.remove(a), 10);
        assert_eq!(pool.get(a), None);
        assert_eq!(pool.live_count(), 1);
        assert_ok(&pool);
    }

    #[test]
    fn reused_slot_gets_a_new_generation() {
        let mut pool = TestPool::new();
        let old = pool.insert_with(|_| 1);
        pool.remove(old);
        let new = pool.insert_with(|_| 2);
        assert_eq!(old.slot(), new.slot(), "the slot should be reused");
        assert_ne!(old, new);
        assert_eq!(pool.get(old), None);
        assert_eq!(pool[new], 2);
        assert_ok(&pool);
    }

    #[test]
    fn iter_returns_only_live_elements() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        let b = pool.insert_with(|_| 2);
        let c = pool.insert_with(|_| 3);
        pool.remove(b);
        let items: Vec<_> = pool.iter().map(|(id, value)| (id, *value)).collect();
        assert_eq!(items, vec![(a, 1), (c, 3)]);
    }

    #[test]
    #[should_panic(expected = "stale ID")]
    fn index_panics_on_stale_id() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        pool.remove(a);
        let _ = pool[a];
    }

    #[test]
    #[should_panic(expected = "stale ID")]
    fn index_mut_panics_on_stale_id() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        pool.remove(a);
        pool[a] = 2;
    }

    #[test]
    fn ids_run_out_safely() {
        let mut pool = TestPool::new();
        pool.test_set_next_generation(u32::MAX);
        assert!(pool.can_allocate(1));
        assert!(!pool.can_allocate(2));
        let last = pool.insert_with(|_| 1);
        assert_eq!(last.generation().get(), u32::MAX);
        assert!(!pool.can_allocate(1));
        assert_ok(&pool);
    }

    #[test]
    fn rollback_restores_everything() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        let b = pool.insert_with(|_| 2);
        let c = pool.insert_with(|_| 3);
        pool.remove(b);
        let before = pool.clone();

        pool.begin_journal();
        pool[a] = 100;
        pool[a] = 101;
        pool.remove(c);
        let reused = pool.insert_with(|_| 4);
        let appended = pool.insert_with(|_| 5);
        let appended_2 = pool.insert_with(|_| 6);
        pool.remove(appended);
        pool.rollback_journal();

        assert_eq!(pool, before);
        assert_ok(&pool);
        assert_eq!(pool[a], 1);
        assert_eq!(pool[c], 3);
        for stale in [reused, appended, appended_2] {
            assert_eq!(pool.get(stale), None);
        }
    }

    #[test]
    fn delta_swaps_back_and_forth() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        let b = pool.insert_with(|_| 2);
        pool.remove(b);
        let before = pool.clone();

        pool.begin_journal();
        pool[a] = 50;
        let reused = pool.insert_with(|_| 7);
        let appended = pool.insert_with(|_| 8);
        let mut delta = pool.finish_journal();
        let after = pool.clone();
        let bytes = delta.memory_bytes();

        for _ in 0..3 {
            pool.swap_delta(&mut delta);
            assert_eq!(pool, before);
            assert_ok(&pool);
            assert_eq!(pool.get(reused), None);
            assert_eq!(
                delta.memory_bytes(),
                bytes,
                "cost must not change with direction"
            );

            pool.swap_delta(&mut delta);
            assert_eq!(pool, after);
            assert_ok(&pool);
            assert_eq!(pool[reused], 7);
            assert_eq!(pool[appended], 8);
            assert_eq!(
                delta.memory_bytes(),
                bytes,
                "cost must not change with direction"
            );
        }
    }

    #[test]
    fn generation_is_not_rewound_by_undo_or_rollback() {
        let mut pool = TestPool::new();
        pool.begin_journal();
        let undone = pool.insert_with(|_| 1);
        let mut delta = pool.finish_journal();
        pool.swap_delta(&mut delta);

        let after_undo = pool.insert_with(|_| 2);
        assert_eq!(undone.slot(), after_undo.slot());
        assert_ne!(
            undone, after_undo,
            "a slot reused after undo must get a new ID"
        );
        assert_eq!(pool.get(undone), None);

        pool.begin_journal();
        let rolled_back = pool.insert_with(|_| 3);
        pool.rollback_journal();
        let after_rollback = pool.insert_with(|_| 4);
        assert_eq!(rolled_back.slot(), after_rollback.slot());
        assert_ne!(rolled_back, after_rollback);
        assert_eq!(pool.get(rolled_back), None);
    }

    #[test]
    fn unchanged_values_are_dropped_from_the_delta() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        pool.begin_journal();
        pool[a] = 2;
        pool[a] = 1;
        let delta = pool.finish_journal();
        assert!(delta.is_empty());
    }

    #[test]
    fn each_slot_is_saved_once_per_recording() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        pool.begin_journal();
        for value in 2..1000 {
            pool[a] = value;
        }
        let mut delta = pool.finish_journal();
        assert_eq!(delta.records.len(), 1);
        pool.swap_delta(&mut delta);
        assert_eq!(pool[a], 1, "the first old value is what must be saved");
    }

    #[test]
    fn slots_past_a_64_boundary_record_correctly() {
        let mut pool = TestPool::new();
        let ids: Vec<_> = (0..130).map(|i| pool.insert_with(|_| i)).collect();
        let before = pool.clone();
        pool.begin_journal();
        for &id in &ids {
            pool[id] = -1;
        }
        let mut delta = pool.finish_journal();
        assert_eq!(delta.records.len(), 130);
        pool.swap_delta(&mut delta);
        assert_eq!(pool, before);
    }

    #[test]
    fn memory_bytes_counts_records_and_new_slots() {
        let mut pool = TestPool::new();
        let a = pool.insert_with(|_| 1);
        pool.begin_journal();
        pool[a] = 2;
        pool.insert_with(|_| 3);
        pool.insert_with(|_| 4);
        let delta = pool.finish_journal();
        let expected = size_of::<PoolDelta<i32>>()
            + size_of::<(u32, Slot<i32>)>()
            + 2 * size_of::<Slot<i32>>();
        assert_eq!(delta.memory_bytes(), expected);
    }

    #[test]
    fn memory_bytes_of_a_pool_is_capacity_times_slot_size() {
        let mut pool = TestPool::new();
        assert_eq!(pool.memory_bytes(), 0);
        for i in 0..100 {
            pool.insert_with(|_| i);
        }
        assert_eq!(
            pool.memory_bytes(),
            pool.slots.capacity() * size_of::<Slot<i32>>()
        );
        assert!(pool.memory_bytes() >= 100 * size_of::<Slot<i32>>());
    }

    #[test]
    fn id_counter_reports_exactly_what_is_left() {
        let mut pool = TestPool::new();
        pool.test_set_next_generation(u32::MAX - 9);
        assert!(pool.can_allocate(10));
        assert!(!pool.can_allocate(11));
        pool.test_set_next_generation(1);
        assert!(pool.can_allocate(u32::MAX as usize));
    }

    #[test]
    fn integrity_detects_corruption() {
        let healthy = || {
            let mut pool = TestPool::new();
            let a = pool.insert_with(|_| 1);
            let b = pool.insert_with(|_| 2);
            let c = pool.insert_with(|_| 3);
            let d = pool.insert_with(|_| 4);
            pool.remove(a);
            pool.remove(c);
            let _ = (b, d);
            pool.check_integrity("t").unwrap();
            pool
        };
        type Corrupt = fn(&mut TestPool);
        let cases: Vec<(&str, Corrupt)> = vec![
            ("past the array", |p| p.free_head = Some(99)),
            ("occupied slot", |p| p.free_head = Some(1)),
            ("cycle", |p| p.slots[0] = Slot::Free { next: Some(2) }),
            ("not on the free list", |p| p.free_head = Some(0)),
            ("live count", |p| p.live = 5),
            ("not been issued", |p| p.next_generation = 2),
            ("used twice", |p| {
                let Slot::Occupied { generation, .. } = p.slots[1] else {
                    panic!()
                };
                p.slots[3] = Slot::Occupied {
                    generation,
                    value: 9,
                };
            }),
        ];
        for (expected, corrupt) in cases {
            let mut pool = healthy();
            corrupt(&mut pool);
            let message = pool.check_integrity("t").expect_err(expected);
            assert!(
                message.contains(expected),
                "wanted {expected:?}, got {message:?}"
            );
        }
    }

    #[test]
    fn integrity_allows_exhausted_generation_counter() {
        let mut pool = TestPool::new();
        pool.insert_with(|_| 1);
        pool.test_set_next_generation(0);
        assert_ok(&pool);
    }
}
