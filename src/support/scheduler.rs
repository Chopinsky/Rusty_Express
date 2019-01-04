#![allow(dead_code)]

use std::mem;
use std::sync::{mpsc, Arc, Mutex, Once, ONCE_INIT};
use std::thread;
use std::time::Duration;
use crate::support::debug::{self, InfoLevel};

static TIMEOUT: Duration = Duration::from_millis(200);

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

struct Inbox {
    receiver: mpsc::Receiver<Message>,
    is_closing: bool,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Message>,
}

impl ThreadPool {
    pub(crate) fn new(size: usize) -> ThreadPool {
        let pool_size = match size {
            _ if size < 1 => 1,
            _ => size,
        };

        let (sender, receiver) = mpsc::channel();

        // TODO: when switching to mpmc, try get rid of the Arc-Mutex structure
        let inbox = Arc::new(Mutex::new(Inbox {
            receiver,
            is_closing: false
        }));

        let mut workers = Vec::with_capacity(pool_size);
        for id in 0..pool_size {
            workers.push(Worker::new(id, Arc::clone(&inbox)));
        }

        ThreadPool {
            workers,
            sender,
        }
    }

    pub(crate) fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);

        //TODO: switching to mpmc, and check sender.len() to determine if we need to add more workers

        if let Err(err) = self.sender.send(Message::NewJob(job)) {
            print!("Failed: {}", err);
            debug::print(&format!("Unable to distribute the job: {}", err), InfoLevel::Error);
        };
    }

    pub(crate) fn clear(&mut self) {
        self.sender.send(Message::Terminate).unwrap_or_else(|err| {
            debug::print(&format!("Unable to send message: {}", err), InfoLevel::Error);
            return;
        });

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread
                    .join()
                    .expect("Couldn't join on the associated thread");
            }
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        debug::print("Job done, sending terminate message to all workers.", InfoLevel::Error);
        self.clear();
    }
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, inbox: Arc<Mutex<Inbox>>) -> Worker {
        let thread = thread::spawn(move || {
            let mut assignment = None;

            loop {
                if let Ok(mut locked_box) = inbox.lock() {
                    if locked_box.is_closing {
                        return;
                    }

                    if let Ok(message) = locked_box.receiver.recv() {
                        match message {
                            Message::NewJob(job) => assignment = Some(job),
                            Message::Terminate => {
                                locked_box.is_closing = true;
                                return
                            },
                        }
                    }
                }

                if let Some(job) = assignment.take() {
                    job.call_box()
                }
            }
        });

        Worker {
            id,
            thread: Some(thread),
        }
    }
}

struct Pool {
    req_workers: Box<ThreadPool>,
    resp_workers: Box<ThreadPool>,
}

pub enum TaskType {
    Request,
    Response,
}

static ONCE: Once = ONCE_INIT;
static mut POOL: Option<Pool> = None;

pub(crate) fn initialize_with(sizes: Vec<usize>) {
    unsafe {
        ONCE.call_once(|| {
            let pool_sizes: Vec<usize> = sizes
                .iter()
                .map(|val| match val {
                    &0 => 1,
                    _ => *val,
                })
                .collect();

            let (req_size, resp_size) = match pool_sizes.len() {
                1 => (pool_sizes[0], pool_sizes[0]),
                2 => (pool_sizes[0], pool_sizes[1]),
                _ => panic!("Requiring vec sizes of 2 for each, or 1 for all"),
            };

            // Make the pool
            let pool = Some(Pool {
                req_workers: Box::new(ThreadPool::new(req_size)),
                resp_workers: Box::new(ThreadPool::new(2 * resp_size)),
            });

            // Put it in the heap so it can outlive this call
            POOL = mem::transmute(pool);
        });
    }
}

pub(crate) fn run<F>(f: F, task: TaskType)
where
    F: FnOnce() + Send + 'static,
{
    unsafe {
        if let Some(ref mut pool) = POOL {
            // if pool has been created
            match task {
                TaskType::Request => {
                    pool.req_workers.execute(f);
                    return;
                }
                TaskType::Response => {
                    pool.resp_workers.execute(f);
                    return;
                }
            };
        }

        // otherwise, spawn to a new thread for the work;
        thread::spawn(f);
    }
}

pub(crate) fn close() {
    unsafe {
        if let Some(mut pool) = POOL.take() {
            pool.req_workers.clear();
            pool.resp_workers.clear();
        }
    }
}
