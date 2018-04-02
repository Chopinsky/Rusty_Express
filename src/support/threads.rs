#![allow(dead_code)]

use std::mem;
use std::thread;
use std::sync::{Arc, mpsc, Mutex, Once, ONCE_INIT};
use support::debug;

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
        self.sender.send(Message::NewJob(job)).unwrap_or_else(|err| {
            debug::print(&format!("Unable to distribute the job: {}", err), 3);
        });
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
            Worker::launch(receiver);
        });

        Worker {
            id,
            thread: Some(thread),
        }
    }

    fn launch(receiver: Arc<Mutex<mpsc::Receiver<Message>>>) {
        let mut next_message: Option<Message> = None;

        loop {
            if let Ok(rx) = receiver.lock() {
                if let Ok(message) = rx.recv() {
                    next_message = Some(message);
                }
            }

            if let Some(msg) = next_message.take() {
                match msg {
                    Message::NewJob(job) => job.call_box(),
                    Message::Terminate => break,
                }
            }
        }
    }
}

struct Pool {
    req_workers: Box<ThreadPool>,
    resp_workers: Box<ThreadPool>,
    body_workers: Box<ThreadPool>,
}

static ONCE: Once = ONCE_INIT;
static mut POOL: Option<Pool> = None;

pub enum TaskType {
    Request,
    Response,
    Body,
}

pub fn initialize_with(sizes: Vec<usize>) {
    unsafe {
        ONCE.call_once(|| {
            let pool_sizes: Vec<usize> = sizes.iter().map(|val| {
                match val {
                    &0 => 1,
                    _ => *val,
                }
            }).collect();

            let (req_size, resp_size, body_size) = match pool_sizes.len() {
                1 => (pool_sizes[0], pool_sizes[0], pool_sizes[0]),
                3 => (pool_sizes[0], pool_sizes[1], pool_sizes[2]),
                _ => panic!("Requiring vec sizes of 3 for each, or 1 for all"),
            };

            // Make the pool
            let pool = Some(Pool {
                req_workers: Box::new(ThreadPool::new(req_size)),
                resp_workers: Box::new(ThreadPool::new(resp_size)),
                body_workers: Box::new(ThreadPool::new(body_size)),
            });

            // Put it in the heap so it can outlive this call
            POOL = mem::transmute(pool);
        });
    }
}

pub fn run<F>(f: F, task: TaskType)
    where F: FnOnce() + Send + 'static {

    unsafe {
        if let Some(ref pool) = POOL {
            // if pool has been created
            match task {
                TaskType::Request => pool.req_workers.execute(f),
                TaskType::Response => pool.resp_workers.execute(f),
                TaskType::Body => pool.body_workers.execute(f),
            };
        } else {
            // otherwise, spawn to a new thread for the work;
            thread::spawn(f);
        }
    }
}

pub fn close() {
    unsafe {
        if let Some(mut pool) = POOL.take() {
            pool.req_workers.clear();
            pool.resp_workers.clear();
            pool.body_workers.clear();

            drop(pool);
        }
    }
}