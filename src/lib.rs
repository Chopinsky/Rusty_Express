extern crate thread_utils;

pub mod connection;
pub mod http;
pub mod router;

use std::net::{SocketAddr, TcpListener};
use connection::*;
use router::*;
use thread_utils::ThreadPool;

pub struct HttpServer {
    pub pool_size: usize,
    pub route: Route,
}

impl HttpServer {
    pub fn new(pool_size: usize) -> Self {
        return HttpServer {
            pool_size,
            route: Route::new(),
        }
    }

    pub fn listen(&self, port: u16) {
        start_with(&self, port);
    }
}

//TODO: impl trait for Router

fn start_with(server: &HttpServer, port: u16) {
    let listener: TcpListener;
    let server_address = SocketAddr::from(([127, 0, 0, 1], port));

    match TcpListener::bind(server_address) {
        Ok(result) => {
            println!("Listening for connections on port {}", port);
            listener = result;
        },
        Err(e) => {
            println!("Unable to start the http server: {}", e);
            return;
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let pool = ThreadPool::new(server.pool_size);
                pool.execute(|| {
                    handle_connection(s);
                });
            },
            Err(e) => {
                panic!("Server is unable to read from the upcoming stream: {}", e);
            }
        }
    }
}
