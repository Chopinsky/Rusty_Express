#![allow(dead_code)]

use std::io::ErrorKind;
use std::str;
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Once, ONCE_INIT};
use std::time::{Duration, SystemTime};
use std::vec;

static mut BUFFER: Option<Vec<ByteBuffer>> = None;
static mut DEFAULT_CAPACITY: usize = 1;

static mut LOCK: AtomicBool = AtomicBool::new(false);
static mut BUF_SIZE: AtomicUsize = AtomicUsize::new(0);

const ONCE: Once = ONCE_INIT;
const LOCK_TIMEOUT: Duration = Duration::from_millis(64);
const DEFAULT_GROWTH: usize = 4;
const BUF_ROOF: usize = 65535;

pub(crate) struct ByteBuffer {
    buf: Vec<u8>,
    is_lent: bool,
}

impl ByteBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        ByteBuffer {
            buf: vec::from_elem(0, capacity),
            is_lent: false,
        }
    }

    pub(crate) fn update_status(&mut self, is_lent: bool) {
        self.is_lent = is_lent;
    }

    // Super unsafe, as we're using super unsafe [`Vec::from_raw_parts`] here... Swap out the inner
    // buf and replace it with a new Vec<u8>. The swapped out buf will transfer the ownership to
    // the caller of this function.
    fn buf_swap(&mut self, target: Vec<u8>) -> Vec<u8> {
        let res = unsafe {
            Vec::from_raw_parts(self.buf.as_mut_ptr(), self.buf.len(), self.buf.capacity())
        };

        self.buf = target;
        res
    }
}

pub(crate) trait BufferOp {
    fn as_writable(&mut self) -> &mut Vec<u8>;
    fn as_writable_slice(&mut self) -> &mut [u8];
    fn read(&self) -> &Vec<u8>;
    fn read_as_slice(&self) -> &[u8];
    fn reset(&mut self);
    fn try_into_string(&self) -> Result<String, String>;
}

impl BufferOp for ByteBuffer {
    fn as_writable(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    fn as_writable_slice(&mut self) -> &mut [u8] {
        self.buf.as_mut_slice()
    }

    fn read(&self) -> &Vec<u8> {
        &self.buf
    }

    fn read_as_slice(&self) -> &[u8] {
        &self.buf.as_slice()
    }

    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    fn try_into_string(&self) -> Result<String, String> {
        match str::from_utf8(&self.buf.as_slice()) {
            Ok(raw) => Ok(String::from(raw)),
            Err(e) => Err(format!(
                "Unable to convert the buffered data into utf-8 string, error occurs at {}",
                e.valid_up_to()
            )),
        }
    }
}

impl Drop for ByteBuffer {
    fn drop(&mut self) {
        // if buffer is dropped without being released back to the buffer pool, try save it.
        if self.is_lent {
            // swap the pointer out so it won't be killed by drop
            let vec = self.buf_swap(Vec::new());

            push_back(ByteBuffer {
                buf: vec,
                is_lent: false,
            });
        }
    }
}

pub(crate) fn init(size: usize, capacity: usize) {
    ONCE.call_once(|| {
        let mut buffer = Vec::with_capacity(size);
        (0..size).for_each(|_| {
            buffer.push(ByteBuffer::new(capacity));
        });

        unsafe {
            BUFFER = Some(buffer);
            DEFAULT_CAPACITY = capacity;
            BUF_SIZE.fetch_add(size, Ordering::SeqCst);
        }
    });
}

pub(crate) fn reserve() -> ByteBuffer {
    let buf = match try_reserve() {
        Some(buf) => buf,
        None => unsafe {
            let cap = DEFAULT_CAPACITY;
            let (buf, inc) =
                if let Some(ref mut buffer) = BUFFER {
                    // the BUFFER store is still valid
                    if BUF_SIZE.load(Ordering::SeqCst) > BUF_ROOF {
                        // already blow the memory guard, be gentle
                        (ByteBuffer::new(cap), 1)
                    } else {
                        // grow the buffer with pre-determined size
                        if lock().is_ok() {
                            (0..DEFAULT_GROWTH).for_each(|_| {
                                buffer.push(ByteBuffer::new(cap));
                            });

                            unlock();
                        }

                        // don't bother pop again, lend a new slice
                        (ByteBuffer::new(cap), DEFAULT_GROWTH + 1)
                    }
                } else {
                    // can't get a hold of the BUFFER store, just make the slice
                    (ByteBuffer::new(cap), 1)
                };

            // update the buffer size -- including the lent out ones
            BUF_SIZE.fetch_add(inc, Ordering::SeqCst);

            buf
        }
    };

    buf
}

pub(crate) fn try_reserve() -> Option<ByteBuffer> {
    unsafe {
        // wait for the lock
        if lock().is_err() {
            return None;
        }

        let res =
            if let Some(ref mut buffer) = BUFFER {
                match buffer.pop() {
                    Some(vec) => Some(vec),
                    None => None,
                }
            } else {
                None
            };

        unlock();
        res
    }
}

pub(crate) fn release(buf: ByteBuffer) {
    push_back(buf);
}

fn push_back(buf: ByteBuffer) {
    let mut buf_slice = buf;

    // the ownership of the buffer slice is returned, update the status as so regardless if it
    // needs to be dropped right away
    buf_slice.update_status(false);

    unsafe {
        if BUF_SIZE.load(Ordering::SeqCst) > BUF_ROOF {
            // if we've issued too many buffer slices, just let this one expire on itself
            BUF_SIZE.fetch_sub(1, Ordering::SeqCst);

            return;
        }

        if buf_slice.buf.capacity() > DEFAULT_CAPACITY {
            buf_slice.buf.truncate(DEFAULT_CAPACITY);
        }

        if let Some(ref mut buffer) = BUFFER {
            buf_slice.reset();

            if lock().is_ok() {
                buffer.push(buf_slice);
                unlock();
            }
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