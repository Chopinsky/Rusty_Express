#![allow(dead_code)]

use std::io::ErrorKind;
use std::str;
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Once, ONCE_INIT};
use std::time::{Duration, SystemTime};
use std::thread;
use std::vec;
use crate::channel::{self as channel, Sender, Receiver};

const ONCE: Once = ONCE_INIT;
const LOCK_TIMEOUT: Duration = Duration::from_millis(64);
const DEFAULT_GROWTH: u8 = 4;
const DEFAULT_CAPACITY: usize = 512;

static mut LOCK: AtomicBool = AtomicBool::new(false);
static mut BUFFER: Option<BufferPool> = None;
static mut SIZE_CAP: AtomicUsize = AtomicUsize::new(65535);

enum BufOp {
    Reserve(bool),
    Release(usize),
    ReleaseAndExtend(Vec<u8>),
    Extend(usize),
}

enum WorkerOp {
    Cleanup(usize),
    Shutdown,
}

struct BufferPool {
    store: Vec<Vec<u8>>,
    pool: Vec<usize>,
    slice_capacity: usize,
    worker_chan: Sender<WorkerOp>,
    closing: AtomicBool,
}

pub(crate) struct ByteBuffer {}

impl ByteBuffer {
    pub(crate) fn init(size: usize, capacity: usize) {
        ONCE.call_once(|| {
            let mut store = Vec::with_capacity(size);
            let mut pool = Vec::with_capacity(size);

            (0..size).for_each(|id| {
                store.push(vec::from_elem(0, capacity));
                pool.push(id);
            });

            let (sender, receiver) = channel::bounded(8);
            thread::spawn(move || {
               BufferPool::handle_work(receiver);
            });

            BufferPool::make(BufferPool {
                store,
                pool,
                slice_capacity: capacity,
                worker_chan: sender,
                closing: AtomicBool::new(false),
            });
        });
    }

    pub(crate) fn slice() -> BufferSlice {
        match BufferPool::manage(BufOp::Reserve(true)) {
            Some(val) => val,
            None => unsafe {
                let capacity = if let Some(buf) = BUFFER.as_ref() {
                    buf.slice_capacity
                } else {
                    // guess the capacity
                    DEFAULT_CAPACITY
                };

                BufferSlice { id: 0, fallback: Some(vec::from_elem(0, capacity)) }
            },
        }
    }

    #[inline]
    pub(crate) fn try_slice() -> Option<BufferSlice> {
        BufferPool::manage(BufOp::Reserve(false))
    }
}

trait BufferOperations {
    fn reserve(&mut self, force: bool) -> Option<BufferSlice>;
    fn release(&mut self, id: usize);
    fn reset(&mut self, id: usize);
    fn extend(&mut self, count: usize) -> usize;
}

