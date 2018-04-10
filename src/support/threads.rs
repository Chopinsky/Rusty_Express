#![allow(dead_code)]

use std::mem;
use std::thread;
use std::time::Duration;
use std::sync::{mpsc, Arc, Mutex, Once, ONCE_INIT};
use support::debug;

static TIMEOUT: Duration = Duration::from_millis(200);
type Job = Box<FnBox + Send + 'static>;

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

enum Message {
    NewJob(Job),
    Terminate,
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Message>,
}

impl ThreadPool {
    pub fn new(size: usize) -> ThreadPool {
        let pool_size = match size {
            _ if size < 1 => 1,
            _ => size,
        };

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(pool_size);
        for id in 0..pool_size {
            workers.push(Worker::new(id, Arc::clone(&receiver)));
        }

        ThreadPool {
            workers,
            sender,
        }
    }

    pub fn execute<F>(&self, f: F) where F: FnOnce() + Send + 'static {
        let job = Box::new(f);

        if let Err(err) = self.sender.send(Message::NewJob(job)) {
            print!("Failed: {}", err);
            debug::print(&format!("Unable to distribute the job: {}", err), 3);
        };
    }

    pub fn clear(&mut self) {
        for _ in &mut self.workers {
            self.sender.send(Message::Terminate).unwrap_or_else(|err| {
                debug::print(&format!("Unable to send message: {}", err), 3);
            });
        }

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread.join().expect("Couldn't join on the associated thread");
            }
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        debug::print("Job done, sending terminate message to all workers.", 3);
        self.clear();
    }
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let thread = thread::spawn(move || {
            let mut new_assignment = None;

            loop {
                if let Ok(rx) = receiver.lock() {
                    if let Ok(message) = rx.recv() {
                        new_assignment = Some(message);
                    }
                }

                if let Some(message) = new_assignment.take() {
                    match message {
                        Message::NewJob(job) => job.call_box(),
                        Message::Terminate => break,
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

struct Pool {
    req_workers: Mutex<Box<ThreadPool>>,
    resp_workers: Mutex<Box<ThreadPool>>,
}

pub enum TaskType {
    Request,
    Response,
}

static ONCE: Once = ONCE_INIT;
static mut POOL: Option<Pool> = None;

pub fn initialize_with(sizes: Vec<usize>) {
    unsafe {
        ONCE.call_once(|| {
            let pool_sizes: Vec<usize> = sizes.iter().map(|val| {
                match val {
                    &0 => 1,
                    _ => *val,
                }
            }).collect();

            let (req_size, resp_size) = match pool_sizes.len() {
                1 => (pool_sizes[0], pool_sizes[0]),
                2 => (pool_sizes[0], pool_sizes[1]),
                _ => panic!("Requiring vec sizes of 2 for each, or 1 for all"),
            };

            // Make the pool
            let pool = Some(Pool {
                req_workers: Mutex::new(Box::new(ThreadPool::new(req_size))),
                resp_workers: Mutex::new(Box::new(ThreadPool::new(2 * resp_size))),
            });

            // Put it in the heap so it can outlive this call
            POOL = mem::transmute(pool);
        });
    }
}

pub fn run<F>(f: F, task: TaskType)
    where F: FnOnce() + Send + 'static {

    //TODO: update thread_pool

    unsafe {
        if let Some(ref pool) = POOL {
            // if pool has been created
            match task {
                TaskType::Request => {
                    if let Ok(workers) = pool.req_workers.lock() {
                        workers.execute(f);
                        return;
                    }
                },
                TaskType::Response => {
                    if let Ok(workers) = pool.resp_workers.lock() {
                        workers.execute(f);
                        return;
                    }
                },
            };
        }

        // otherwise, spawn to a new thread for the work;
        thread::spawn(f);
    }
}

pub fn close() {
    unsafe {
        if let Some(pool) = POOL.take() {
            if let Ok(mut workers) = pool.req_workers.lock() {
                workers.clear();
            }

            if let Ok(mut workers) = pool.resp_workers.lock() {
                workers.clear();
            }
        }
    }
}