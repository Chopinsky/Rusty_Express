#![allow(dead_code)]

use std::io::ErrorKind;
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Once, ONCE_INIT};
use std::time::{Duration, SystemTime};
use std::thread::{self, JoinHandle};
use std::vec;
use crate::channel::{self, Receiver, Sender};

const ONCE: Once = ONCE_INIT;
const LOCK_TIMEOUT: Duration = Duration::from_millis(64);

static mut LOCK: AtomicBool = AtomicBool::new(false);
static mut BUFFER: Option<BufferPool> = None;
static mut DEFAULT_CAPACITY: AtomicUsize = AtomicUsize::new(1);

enum WorkRequest {
    Reserve,
    Assign(usize),
    AssignmentFailed,
    Release(usize),
    Shutdown,
}

enum WorkerMessage {
    Clear(usize),
    Done(usize),
}

struct BufferPool {
    store: Vec<Vec<u8>>,
    closing: AtomicBool,
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
            if let Some(ref mut buf) = BUFFER {
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
            if let Some(ref mut buf) = BUFFER {
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
            if let Some(ref mut buf) = BUFFER {
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
        let vec = self.read()?;
        Ok(vec.to_vec())
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
            DEFAULT_CAPACITY.store(capacity, Ordering::SeqCst);

            let (req_rx, req_tx) = channel::bounded(0);
            let (resp_rx, resp_tx) = channel::bounded(0);

            let (worker_rx, worker_tx) = channel::unbounded();
            let (manager_rx, manager_tx) = channel::unbounded();

            let manager_thread = thread::spawn(|| {
                manage_pool(pool, req_tx, resp_rx, worker_rx, manager_tx);
            });

            let worker_thread = thread::spawn(|| {
                manage_worker(worker_tx, manager_rx);
            });

            BUFFER = Some(BufferPool {
                store,
                closing: AtomicBool::new(false),
            });
        }
    });
}

pub(crate) fn slice() -> ByteBuffer {
    unimplemented!();
}

fn clear_slice(buf: &mut BufferPool, id: usize) {
    let capacity: usize = unsafe { DEFAULT_CAPACITY.load(Ordering::SeqCst) };
    if buf.store[id].capacity() > capacity {
        buf.store[id].truncate(capacity);
    }

    buf.store[id].iter_mut().for_each(|val| {
        *val = 0;
    });
}

fn manage_pool(
    pool: Vec<usize>,
    req_chan: Receiver<WorkRequest>,
    resp_chan: Sender<WorkRequest>,
    worker_chan: Sender<WorkerMessage>,
    manager_chan: Receiver<WorkerMessage>)
{
    loop {
        channel::select! {
            recv(req_chan) -> message => {

            },
            recv(manager_chan) -> message => {

            }
        }
    }
}

fn manage_worker(worker_chan: Receiver<WorkerMessage>, manager_chan: Sender<WorkerMessage>) {
    loop {
        for message in worker_chan.recv() {
            match message {
                WorkerMessage::Clear(id) => unsafe {
                    if let Some(ref mut buf) = BUFFER {
                        clear_slice(buf, id);
                    }
                },
                _ => unreachable!(),
            }
        }
    }
}

fn extend(count: usize) -> usize {
    assert!(count > 0);

    unsafe {
        if let Some(ref mut buf) = BUFFER {
            let capacity = DEFAULT_CAPACITY.load(Ordering::SeqCst);

            buf.store.reserve(count);
            (0..count).for_each(|_| {
                buf.store.push(vec::from_elem(0, capacity));
            });

            //TODO: send message to pool manager to update self with additional nodes

            // return the last element in the buffer
            buf.store.len() - 1
        } else {
            panic!("Try to use the buffer before it is initialized");
        }
    }
}

fn lock() -> Result<(), ErrorKind> {
    let start = SystemTime::now();
    loop {
        let locked = unsafe {
            match LOCK.compare_exchange(
                false, true, Ordering::SeqCst, Ordering::SeqCst
            ) {
                Ok(res) => res == false,
                Err(_) => false,
            }
        };

        if locked {
            break;
        }

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

fn unlock() {
    unsafe {
        *LOCK.get_mut() = false;
    }
}