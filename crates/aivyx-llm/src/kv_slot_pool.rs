//! A pure numeric slot-id pool -- tracks which of `0..total_slots` are
//! currently checked out. No I/O, no knowledge of `aivyx-kvcache` at all;
//! `LlmPlanner` (the caller) decides what a checked-out slot id is
//! actually used for.

use std::collections::HashSet;
use std::sync::Mutex;

pub struct KvSlotPool {
    total_slots: u32,
    checked_out: Mutex<HashSet<u32>>,
}

impl KvSlotPool {
    pub fn new(total_slots: u32) -> Self {
        Self {
            total_slots,
            checked_out: Mutex::new(HashSet::new()),
        }
    }

    /// Returns the lowest-numbered free slot id, or `None` if every slot
    /// is already checked out.
    pub fn checkout(&self) -> Option<u32> {
        let mut checked_out = self.checked_out.lock().unwrap();
        (0..self.total_slots).find(|id| checked_out.insert(*id))
    }

    /// Returns `slot_id` to the pool. A `slot_id` that was never checked
    /// out (or already released) is a silent no-op -- release is called
    /// from `Drop` impls, where panicking or erroring is not an option.
    pub fn release(&self, slot_id: u32) {
        self.checked_out.lock().unwrap().remove(&slot_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_returns_lowest_free_id_first() {
        let pool = KvSlotPool::new(4);
        assert_eq!(pool.checkout(), Some(0));
        assert_eq!(pool.checkout(), Some(1));
    }

    #[test]
    fn checkout_returns_none_once_the_pool_is_full() {
        let pool = KvSlotPool::new(2);
        assert_eq!(pool.checkout(), Some(0));
        assert_eq!(pool.checkout(), Some(1));
        assert_eq!(pool.checkout(), None, "pool of size 2 must reject a third concurrent checkout");
    }

    #[test]
    fn release_makes_a_slot_available_again() {
        let pool = KvSlotPool::new(1);
        let id = pool.checkout().expect("pool of size 1 has a free slot");
        assert_eq!(pool.checkout(), None, "the only slot is already checked out");
        pool.release(id);
        assert_eq!(pool.checkout(), Some(id), "release must make the slot checkoutable again");
    }

    #[test]
    fn releasing_a_never_checked_out_id_is_a_silent_no_op() {
        let pool = KvSlotPool::new(4);
        pool.release(99); // never checked out -- must not panic
        assert_eq!(pool.checkout(), Some(0), "pool must still function normally after a no-op release");
    }
}
