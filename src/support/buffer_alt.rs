#![allow(dead_code)]

use std::io::ErrorKind;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Once, ONCE_INIT};
use std::time::{Duration, SystemTime};
use std::thread::{self, JoinHandle};
use std::vec;
use crate::channel::{self, Receiver, Sender};

const ONCE: Once = ONCE_INIT;
const LOCK_TIMEOUT: Duration = Duration::from_millis(64);
const DEFAULT_GROWTH: u8 = 4;

static mut LOCK: AtomicBool = AtomicBool::new(false);
static mut BUFFER: Option<BufferPool> = None;

enum BufferOperation {
    Reserve(bool),
    Release(usize),
    Extend(usize),
}

struct BufferPool {
    store: Vec<Vec<u8>>,
    pool: Vec<usize>,
    slice_capacity: usize,
    closing: AtomicBool,
}

impl BufferPool {
    fn reserve(&mut self, force: bool) -> Option<ByteBuffer> {
        match self.pool.pop() {
            Some(id) => Some(ByteBuffer { id }),
            None => {
                if force {
                    Some(ByteBuffer {
                        id: self.extend(DEFAULT_GROWTH as usize)
                    })
                } else {
                    None
                }
            }
        }
    }

    fn release(&mut self, id: usize) {
        if id < self.store.len() {
            self.pool.push(id);
        }
    }

    fn reset(&mut self, id: usize) {
        assert!(id < self.store.len());

        let capacity: usize = self.slice_capacity;
        if self.store[id].capacity() > capacity {
            self.store[id].truncate(capacity);
        }

        self.store[id].iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    fn reset_slice(id: usize) {
        unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                buf.reset(id);
            }
        }
    }

    fn extend(&mut self, count: usize) -> usize {
        assert!(count > 0);

        let capacity = self.slice_capacity;
        let start = self.store.len();

        self.store.reserve(count);
        self.pool.reserve(count);

        (0..count).for_each(|id| {
            self.store.push(vec::from_elem(0, capacity));
            self.pool.push(start + id);
        });

        // return the last element in the buffer
        self.store.len() - 1
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        *self.closing.get_mut() = true;
    }
}

pub(crate) struct ByteBuffer {
    id: usize
}

impl ByteBuffer {
    pub(crate) fn as_writable(&self) -> Result<&mut [u8], ErrorKind> {
        unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                if buf.closing.load(Ordering::SeqCst) {
                    return Err(ErrorKind::NotConnected);
                }

                if self.id < buf.store.len() {
                    return Ok(buf.store[self.id].as_mut_slice());
                } else {
                    return Err(ErrorKind::InvalidData);
                }
            }
        }

        Err(ErrorKind::NotConnected)
    }

    pub(crate) fn as_writable_vec(&self) -> Result<&mut Vec<u8>, ErrorKind> {
        unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                if buf.closing.load(Ordering::SeqCst) {
                    return Err(ErrorKind::NotConnected);
                }

                if self.id < buf.store.len() {
                    return Ok(&mut buf.store[self.id]);
                } else {
                    return Err(ErrorKind::InvalidData);
                }
            }
        }

        Err(ErrorKind::NotConnected)
    }

    pub(crate) fn read(&self) -> Result<&[u8], ErrorKind> {
        unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                if buf.closing.load(Ordering::SeqCst) {
                    return Err(ErrorKind::NotConnected);
                }

                if self.id < buf.store.len() {
                    return Ok(buf.store[self.id].as_slice());
                } else {
                    return Err(ErrorKind::InvalidData);
                }
            }
        }

        Err(ErrorKind::NotConnected)
    }

    pub(crate) fn copy_to_vec(&self) -> Result<Vec<u8>, ErrorKind> {
        Ok(self.read()?.to_vec())
    }
}

impl Drop for ByteBuffer {
    fn drop(&mut self) {
        BufferPool::reset_slice(self.id);
        manage_buffer(BufferOperation::Release(self.id));
    }
}

pub(crate) fn init(size: usize, capacity: usize) {
    ONCE.call_once(|| {
        let mut store = Vec::with_capacity(size);
        let mut pool = Vec::with_capacity(size);

        (0..size).for_each(|id| {
            store.push(vec::from_elem(0, capacity));
            pool.push(id);
        });

        unsafe {
            BUFFER = Some(BufferPool {
                store,
                pool,
                slice_capacity: capacity,
                closing: AtomicBool::new(false),
            });
        }
    });
}

pub(crate) fn slice() -> ByteBuffer {
    manage_buffer(BufferOperation::Reserve(true)).unwrap()
}

#[inline]
pub(crate) fn try_slice() -> Option<ByteBuffer> {
    manage_buffer(BufferOperation::Reserve(false))
}

fn manage_buffer(command: BufferOperation) -> Option<ByteBuffer> {
    if lock().is_err() {
        return None;
    }

    let result = unsafe {
        if let Some(buf) = BUFFER.as_mut() {
            match command {
                BufferOperation::Reserve(forced) => buf.reserve(forced),
                BufferOperation::Release(id) => {
                    buf.release(id);
                    None
                },
                BufferOperation::Extend(count) => {
                    buf.extend(count);
                    None
                }
            }
        } else {
            None
        }
    };

    unlock();
    result
}

fn lock() -> Result<(), ErrorKind> {
    let start = SystemTime::now();

    loop {
        unsafe {
            match LOCK.compare_exchange(
                false, true, Ordering::SeqCst, Ordering::SeqCst
            ) {
                Ok(res) => if res == false {
                    // if not locked previously, we've grabbed the lock and break the wait
                    break;
                },
                Err(_) => {
                    // locked by someone else,
                },
            }
        };

        match start.elapsed() {
            Ok(period) => {
                if period > LOCK_TIMEOUT {
                    return Err(ErrorKind::TimedOut);
                }
            },
            _ => return Err(ErrorKind::TimedOut),
        }
    }

    Ok(())
}

#[inline]
fn unlock() {
    unsafe { *LOCK.get_mut() = false; }
}