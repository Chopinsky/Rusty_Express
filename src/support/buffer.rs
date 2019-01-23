#![allow(dead_code)]

use std::sync::{atomic::AtomicBool, atomic::Ordering, Once, ONCE_INIT};

static mut BUFFER: Option<Vec<Vec<u8>>> = None;
static mut DEFAULT_CAPACITY: usize = 1;
static mut LOCKED: AtomicBool = AtomicBool::new(false);

const ONCE: Once = ONCE_INIT;
const DEFAULT_GROWTH: usize = 4;

pub struct ByteBuffer {
    inner: Vec<Vec<u8>>,
}

impl ByteBuffer {
    pub fn write_into(&mut self) -> &mut Vec<u8> {
        assert_eq!(self.inner.len(), 1);
        &mut self.inner[0]
    }

    pub fn write_into_as_slice(&mut self) -> &mut [u8] {
        assert_eq!(self.inner.len(), 1);
        self.inner[0].as_mut_slice()
    }

    pub fn read(&self) -> &Vec<u8> {
        assert_eq!(self.inner.len(), 1);
        &self.inner[0]
    }

    pub fn swap(&mut self, buffer: Vec<u8>) -> Vec<u8> {
        self.inner.push(buffer);
        self.inner.swap_remove(0)
    }

    pub fn take(&mut self) -> Vec<u8> {
        self.inner.push(make_vec(unsafe { DEFAULT_CAPACITY }));
        self.inner.swap_remove(0)
    }

    pub fn clear(&mut self) {
        assert_eq!(self.inner.len(), 1);
        for val in self.inner[0].iter_mut() {
            *val = 0;
        }
    }
}

impl Drop for ByteBuffer {
    fn drop(&mut self) {
        if let Some(vec) = self.inner.pop() {
            push_back(vec);
        }
    }
}

pub fn init(size: usize, capacity: usize) {
    ONCE.call_once(|| {
        unsafe {
            let mut buffer = Vec::with_capacity(size);
            (0..size).for_each(|_| {
                buffer.push(make_vec(capacity));
            });

            BUFFER = Some(buffer);
            DEFAULT_CAPACITY = capacity;
        }
    });
}

pub fn get() -> ByteBuffer {
    unsafe {
        loop {
            // use a loop-and-hold method for cheap lock check
            if let Ok(false) = LOCKED.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::SeqCst
            ) {
                break;
            }
        }

        let inner =
            if let Some(ref mut buffer) = BUFFER {
                match buffer.pop() {
                    Some(vec) => vec,
                    None => {
                        let cap = DEFAULT_CAPACITY;
                        (0..DEFAULT_GROWTH).for_each(|_| {
                            buffer.push(make_vec(cap));
                        });

                        make_vec(cap)
                    }
                }
            } else {
                make_vec(DEFAULT_CAPACITY)
            };

        // the protected section is finished, release the lock
        *LOCKED.get_mut() = false;

        ByteBuffer {
            inner: vec![inner],
        }
    }
}

fn make_vec(capacity: usize) -> Vec<u8> {
    let mut vec = Vec::with_capacity(capacity);
    (0..capacity).for_each(|_| {
        vec.push(0);
    });

    vec
}

fn push_back(vec: Vec<u8>) {
    unsafe {
        if let Some(ref mut buffer) = BUFFER {
            buffer.push(vec);
        }
    }
}