impl BufferOperations for BufferPool {
    fn reserve(&mut self, force: bool) -> Option<BufferSlice> {
        match self.pool.pop() {
            Some(id) => Some(BufferSlice {
                id,
                fallback: None,
            }),
            None => {
                if force {
                    Some(BufferSlice {
                        id: self.extend(DEFAULT_GROWTH as usize),
                        fallback: None,
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
        let vec_cap: usize = self.store[id].capacity();

        if vec_cap > capacity {
            self.store[id].truncate(capacity);
        } else if vec_cap < capacity {
            self.store[id].reserve(capacity - vec_cap);
        }

        self.store[id].iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    fn extend(&mut self, count: usize) -> usize {
        assert!(count > 0);

        let capacity = self.slice_capacity;
        let start = self.store.len();

        self.store.reserve(count);
        self.pool.reserve(count);

        (0..count).for_each(|offset| {
            self.store.push(vec::from_elem(0, capacity));
            self.pool.push(start + offset);
        });

        // return the last element in the buffer
        self.store.len() - 1
    }
}

trait BufferManagement {
    fn make(buf: BufferPool);
    fn reset_and_release(id: usize);
    fn handle_work(rx: Receiver<WorkerOp>);
    fn manage(command: BufOp) -> Option<BufferSlice>;
}

impl BufferManagement for BufferPool {
    fn make(buf: BufferPool) {
        unsafe { BUFFER = Some(buf); }
    }

    fn reset_and_release(id: usize) {
        unsafe {
            if let Some(buf) = BUFFER.as_ref() {
                buf.worker_chan
                    .send(WorkerOp::Cleanup(id))
                    .unwrap_or_else(|err| {
                        eprintln!("Failed to release buffer slice: {}, err: {}", id, err);
                    });
            }
        }
    }

    fn handle_work(rx: Receiver<WorkerOp>) {
        for message in rx.recv() {
            match message {
                WorkerOp::Cleanup(id) => BufferPool::manage(BufOp::Release(id)),
                WorkerOp::Shutdown => return,
            };
        }
    }

    fn manage(command: BufOp) -> Option<BufferSlice> {
        if lock().is_err() {
            return None;
        }

        let result = unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                match command {
                    BufOp::Reserve(forced) => buf.reserve(forced),
                    BufOp::Release(id) => {
                        buf.reset(id);
                        buf.release(id);
                        None
                    },
                    BufOp::Extend(count) => {
                        buf.extend(count);
                        None
                    },
                    BufOp::ReleaseAndExtend(vec) => {
                        if buf.store.len() < SIZE_CAP.load(Ordering::SeqCst) {
                            let id = buf.store.len();

                            buf.store.push(vec);
                            buf.pool.push(id);
                            buf.reset(id);
                        }

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
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        *self.closing.get_mut() = true;
    }
}

pub(crate) struct BufferSlice {
    id: usize,
    fallback: Option<Vec<u8>>,
}

impl BufferSlice {
    pub(crate) fn as_writable(&mut self) -> Result<&mut [u8], ErrorKind> {
        match self.fallback {
            Some(ref mut vec) => return Ok(vec.as_mut_slice()),
            None => {},
        }

        if let Some(buf) = unsafe { BUFFER.as_mut() } {
            if buf.closing.load(Ordering::SeqCst) {
                return Err(ErrorKind::NotConnected);
            }

            if self.id < buf.store.len() {
                return Ok(buf.store[self.id].as_mut_slice());
            } else {
                return Err(ErrorKind::InvalidData);
            }
        }

        Err(ErrorKind::NotConnected)
    }

    pub(crate) fn as_writable_vec(&mut self) -> Result<&mut Vec<u8>, ErrorKind> {
        match self.fallback {
            Some(ref mut vec) => return Ok(vec),
            None => {},
        }

        if let Some(buf) = unsafe { BUFFER.as_mut() } {
            if buf.closing.load(Ordering::SeqCst) {
                return Err(ErrorKind::NotConnected);
            }

            if self.id < buf.store.len() {
                return Ok(&mut buf.store[self.id]);
            } else {
                return Err(ErrorKind::InvalidData);
            }
        }

        Err(ErrorKind::NotConnected)
    }

    pub(crate) fn read(&self) -> Result<&[u8], ErrorKind> {
        match self.fallback {
            Some(ref vec) => return Ok(vec.as_slice()),
            None => {},
        }

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

    pub(crate) fn reset(&mut self) {
        unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                buf.reset(self.id);
            }
        }
    }

    pub(crate) fn try_into_string(&self) -> Result<&str, ErrorKind> {
        let buf = self.read()?;
        match str::from_utf8(buf) {
            Ok(raw) => Ok(raw),
            Err(e) => Err(ErrorKind::InvalidData),
        }
    }

    pub(crate) fn len(&self) -> usize {
        unsafe {
            if let Some(buf) = BUFFER.as_mut() {
                buf.store[self.id].len()
            } else {
                0
            }
        }
    }
}

impl Drop for BufferSlice {
    fn drop(&mut self) {
        if self.id == 0 && self.fallback.is_some() {
            BufferPool::manage(BufOp::ReleaseAndExtend(self.fallback.take().unwrap()));
        } else {
            BufferPool::reset_and_release(self.id);
        }
    }
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