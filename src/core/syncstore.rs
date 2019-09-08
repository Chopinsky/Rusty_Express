#![allow(unused)]

use std::io::ErrorKind;
use std::mem::{self, MaybeUninit};
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::thread;

use crate::support::common::cpu_relax;

const POOL_SIZE: usize = 16;
const SLOT_CAP: usize = 8;
const GET_MASK: u16 = 0b1010_1010_1010_1010;
const PUT_MASK: u16 = 0b1111_1111_1111_1111;

pub(crate) const TOTAL_ELEM_COUNT: usize = POOL_SIZE * SLOT_CAP;

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
        let mut count = if is_get { 2 } else { 4 };

        // check the lock and wait if not available
        while self
            .lock
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_err()
        {
            // "timeout" -- tried 4 times and still can't get the try_lock, rare case but fine, move on.
            count -= 1;
            if count == 0 {
                return false;
            }

            cpu_relax(2 * count);
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
    fn checkout(&mut self) -> T {
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
    fn release(&mut self, val: T) {
        // need to loop over the slots to make sure we're getting the valid value
        for i in self.len..SLOT_CAP {
            if self.slot[i].is_none() {
                // update internal states
                self.slot[i].replace(val);
                self.len = i + 1;

                // done
                return;
            }
        }

        // if all slots are full, no need to fallback, the `val` will be dropped here
        drop(val);
    }
}

pub(crate) struct Bucket<T> {
    /// the actual data store
    slot: [*mut T; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: AtomicUsize,

    /// The bitmap containing metadata information of the underlying slots
    bitmap: AtomicU16,
}

impl<T: Default> Bucket<T> {
    pub(crate) fn new(fill: bool) -> Self {
        // create the placeholder
        let mut slice: [*mut T; SLOT_CAP] = [ptr::null_mut(); SLOT_CAP];
        let mut bitmap: u16 = 0;

        // fill the slots and update the bitmap
        if fill {
            for (i, item) in slice.iter_mut().enumerate() {
                *item = Box::into_raw(Box::new(Default::default()));
                bitmap |= 1 << (2 * i as u16);
            }
        }

        // done
        Bucket {
            slot: slice,
            len: AtomicUsize::new(SLOT_CAP),
            bitmap: AtomicU16::new(bitmap),
        }
    }

    pub(crate) fn size_hint(&self) -> usize {
        //        println!("{:#018b}", self.bitmap.load(Ordering::Acquire));
        self.len.load(Ordering::Acquire) % (SLOT_CAP + 1)
    }

    pub(crate) fn access(&self, get: bool) -> Result<usize, ()> {
        // pre-checkout, make sure the len is in post-action state so it can reject future attempts
        // if it's unlikely to succeed in this slot.
        let curr_len = if get {
            self.len.fetch_sub(1, Ordering::AcqRel)
        } else {
            self.len.fetch_add(1, Ordering::AcqRel)
        };

        // oops, last op blew off the roof, back off mate. Note that (0 - 1 == MAX_USIZE) for stack
        // overflow, still way off the roof and a proof of not doing well.
        if curr_len > SLOT_CAP || (get && curr_len == 0) {
            return self.access_failure(get);
        }

        let mut trials: usize = 2;
        while trials > 0 {
            trials -= 1;

            // init try
            let (pos, mask) = match self.enter(get) {
                Ok(pos) => (pos, 0b10 << (2 * pos)),
                Err(()) => continue,
            };

            // main loop to try to update the bitmap
            let old = self.bitmap.fetch_or(mask, Ordering::AcqRel);

            // if the lock bit we replaced was not yet marked at the atomic op, we're good
            if old & mask == 0 {
                return Ok(pos as usize);
            }

            // otherwise, try again after some wait. The earliest registered gets some favor by
            // checking and trying to lodge a position more frequently than the later ones.
            cpu_relax(trials + 1);
        }

        self.access_failure(get)
    }

    pub(crate) fn leave(&self, pos: u16) {
        // the lock bit we want to toggle
        let lock_bit = 0b10 << (2 * pos);

        loop {
            // update both lock bit and the slot bit
            let old = self.bitmap.fetch_xor(0b11 << (2 * pos), Ordering::SeqCst);
            if old & lock_bit == lock_bit {
                return;
            }
        }
    }

    /// Locate the value from the desired position. The API will return an error if such operation
    /// can't be accomplished, such as the destination doesn't contain a value, or the desired position
    /// is OOB.
    ///
    /// The function is safe because it's used internally, and each time it's guaranteed an exclusive
    /// access has been acquired previously.
    pub(crate) fn checkout(&mut self, pos: usize) -> Result<Box<T>, ()> {
        // return the value
        if pos >= SLOT_CAP {
            return Err(());
        }

        let val = mem::replace(&mut self.slot[pos], ptr::null_mut());
        if val.is_null() {
            return Err(());
        }

        Ok(unsafe { Box::from_raw(val) })
    }

    /// Release the value back into the pool. If a reset function has been previously provided, we
    /// will call the function to reset the value before putting it back. The API will be no-op if
    /// the desired operation can't be conducted, such as if the position is OOB, or the position
    /// already contains a value.
    ///
    /// The function is safe because it's used internally, and each time it's guaranteed an exclusive
    /// access has been acquired previously
    pub(crate) fn release(&mut self, pos: usize, val: Box<T>) {
        // need to loop over the slots to make sure we're getting the valid value
        if pos >= SLOT_CAP {
            return;
        }

        // check if the slot has already been occupied (unlikely but still)
        if !self.slot[pos].is_null() {
            return;
        }

        // move the value in
        self.slot[pos] = Box::into_raw(val);
    }

    #[inline]
    fn access_failure(&self, get: bool) -> Result<usize, ()> {
        if get {
            self.len.fetch_add(1, Ordering::AcqRel);
        } else {
            self.len.fetch_sub(1, Ordering::AcqRel);
        }

        Err(())
    }

    /// Assuming we have 8 elements per slot, otherwise must update the assumption.
    fn enter(&self, get: bool) -> Result<u16, ()> {
        let src: u16 = self.bitmap.load(Ordering::Acquire);

        let mut pos = 0;
        let mut base = if get { src ^ GET_MASK } else { src ^ PUT_MASK };

        while base > 0 {
            if base & 0b11 == 0b11 {
                // update the state and the position
                return Ok(pos);
            }

            pos += 1;
            base >>= 2;
        }

        Err(())
    }
}

impl<T> Drop for Bucket<T> {
    fn drop(&mut self) {
        for item in self.slot.iter_mut() {
            unsafe {
                ptr::drop_in_place(*item);
            }
            *item = ptr::null_mut();
        }
    }
}

struct VisitorGuard<'a>(&'a AtomicUsize);

impl<'a> VisitorGuard<'a> {
    fn register(base: &'a (AtomicUsize, AtomicBool), get: bool) -> Option<Self> {
        let mut count = 0;

        // wait if the underlying storage is in protection mode
        while base.1.load(Ordering::Acquire) {
            if get {
                return None;
            }

            cpu_relax(count + 8);

            if count < 8 {
                count += 1;
            }
        }

        base.0.fetch_add(1, Ordering::SeqCst);
        Some(VisitorGuard(&base.0))
    }
}

impl<'a> Drop for VisitorGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) struct SyncPool<T> {
    /// The slots storage
    slots: Vec<Bucket<T>>,

    /// the next channel to try
    curr: (AtomicUsize, AtomicUsize),

    /// First node -- how many threads are concurrently accessing the struct:
    ///   0   -> updating the `slots` field;
    ///   1   -> no one is using the pool;
    ///   num -> number of visitors
    /// Second node -- write barrier:
    ///   true  -> write barrier has been raised
    ///   false -> no write barrier
    visitor_counter: (AtomicUsize, AtomicBool),
}

