extern crate thread_utils;

pub mod connection;
pub mod http;
pub mod router;

use std::net::TcpListener;
use connection::*;
use router::*;
use thread_utils::ThreadPool;

pub struct HttpServer {
    pub pool_size: usize,
    pub route: Route,
}

impl HttpServer {
    pub fn new(pool_size: usize) -> HttpServer {
        return HttpServer {
            pool_size,
            route: Route::new(),
        }
    }

    pub fn listen(&self, port: String) {
        start_with(&self, port);
    }
}

//TODO: impl trait for Router

fn start_with(server: &HttpServer, port: String) {
    let server_port =
        if port.is_empty() {
            String::from("8080")
        } else {
            String::from(&port[..])
        };

    let listener: TcpListener;
    match TcpListener::bind(format!("127.0.0.1:{}", server_port)) {
        Ok(result) => {
            println!("Listening for connections on port {}", server_port);
            listener = result;
        },
        Err(e) => {
            println!("Unable to start the http server: {}", e);
            return;
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let pool = ThreadPool::new(server.pool_size);
                pool.execute(|| {
                    handle_connection(stream);
                });
            },
            Err(e) => {
                println!("Server is unable to read from the upcoming stream: {}", e);
            }
        }
    }
}
