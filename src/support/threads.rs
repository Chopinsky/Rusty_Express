#![allow(dead_code)]

use std::cmp;
use std::{mem, thread};
use std::sync::{Arc, mpsc, Mutex, Once, ONCE_INIT};

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
    pub fn new(mut size: usize) -> ThreadPool {
        if size < 1 { size = 1; }

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            workers.push(Worker::new(id, Arc::clone(&receiver)));
        }

        ThreadPool {
            workers,
            sender,
        }
    }

    pub fn execute<F>(&self, f: F)
        where F: FnOnce() + Send + 'static
    {
        let job = Box::new(f);
        self.sender.send(Message::NewJob(job)).unwrap();
    }
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let thread = thread::spawn(move || {
            loop {
                let message = receiver.lock().unwrap().recv().unwrap();

                match message {
                    Message::NewJob(job) => {
                        job.call_box();
                    },
                    Message::Terminate => {
                        break;
                    },
                }
            }
        });

        Worker {
            id,
            thread: Some(thread),
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        println!("Job done, sending terminate message to all workers.");

        for _ in &mut self.workers {
            self.sender.send(Message::Terminate).unwrap();
        }

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        }
    }
}

static MIN_POOL_SIZE: usize = 12;
static ONCE: Once = ONCE_INIT;
static mut POOL: Pool = Pool { store: None };

struct Pool {
    store: Option<Box<ThreadPool>>,
}

fn initialize_with(size: usize) {
    let count = cmp::max(MIN_POOL_SIZE, size);

    unsafe {
        ONCE.call_once(|| {
            // Make it
            let pool = Pool { store: Some(Box::new(ThreadPool::new(count))), };

            // Put it in the heap so it can outlive this call
            POOL = mem::transmute(pool);
        });
    }
}

pub fn initialize() {
    initialize_with(MIN_POOL_SIZE);
}

pub fn run<F>(f: F)
    where F: FnOnce() + Send + 'static {

    unsafe {
        if let Some(ref store) = POOL.store {
            // if pool is created
            store.execute(f);
        } else {
            // otherwise, spawn to a new thread for the work;
            thread::spawn(f);
            initialize();
        }
    }
}