impl<T: Default> SyncPool<T> {
    pub fn new() -> Self {
        Self::make_pool(POOL_SIZE)
    }

    pub fn with_size(size: usize) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size)
    }

    pub fn get(&mut self) -> Box<T> {
        // update user count
        let guard = VisitorGuard::register(&self.visitor_counter, true);

        // if the pool itself is being operated on, no need to wait, just create the object on the fly.
        if guard.is_none() {
            return Default::default();
        }

        // start from where we're left
        let cap = self.slots.len();
        let mut trials = cap;
        let mut pos: usize = self.curr.0.load(Ordering::Acquire) % cap;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the access or move on
            if let Ok(i) = slot.access(true) {
                // try to checkout one slot
                let checkout = slot.checkout(i);
                slot.leave(i as u16);

                /*
                if slot.access(true) {
                    // try to checkout one slot
                    let checkout = slot.checkout();
                    slot.leave();
                */

                if let Ok(val) = checkout {
                    // now we're locked, get the val and update internal states
                    self.curr.0.store(pos, Ordering::Release);

                    // done
                    return val;
                }

                // failed to checkout, break and let the remainder logic to handle the rest
                break;
            }

            // update to the next position now.
            pos = self.curr.0.fetch_add(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                break;
            }
        }

        // make sure our guard has been returned if we want the correct visitor count
        drop(guard);

        Default::default()
    }

    pub fn put(&mut self, val: Box<T>) {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter, false);

        // start from where we're left
        let cap = self.slots.len();
        let mut pos: usize = self.curr.1.load(Ordering::Acquire) % cap;
        let mut trials = 2 * cap;

        loop {
            // check this slot
            let bucket = &mut self.slots[pos];

            // try the access or move on
            if let Ok(i) = bucket.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.1.store(pos, Ordering::Release);

                // put the value back and reset
                bucket.release(i, val);
                bucket.leave(i as u16);

                return;
            }

            /*
            if slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.1.store(pos, Ordering::Release);

                // put the value back into the slot
                slot.release(val, self.reset_handle.load(Ordering::Acquire));
                slot.leave();

                return;
            }
            */

            // update states
            pos = self.curr.1.fetch_add(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                break;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .fold(0, |sum, item|
                sum + item.size_hint()
            )
    }

    pub fn expand(&mut self, additional: usize, block: bool) -> bool {
        // raise the write barrier now, if someone has already raised the flag to indicate the
        // intention to write, let me go away.
        if self
            .visitor_counter
            .1
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Acquire)
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
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::Relaxed)
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
                self.slots.push(Bucket::new(true));
            });
        }

        // update the internal states
        self.visitor_counter.0.store(1, Ordering::SeqCst);
        self.visitor_counter.1.store(false, Ordering::Release);

        safe
    }

    pub fn refill(&mut self, amount: usize) {
        for _ in 0..amount {
            self.put(Default::default());
            thread::yield_now();
        }
    }

    fn make_pool(size: usize) -> Self {
        let mut s = Vec::with_capacity(size);

        (0..size).for_each(|_| {
            // add the slice back to the vec container
            s.push(Bucket::new(true));
        });

        SyncPool {
            slots: s,
            curr: (AtomicUsize::new(0), AtomicUsize::new(0)),
            visitor_counter: (AtomicUsize::new(1), AtomicBool::new(false)),
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

    pub(crate) fn as_mut(&mut self) -> Result<&mut T, ErrorKind> {
        self.0.as_mut().ok_or(ErrorKind::NotFound)
    }

    pub(crate) fn as_ref(&self) -> Result<&T, ErrorKind> {
        self.0.as_ref().ok_or(ErrorKind::NotFound)
    }

    pub(crate) fn set(&mut self, val: T) {
        self.0.replace(val);
    }

    pub(crate) fn take(&mut self) -> Option<T> {
        self.0.take()
    }
}

impl<T> Deref for StaticStore<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("Calling methods on uninitialized struct is forbidden...")
    }
}

impl<T> DerefMut for StaticStore<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
            .as_mut()
            .expect("Calling methods on uninitialized struct is forbidden...")
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
