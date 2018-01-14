extern crate thread_utils;

pub mod router;

use std::net::TcpListener;
use router::*;
use thread_utils::ThreadPool;

pub struct HttpServerDefinition {
    pub threads: usize,
    pub route: Route,
}

pub trait BaseServer {
    fn start_with(&self);
}

impl BaseServer for HttpServerDefinition {
    fn start_with(&self) {
        let listener = TcpListener::bind("127.0.0.1:8080").unwrap();

        for stream in listener.incoming() {
            let stream = stream.unwrap();
            println!("Connected...");
        }
    }
}
