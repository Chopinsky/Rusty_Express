use crate::support::common;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

const RETRY_LIMIT: usize = 16;
const PARK_TIMEOUT: u64 = 16;

pub struct SpinLockGuard<'a> {
    inner: &'a SpinLock,
}

impl Drop for SpinLockGuard<'_> {
    fn drop(&mut self) {
        self.inner.unlock();
    }
}

pub struct SpinLock {
    lock: Arc<LockInner>,
}

struct LockInner {
    flag: AtomicBool,
}

impl SpinLock {
    pub fn new() -> Self {
        SpinLock {
            lock: Arc::new(LockInner {
                flag: AtomicBool::new(false),
            }),
        }
    }

    pub fn lock(&self, new: i8) -> SpinLockGuard<'_> {
        // retry counter
        let mut retry = 0;
        let mut yielded = false;

        // wait till the mutating state is restored to state 0
        while self
            .lock
            .flag
            .compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::SeqCst)
            != Ok(false)
        {
            if retry < RETRY_LIMIT {
                retry += 1;
                common::cpu_relax(retry);
            } else if !yielded {
                // update handles
                yielded = true;
                retry = 0;

                thread::yield_now();
            } else {
                self.park();
            }
        }

        SpinLockGuard { inner: self }
    }

    pub fn unlock(&self) {
        self.lock.flag.store(false, Ordering::Release);
        self.notify_one();
    }
}

trait ThreadParker {
    fn park(&self);
    fn notify_one(&self);
}

impl ThreadParker for SpinLock {
    fn park(&self) {
        thread::yield_now();
    }

    fn notify_one(&self) {}
}

impl Clone for SpinLock {
    fn clone(&self) -> Self {
        SpinLock {
            lock: Arc::clone(&self.lock),
        }
    }
}

//TODO: seqlock
