use std::sync::{Once, ONCE_INIT};
use std::{mem, thread};
use support::threads::*;

static POOL_SIZE: usize = 8;

struct Pool {
    store: Option<ThreadPool>,
}

pub fn execute<F>(f: F)
    where F: FnOnce() + Send + 'static {

    static ONCE: Once = ONCE_INIT;
    static mut POOL: Pool = Pool { store: None };

    unsafe {
        ONCE.call_once(|| {
            // Make it
            let pool = Pool { store: Some(ThreadPool::new(POOL_SIZE)) };

            // Put it in the heap so it can outlive this call
            POOL = mem::transmute(pool);
        });

        if let Some(ref store) = POOL.store {
            // if pool is created
            store.execute(f);
        } else {
            // otherwise, spawn to a new thread for the work;
            thread::spawn(f);
        }
    }
}