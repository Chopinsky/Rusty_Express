#![allow(dead_code)]

use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::support::common::cpu_relax;

const POOL_SIZE: usize = 8;
const SLOT_CAP: usize = 16;
const EXPANSION_CAP: usize = 512;
const EXPANSION_THRESHOLD: usize = 8;

struct Slot<T> {
    /// the actual data store
    slot: [Option<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: usize,

    /// if the slot is currently being read/write to
    lock: AtomicBool,
}

impl<T: Default> Slot<T> {
    fn new() -> Self {
        // create the placeholder
        let mut slice: [Option<T>; SLOT_CAP] = unsafe { MaybeUninit::zeroed().assume_init() };

        // fill the placeholder
        slice.iter_mut().for_each(|item| {
            *item = Some(Default::default());
        });

        // done
        Slot {
            slot: slice,
            len: SLOT_CAP,
            lock: AtomicBool::new(false),
        }
    }

    fn try_lock(&self, is_get: bool) -> bool {
        // be more patient if we're to return a value
        let mut count = if is_get { 4 } else { 6 };

        // check the lock and wait if not available
        while self
            .lock
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_err()
        {
            cpu_relax(2*count);
            count -= 1;

            // "timeout" -- tried 4 times and still can't get the try_lock, rare case but fine, move on.
            if count == 0 {
                return false;
            }
        }

        if (is_get && self.len == 0) || (!is_get && self.len == SLOT_CAP) {
            // not actually locked
            self.unlock();

            // read but empty, or write but full, all fail
            return false;
        }

        true
    }

    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a try_lock has
    /// been acquired previously
    fn get_one(&mut self) -> T {
        // need to loop over the slots to make sure we're getting the valid value, starting from
        for i in (0..self.len).rev() {
            if let Some(val) = self.slot[i].take() {
                // update internal states
                self.len = i;

                // return the value
                return val;
            }
        }

        Default::default()
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a try_lock has
    /// been acquired previously
    fn put_one(&mut self, val: T) {
        // need to loop over the slots to make sure we're getting the valid value
        for i in self.len..SLOT_CAP {
            if self.slot[i].is_none() {
                // update internal states
                self.slot[i].replace(val);
                self.len = i;

                // done
                return;
            }
        }

        // if all slots are full, no need to fallback, the `val` will be dropped here
        drop(val);
    }
}

struct VisitorGuard<'a>(&'a AtomicUsize);

impl<'a> VisitorGuard<'a> {
    fn register(base: &'a AtomicUsize) -> Self {
        let mut count = 0;

        // wait if the underlying storage is in protection mode
        while base.load(Ordering::Acquire) == 0 {
            cpu_relax(count + 8);

            if count < 8 {
                count += 1;
            }
        }

        base.fetch_and(1, Ordering::SeqCst);
        VisitorGuard(base)
    }
}

impl<'a> Drop for VisitorGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct SyncPool<T> {
    /// The slots storage
    slots: Vec<Slot<T>>,

    /// the next channel to try
    curr: AtomicUsize,

    /// how many threads are concurrently accessing the struct:
    ///   0   -> updating the `slots` field;
    ///   1   -> no one is using the pool;
    ///   num -> number of visitors
    visitor_count: AtomicUsize,

    /// the number of times we failed to find an in-store struct to offer
    fault_count: AtomicUsize,
}

impl<T: Default> SyncPool<T> {
    pub(crate) fn new() -> Self {
        Self::make_pool(POOL_SIZE)
    }

    pub(crate) fn with_size(size: usize) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size)
    }

    pub(crate) fn get(&mut self) -> T {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_count);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.load(Ordering::Acquire) % cap;
        let mut pos = origin;

        loop {
            // check this slot
            let slot: &mut Slot<T> = &mut self.slots[pos];
            let next = if pos == cap - 1 {
                0
            } else {
                pos + 1
            };

            // try the try_lock or move on
            if !slot.try_lock(true) {
                pos = next;

                // we've finished 1 loop but not finding a value to extract, quit
                if pos == origin {
                    break;
                }

                continue;
            }

            // now we're locked, get the val and update internal states
            self.curr.store(next, Ordering::Release);
            let val = slot.get_one();
            slot.unlock();

            // done
            return val;
        }

        // make sure our guard has been returned if we want the correct visitor count
        drop(_guard);

        if self.fault_count.fetch_add(1, Ordering::AcqRel) > EXPANSION_THRESHOLD
            && cap < EXPANSION_CAP
        {
            let mut count = 0;

            // busy waiting ... for all visitors to leave
            loop {
                match self.visitor_count.compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => break,
                    Err(old) => {
                        cpu_relax(4)
                    },
                }
            }

            // update the slots by pushing 1 more new slot
            self.slots.push(Slot::new());

            // update the internal states
            self.fault_count.store(0, Ordering::Release);
            self.visitor_count.store(1, Ordering::SeqCst);
        }

        Default::default()
    }

    pub(crate) fn put(&mut self, val: T) {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_count);

        // start from where we're left
        let cap = self.slots.len();
        let curr: usize = self.curr.load(Ordering::Acquire) % cap;

        // origin is 1 `Slots` off from the next "get" position
        let origin = if curr > 0 {
            curr - 1
        } else {
            0
        };

        let mut pos = origin;

        loop {
            // check this slot
            let slot: &mut Slot<T> = &mut self.slots[pos];
            let next = if pos == 0 {
                cap - 1
            } else {
                pos - 1
            };

            // try the try_lock or move on
            if !slot.try_lock(false) {
                pos = next;

                // we've finished 1 loop but not finding a value to extract, quit
                if pos == origin {
                    break;
                }

                continue;
            }

            // now we're locked, get the val and update internal states
            self.curr.store(pos, Ordering::Release);
            slot.put_one(val);
            slot.unlock();

            return;
        }
    }

    fn make_pool(size: usize) -> Self {
        let mut s = Vec::with_capacity(size);

        (0..size).for_each(|_| {
            // add the slice back to the vec container
            s.push(Slot::new());
        });

        SyncPool {
            slots: s,
            curr: AtomicUsize::new(0),
            visitor_count: AtomicUsize::new(1),
            fault_count: AtomicUsize::new(0),
        }
    }
}
