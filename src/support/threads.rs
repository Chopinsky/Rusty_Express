#![allow(dead_code)]

use std::mem;
use std::thread;
use std::sync::{Arc, mpsc, Mutex, Once, ONCE_INIT};
use num_cpus;

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
            Worker::launch(receiver);
        });

        Worker {
            id,
            thread: Some(thread),
        }
    }

    fn launch(receiver: Arc<Mutex<mpsc::Receiver<Message>>>) {
        loop {
            if let Ok(rx) = receiver.lock() {
                if let Ok(message) = rx.recv() {
                    if let Message::NewJob(job) = message {
                        job.call_box();
                    } else {
                        break;
                    }
                }
            }
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

pub fn initialize_with(size: usize) {
    unsafe {
        ONCE.call_once(|| {
            // Make it
            let pool = Some(Pool {
                req_workers: Box::new(ThreadPool::new(size)),
                resp_workers: Box::new(ThreadPool::new(2 * size)),
                body_workers: Box::new(ThreadPool::new(size)),
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