//! A pure numeric slot-id pool -- tracks which of `0..total_slots` are
//! currently checked out. No I/O, no knowledge of `aivyx-kvcache` at all;
//! `LlmPlanner` (the caller) decides what a checked-out slot id is
//! actually used for.

use std::collections::HashSet;
use std::sync::Mutex;

pub struct KvSlotPool {
    total_slots: u32,
    checked_out: Mutex<HashSet<u32>>,
    /// Tracks which prefix_hash each slot id was last loaded with in
    /// THIS process (a checkout/restore, or a checkout/warm-up/save) --
    /// not cleared on release, since it describes what llama-server
    /// physically has in that slot's GPU memory right now, which
    /// outlives our own bookkeeping's "checked out" state. Lets a later
    /// checkout of the same slot for the same prefix skip a redundant
    /// (and destructive -- it overwrites live conversation KV state
    /// with the frozen prefix-only snapshot) restore-from-disk.
    last_loaded_prefix: Mutex<std::collections::HashMap<u32, String>>,
}

impl KvSlotPool {
    pub fn new(total_slots: u32) -> Self {
        Self {
            total_slots,
            checked_out: Mutex::new(HashSet::new()),
            last_loaded_prefix: Mutex::new(std::collections::HashMap::new()),
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

    /// What prefix_hash `slot_id` was last recorded as holding in this
    /// process, if any. `None` means either this slot has never been
    /// used by this process, or nothing was ever recorded for it.
    pub fn last_loaded_prefix(&self, slot_id: u32) -> Option<String> {
        self.last_loaded_prefix.lock().unwrap().get(&slot_id).cloned()
    }

    /// Records that `slot_id` now holds `prefix_hash`'s content --
    /// called after a successful restore OR a successful warm-up+save,
    /// so the next checkout of this same slot for the same prefix can
    /// skip a redundant restore.
    pub fn record_loaded_prefix(&self, slot_id: u32, prefix_hash: String) {
        self.last_loaded_prefix.lock().unwrap().insert(slot_id, prefix_hash);
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

    #[test]
    fn last_loaded_prefix_is_none_for_a_slot_never_recorded() {
        let pool = KvSlotPool::new(4);
        assert_eq!(pool.last_loaded_prefix(0), None);
    }

    #[test]
    fn record_loaded_prefix_is_retrievable_and_survives_release() {
        let pool = KvSlotPool::new(4);
        let id = pool.checkout().unwrap();
        pool.record_loaded_prefix(id, "abc123".to_string());
        assert_eq!(pool.last_loaded_prefix(id), Some("abc123".to_string()));
        pool.release(id);
        // Still recorded after release -- it describes physical GPU
        // state, not our own bookkeeping's checkout status.
        assert_eq!(pool.last_loaded_prefix(id), Some("abc123".to_string()));
    }

    #[test]
    fn record_loaded_prefix_overwrites_a_stale_entry() {
        let pool = KvSlotPool::new(4);
        let id = pool.checkout().unwrap();
        pool.record_loaded_prefix(id, "old-prefix".to_string());
        pool.record_loaded_prefix(id, "new-prefix".to_string());
        assert_eq!(pool.last_loaded_prefix(id), Some("new-prefix".to_string()));
    }
}
