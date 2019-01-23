#![allow(dead_code)]

use std::str;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Once, ONCE_INIT};
use std::vec;

static mut BUFFER: Option<Vec<Vec<u8>>> = None;
static mut DEFAULT_CAPACITY: usize = 1;
static mut LOCKED: AtomicBool = AtomicBool::new(false);

const ONCE: Once = ONCE_INIT;
const DEFAULT_GROWTH: usize = 4;

pub struct ByteBuffer {
    inner: Vec<Vec<u8>>,
}

impl ByteBuffer {
    pub fn as_writable(&mut self) -> &mut Vec<u8> {
        assert_eq!(self.inner.len(), 1);
        &mut self.inner[0]
    }

    pub fn as_writable_slice(&mut self) -> &mut [u8] {
        assert_eq!(self.inner.len(), 1);
        self.inner[0].as_mut_slice()
    }

    pub fn read(&self) -> &Vec<u8> {
        assert_eq!(self.inner.len(), 1);
        &self.inner[0]
    }

    pub fn swap(&mut self, buffer: Vec<u8>) -> Vec<u8> {
        let old = self.inner.swap_remove(0);
        self.inner.push( buffer);
        old
    }

    pub fn take(&mut self) -> Vec<u8> {
        let old = self.inner.swap_remove(0);
        self.inner.push(vec::from_elem(0, unsafe { DEFAULT_CAPACITY }));
        old
    }

    pub fn reset(&mut self) {
        assert_eq!(self.inner.len(), 1);
        self.inner[0].iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    pub fn try_into_string(&self) -> Result<String, String> {
        match str::from_utf8(&self.inner[0].as_slice()) {
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
        // reset to 0 first
        self.reset();

        // then move the vec back
        if self.inner.len() == 1 {
            push_back(self.inner.swap_remove(0));
        }
    }
}

pub fn init(size: usize, capacity: usize) {
    ONCE.call_once(|| {
        unsafe {
            let mut buffer = Vec::with_capacity(size);
            (0..size).for_each(|_| {
                buffer.push(vec::from_elem(0, capacity));
            });

            BUFFER = Some(buffer);
            DEFAULT_CAPACITY = capacity;
        }
    });
}

pub fn reserve() -> ByteBuffer {
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
                            buffer.push(vec::from_elem(0, cap));
                        }); 

                        vec::from_elem(0, cap)
                    }
                }
            } else {
                vec::from_elem(0, DEFAULT_CAPACITY)
            };

        // the protected section is finished, release the lock
        *LOCKED.get_mut() = false;

        ByteBuffer {
            inner: vec![inner; 1],
        }
    }
}

fn push_back(vec: Vec<u8>) {
    unsafe {
        if let Some(ref mut buffer) = BUFFER {
            buffer.push(vec);
        }
    }
}