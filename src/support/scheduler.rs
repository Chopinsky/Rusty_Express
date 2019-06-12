#![allow(dead_code)]

use std::sync::{atomic::AtomicBool, atomic::Ordering};
use std::thread;
use std::time::Duration;

use crate::channel::{self, Receiver, SendTimeoutError, Sender};
use crate::support::debug::{self, InfoLevel};
use parking_lot::{Once, OnceState, ONCE_INIT};

const CHAN_SIZE: usize = 512;
const POOL_CAP: usize = 65535;
const POOL_INC_STEP: usize = 4;
const TIMEOUT: Duration = Duration::from_millis(200);
const YIELD_DURATION: Duration = Duration::from_millis(64);

static IS_CLOSING: AtomicBool = AtomicBool::new(false);

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    #[inline]
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

type Job = Box<FnBox + Send + 'static>;
enum Message {
    NewJob(Job),
    Terminate,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    auto_expansion: bool,
}

impl ThreadPool {
    pub(crate) fn new(size: usize) -> ThreadPool {
        let pool_size = match size {
            _ if size < 1 => 1,
            _ if size > POOL_CAP => POOL_CAP,
            _ => size,
        };

        let (sender, receiver) = channel::bounded(CHAN_SIZE);

        let mut workers = Vec::with_capacity(pool_size);
        (0..pool_size).for_each(|id| {
            workers.push(Worker::launch(id, receiver.clone(), false));
        });

        ThreadPool {
            workers,
            sender,
            receiver,
            auto_expansion: false,
        }
    }

    pub(crate) fn toggle_auto_expansion(&mut self, on: bool) {
        self.auto_expansion = on;
    }

    pub(crate) fn execute<F>(&mut self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.dispatch(Message::NewJob(Box::new(f)), 0);
    }

    pub(crate) fn close(&mut self) {
        let sent = self.sender.send(Message::Terminate).is_ok();

        for mut worker in self.workers.drain(..) {
            if let Some(t) = worker.thread.take() {
                if sent {
                    // only sync join the threads if channel has not been closed; otherwise, it's
                    // possible that the worker may never receive the shutdown message and quit the
                    // infinite-loop.
                    t.join().unwrap_or_else(|err| {
                        debug::print(
                            &format!("Failed to retire worker: {}, error: {:?}", worker.id, err),
                            InfoLevel::Error,
                        )
                    });
                }
            }
        }
    }

    fn dispatch(&mut self, message: Message, retry: u8) {
        match self
            .sender
            .send_timeout(message, Duration::from_millis(8))
        {
            Err(SendTimeoutError::Timeout(msg)) => {
                debug::print("Unable to distribute the job: execution timed out, all workers are busy for too long", InfoLevel::Error);

                if retry < 4 {
                    if self.auto_expansion && self.workers.len() + POOL_INC_STEP < POOL_CAP {
                        let start = self.workers.len();
                        (0..POOL_INC_STEP).for_each(|id| {
                            self.workers
                                .push(Worker::launch(start + id, self.receiver.clone(), true));
                        });
                    }

                    debug::print(
                        &format!("Try again for the {} times...", retry + 1),
                        InfoLevel::Error,
                    );

                    self.dispatch(msg, retry + 1);
                }
            }
            Err(SendTimeoutError::Disconnected(_)) => {
                debug::print(
                    "Unable to distribute the job: workers have been dropped: {}",
                    InfoLevel::Error,
                );
            }
            _ => {}
        };
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        debug::print(
            "Job done, sending terminate message to all workers.",
            InfoLevel::Info,
        );
        self.close();
    }
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn launch(id: usize, work_queue: Receiver<Message>, is_extra: bool) -> Worker {
        let thread = thread::spawn(move || {
            let mut work = None;

            loop {
                if IS_CLOSING.load(Ordering::Relaxed) {
                    return;
                }

                if let Ok(message) = work_queue.recv_timeout(YIELD_DURATION) {
                    match message {
                        Message::NewJob(job) => work = Some(job),
                        Message::Terminate => {
                            IS_CLOSING.store(true, Ordering::Release);
                            return;
                        }
                    }
                } else if is_extra {
                    return;
                }

                if let Some(job) = work.take() {
                    job.call_box();
                }
            }
        });

        Worker {
            id,
            thread: Some(thread),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // make sure the work is done
            thread.join().unwrap_or_else(|err| {
                debug::print(
                    &format!("Unable to drop worker: {}, error: {:?}", self.id, err),
                    InfoLevel::Error,
                );
            });
        }
    }
}

struct Pool {
    req_workers: ThreadPool,
    resp_workers: ThreadPool,
    parser_workers: ThreadPool,
    stream_workers: ThreadPool,
}

pub enum TaskType {
    Request,
    Response,
    Parser,
    StreamLoader,
}

static ONCE: Once = ONCE_INIT;
static mut POOL: Option<Pool> = None;

pub(crate) fn initialize_with(sizes: Vec<usize>) {
    assert_eq!(
        ONCE.state(), OnceState::New,
        ">>> Only 1 instance of the server is allowed per process ... <<<"
    );

    ONCE.call_once(|| {
        let pool_sizes: Vec<usize> = sizes
            .iter()
            .map(|val| match val {
                0 => 1,
                _ => *val,
            })
            .collect();

        let (worker_size, parser_size) = match pool_sizes.len() {
            1 => (pool_sizes[0], pool_sizes[0]),
            2 => (pool_sizes[0], pool_sizes[1]),
            _ => panic!("Requiring vec sizes of 2 for each, or 1 for all"),
        };

        // Put it in the heap so it can outlive this call
        unsafe {
            POOL.replace(Pool {
                req_workers: ThreadPool::new(worker_size),
                resp_workers: ThreadPool::new(worker_size),
                parser_workers: ThreadPool::new(parser_size),
                stream_workers: ThreadPool::new(parser_size),
            });
        }
    });
}

pub(crate) fn run<F>(f: F, task: TaskType)
where
    F: FnOnce() + Send + 'static,
{
    unsafe {
        if let Some(ref mut pool) = POOL {
            // if pool has been created
            match task {
                TaskType::Request => pool.req_workers.execute(f),
                TaskType::Response => pool.resp_workers.execute(f),
                TaskType::Parser => pool.parser_workers.execute(f),
                TaskType::StreamLoader => pool.stream_workers.execute(f),
            };

            return;
        }

        // otherwise, spawn to a new thread for the work;
        thread::spawn(f);
    }
}

pub(crate) fn close() {
    unsafe {
        if let Some(mut pool) = POOL.take() {
            pool.req_workers.close();
            pool.resp_workers.close();
        }
    }
}
