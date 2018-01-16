extern crate thread_utils;

pub mod router;
pub mod connection;

use std::net::TcpListener;
use connection::*;
use router::*;
use thread_utils::ThreadPool;

pub struct HttpServerDefinition {
    pub port: String,
    pub pool_size: usize,
    pub route: Route,
}

pub trait BaseServer {
    fn start_with(&self);
}

impl BaseServer for HttpServerDefinition {
    fn start_with(&self) {
        let port =
            if self.port.is_empty() {
                String::from("8080")
            } else {
                String::from(&self.port[..])
            };

        let listener =
            TcpListener::bind(format!("127.0.0.1:{}", port)).unwrap();

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    println!("Connected...\nListening for connections on port {}", self.port);

                    let pool = ThreadPool::new(self.pool_size);
                    pool.execute(|| {
                        handle_connection(stream);
                    });
                },
                Err(e) => {
                    println!("Unable to start the server: {}", e);
                }
            }
        }
    }
}
