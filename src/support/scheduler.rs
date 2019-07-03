#![allow(dead_code)]

use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Arc};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::channel::{self, Receiver, SendTimeoutError, Sender};
use crate::support::debug::{self, InfoLevel};
use parking_lot::{Once, OnceState, ONCE_INIT, Mutex};
use hashbrown::HashSet;
use crossbeam_channel::RecvTimeoutError;

const CHAN_SIZE: usize = 512;
const POOL_CAP: usize = 1024;
const POOL_INC_STEP: usize = 4;
const TIMEOUT: Duration = Duration::from_millis(200);
const YIELD_DURATION: Duration = Duration::from_millis(128);

static IS_CLOSING: AtomicBool = AtomicBool::new(false);
static SOFT_POOL_CAP: AtomicUsize = AtomicUsize::new(POOL_CAP);

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
    pressure_status: (
        Option<Duration>,   // -> if we should drop the request after certain period
        Option<SystemTime>, // -> all workers are busy since this system time
    ),
    grave: Arc<Mutex<HashSet<usize>>>,
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
            workers.push(Worker::launch(id, receiver.clone(), None));
        });

        ThreadPool {
            workers,
            sender,
            receiver,
            auto_expansion: false,
            pressure_status: (None, None),
            grave: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub(crate) fn toggle_auto_expansion(&mut self, on: bool, cap: Option<usize>) {
        self.auto_expansion = on;
        if let Some(c) = cap {
            SOFT_POOL_CAP.store(c, Ordering::Release);
        }
    }

    pub(crate) fn is_under_pressure(&self) -> bool {
        if let Some(threshold) = self.pressure_status.0 {
            if let Some(since) = self.pressure_status.1 {
                return since.elapsed().unwrap_or_default() > threshold
            }
        }

        false
    }
    
    pub(crate) fn execute<F>(&mut self, f: F) -> u8
    where
        F: FnOnce() + Send + 'static,
    {
        self.dispatch(Message::NewJob(Box::new(f)), 0)
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

    fn dispatch(&mut self, message: Message, retry: u8) -> u8 {
        match self
            .sender
            .send_timeout(message, Duration::from_millis(2))
        {
            Ok(()) => {
                // if we care about under-pressure dropping, then reset the timer
                if self.pressure_status.0.is_some() {
                    self.pressure_status.1 = None;
                }
            },
            Err(SendTimeoutError::Timeout(msg)) => {
                debug::print("Unable to distribute the job: execution timed out, all workers are busy for too long", InfoLevel::Error);

                // set the busy_since timer
                if self.pressure_status.0.is_some() && self.pressure_status.1.is_none() {
                    self.pressure_status.1 = Some(SystemTime::now());
                }

                if retry < 4 {
                    self.expand();

                    debug::print(
                        &format!("Try again for the {} times...", retry + 1),
                        InfoLevel::Error,
                    );

                    return self.dispatch(msg, retry + 1);
                }
            },
            Err(SendTimeoutError::Disconnected(_)) => {
                debug::print(
                    "Unable to distribute the job: workers have been dropped: {}",
                    InfoLevel::Error,
                );

                return 1;
            },
        };

        0
    }

    fn expand(&mut self) {
        if self.auto_expansion && self.workers.len() + POOL_INC_STEP < POOL_CAP {
            // clean up died workers
            {
                let mut g = self.grave.lock();
                if g.len() > 0 {
                    self.workers.retain(|worker| {
                        !g.contains(&worker.id)
                    });
                }

                g.clear();
            }

            // then expand with new workers
            let start = self.workers[self.workers.len()-1].id;
            (0..POOL_INC_STEP).for_each(|id| {
                self.workers
                    .push(Worker::launch(
                        start + id,
                        self.receiver.clone(),
                        Some(self.grave.clone()),
                    ));
            });
        }
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
    fn launch(id: usize, work_queue: Receiver<Message>, grave: Option<Arc<Mutex<HashSet<usize>>>>) -> Worker {
        let thread = thread::spawn(move || {
            let mut idle_counter = 0;
            let mut message: Result<Message, RecvTimeoutError>;

            loop {
                if IS_CLOSING.load(Ordering::Relaxed) {
                    return;
                }

                message = work_queue.recv_timeout(YIELD_DURATION);

                if let Ok(message) = message {
                    match message {
                        Message::NewJob(job) => {
                            // process the work
                            job.call_box();

                            // give 2 more idle chances on every work processed
                            if idle_counter > 1 {
                                idle_counter -= 2;
                            }
                        },
                        Message::Terminate => {
                            IS_CLOSING.store(true, Ordering::Release);
                            return;
                        }
                    }
                } else if let Some(g) = grave.as_ref() {
                    if idle_counter < 10 {
                        // addition of the idle counts, quit after being idle for around 1 sec.
                        idle_counter += 1;
                    } else {
                        // if an expandable worker, kill it.
                        g.lock().insert(id);
                        return;
                    }
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
