#![allow(dead_code)]

extern crate thread_utils;
extern crate regex;

pub mod connection;
pub mod http;
pub mod router;

use std::net::{SocketAddr, TcpListener};
use connection::*;
use router::*;
use thread_utils::ThreadPool;

pub struct HttpServer {
    pub pool_size: usize,
    router: Route,
}

impl HttpServer {
    pub fn new(pool_size: usize) -> Self {
        HttpServer {
            pool_size,
            router: Route::new(),
        }
    }

    pub fn use_router(&mut self, router: Route) {
        self.router = router;
    }

    pub fn listen(&self, port: u16) {
        start_with(self, port);
    }
}

impl Router for HttpServer {
    fn get(&mut self, uri: RequestPath, callback: Callback) {
        self.router.get(uri, callback);
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) {
        self.router.get(uri, callback);
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) {
        self.router.get(uri, callback);
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) {
        self.router.get(uri, callback);
    }
}

fn start_with(server: &HttpServer, port: u16) {
    let server_address = SocketAddr::from(([127, 0, 0, 1], port));

    if let Ok(listener) = TcpListener::bind(server_address) {
        println!("Listening for connections on port {}", port);

        let pool = ThreadPool::new(server.pool_size);

        for stream in listener.incoming() {
            if let Ok(s) = stream {
                // clone the router so it can out live the closure.
                let router = Route::from(&server.router);
                pool.execute(move || {
                    handle_connection(s, &router);
                });
            }
        }

    } else {
        println!("Unable to start the http server...");
        return;
    }
}


