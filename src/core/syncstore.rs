#![allow(dead_code)]

use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
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
    fn new(fill: bool) -> Self {
        // create the placeholder
        let mut slice: [Option<T>; SLOT_CAP] = unsafe { MaybeUninit::zeroed().assume_init() };

        // fill the placeholder if required
        if fill {
            slice.iter_mut().for_each(|item| {
                *item = Some(Default::default());
            });
        }

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
            cpu_relax(2 * count);
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
    fn register(base: &'a (AtomicUsize, AtomicBool)) -> Self {
        let mut count = 0;

        // wait if the underlying storage is in protection mode
        while base.1.load(Ordering::Acquire) {
            cpu_relax(count + 8);

            if count < 8 {
                count += 1;
            }
        }

        base.0.fetch_add(1, Ordering::SeqCst);
        VisitorGuard(&base.0)
    }
}

impl<'a> Drop for VisitorGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) struct SyncPool<T> {
    /// The slots storage
    slots: Vec<Slot<T>>,

    /// the next channel to try
    curr: AtomicUsize,

    /// First node -- how many threads are concurrently accessing the struct:
    ///   0   -> updating the `slots` field;
    ///   1   -> no one is using the pool;
    ///   num -> number of visitors
    /// Second node -- write barrier:
    ///   true  -> write barrier has been raised
    ///   false -> no write barrier
    visitor_counter: (AtomicUsize, AtomicBool),

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
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.load(Ordering::Acquire) % cap;
        let mut pos = origin;

        loop {
            // check this slot
            let slot: &mut Slot<T> = &mut self.slots[pos];
            let next = if pos == cap - 1 { 0 } else { pos + 1 };

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
            && self.expand(1, false)
        {
            self.fault_count.store(0, Ordering::Release);
        }

        Default::default()
    }

    pub(crate) fn put(&mut self, val: T) {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let curr: usize = self.curr.load(Ordering::Acquire) % cap;

        // origin is 1 `Slots` off from the next "get" position
        let origin = if curr > 0 { curr - 1 } else { 0 };

        let mut pos = origin;

        loop {
            // check this slot
            let slot: &mut Slot<T> = &mut self.slots[pos];
            let next = if pos == 0 { cap - 1 } else { pos - 1 };

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

    pub(crate) fn expand(&mut self, additional: usize, block: bool) -> bool {
        // raise the write barrier now, if someone has already raised the flag to indicate the
        // intention to write, let me go away.
        if self
            .visitor_counter
            .1
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }

        // busy waiting ... for all visitors to leave
        let mut count: usize = 0;
        let safe = loop {
            match self
                .visitor_counter
                .0
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break true,
                Err(old) => {
                    cpu_relax(2);
                    count += 1;

                    if count > 8 && !block {
                        break false;
                    }
                }
            }
        };

        if safe {
            // update the slots by pushing `additional` slots
            (0..additional).for_each(|_| {
                self.slots.push(Slot::new(true));
            });

            // update the internal states
            self.visitor_counter.0.store(1, Ordering::SeqCst);
            self.visitor_counter.1.store(false, Ordering::Release);
        }

        safe
    }

    fn make_pool(size: usize) -> Self {
        let mut s = Vec::with_capacity(size);

        (0..size).for_each(|_| {
            // add the slice back to the vec container
            s.push(Slot::new(true));
        });

        SyncPool {
            slots: s,
            curr: AtomicUsize::new(0),
            visitor_counter: (AtomicUsize::new(1), AtomicBool::new(false)),
            fault_count: AtomicUsize::new(0),
        }
    }
}

impl<T> Drop for SyncPool<T> {
    fn drop(&mut self) {
        self.slots.clear();
    }
}

pub(crate) trait Reusable {
    fn obtain() -> Box<Self>;
    fn release(self: Box<Self>);
    fn reset(&mut self, hard: bool);
}

/// The inner storage wrapper struct
pub(crate) struct StaticStore<T>(Option<T>);

/// The struct that will hold the actual pool. The implementation is sound because all usage is internal
/// and we're guaranteed that before each call, the real values are actually set ahead.
impl<T> StaticStore<T> {
    pub(crate) const fn init() -> Self {
        StaticStore(None)
    }

    pub(crate) fn set(&mut self, val: T) {
        self.0.replace(val);
    }

    pub(crate) fn as_mut(&mut self) -> Result<&mut T, ErrorKind> {
        self.0.as_mut().ok_or(ErrorKind::NotFound)
    }

    pub(crate) fn as_ref(&self) -> Result<&T, ErrorKind> {
        self.0.as_ref().ok_or(ErrorKind::NotFound)
    }
}

impl<T> Deref for StaticStore<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("Calling methods on uninitialized struct is forbidden...")
    }
}

impl<T> DerefMut for StaticStore<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("Calling methods on uninitialized struct is forbidden...")
    }
}

impl<T> Drop for StaticStore<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            // drop the content now
            drop(inner);
        }
    }
}
